//! KAG scenario tokenization and restorable cursors. Script evaluation is delegated to TJS.
use anyhow::{Context, Result, bail, ensure};
use krkrz_core::SourceLocation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub name: String,
    /// Attribute order is significant in KAGParserEx.
    pub attributes: Vec<(String, String)>,
}
impl Tag {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind {
    Text(String),
    Tag(Tag),
    Label {
        name: String,
        caption: Option<String>,
    },
    Newline,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub location: SourceLocation,
    pub kind: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scenario {
    pub storage: String,
    pub tokens: Vec<Token>,
    pub labels: BTreeMap<String, usize>,
}
fn tag(input: &str) -> Result<Tag> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
            *i += 1;
        }
    };
    skip(&mut i);
    let start = i;
    while chars.get(i).is_some_and(|c| !c.is_whitespace()) {
        i += 1;
    }
    ensure!(i > start, "empty KAG tag");
    let name = chars[start..i]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    let mut attributes = vec![];
    while i < chars.len() {
        skip(&mut i);
        if i == chars.len() {
            break;
        }
        let start = i;
        while chars
            .get(i)
            .is_some_and(|c| !c.is_whitespace() && *c != '=')
        {
            i += 1;
        }
        ensure!(i > start, "missing attribute name");
        let key = chars[start..i]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        skip(&mut i);
        let value = if chars.get(i) == Some(&'=') {
            i += 1;
            skip(&mut i);
            ensure!(i < chars.len(), "missing value for KAG attribute {key}");
            let quote = if matches!(chars[i], '\'' | '"') {
                let c = chars[i];
                i += 1;
                Some(c)
            } else {
                None
            };
            let mut value = String::new();
            let mut closed = quote.is_none();
            while let Some(&c) = chars.get(i) {
                if Some(c) == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                if quote.is_none() && c.is_whitespace() {
                    break;
                }
                if c == '`' {
                    i += 1;
                    value.push(*chars.get(i).context("truncated KAG escape")?);
                    i += 1;
                } else {
                    value.push(c);
                    i += 1;
                }
            }
            ensure!(closed, "unterminated KAG attribute quote");
            ensure!(
                chars.get(i).is_none_or(|c| c.is_whitespace()),
                "missing whitespace after KAG attribute"
            );
            value
        } else {
            "true".into()
        };
        attributes.push((key, value));
    }
    Ok(Tag { name, attributes })
}
impl Scenario {
    pub fn parse(storage: &str, source: &str) -> Result<Self> {
        let lines = source.lines().collect::<Vec<_>>();
        let mut tokens = vec![];
        let mut labels = BTreeMap::new();
        let mut line = 0;
        while line < lines.len() {
            let original = line;
            let mut text = lines[line].trim_end_matches('\r').to_owned();
            line += 1;
            let at = SourceLocation {
                storage: storage.into(),
                line: original + 1,
                column: 1,
            };
            if text.starts_with(';') || text.is_empty() {
                continue;
            }
            if let Some(label) = text.strip_prefix('*') {
                let (name, caption) = label
                    .split_once('|')
                    .map_or((label, None), |(n, c)| (n, Some(c.to_owned())));
                ensure!(
                    !name.is_empty() && labels.insert(name.into(), tokens.len()).is_none(),
                    "duplicate or empty KAG label: {name}"
                );
                tokens.push(Token {
                    location: at,
                    kind: Kind::Label {
                        name: name.into(),
                        caption,
                    },
                });
                continue;
            }
            let mut multiline = false;
            while text.ends_with(" \\") {
                multiline = true;
                text.truncate(text.len() - 1);
                let continuation = lines
                    .get(line)
                    .context("missing multiline KAG continuation")?
                    .strip_prefix(';')
                    .context("KAG continuation must start with ';'")?;
                text.push_str(continuation);
                line += 1;
            }
            if let Some(command) = text.strip_prefix('@') {
                tokens.push(Token {
                    location: at.clone(),
                    kind: Kind::Tag(
                        tag(command).with_context(|| format!("{storage}:{}", at.line))?,
                    ),
                });
                continue;
            }
            let chars = text.chars().collect::<Vec<_>>();
            let mut i = 0;
            let mut literal = String::new();
            let mut column = 1;
            while i < chars.len() {
                let c = chars[i];
                if c == '[' && chars.get(i + 1) == Some(&'[') {
                    literal.push('[');
                    i += 2;
                    column += 2;
                    continue;
                }
                if c != '[' {
                    literal.push(c);
                    i += 1;
                    column += c.len_utf16();
                    continue;
                }
                if !literal.is_empty() {
                    tokens.push(Token {
                        location: at.clone(),
                        kind: Kind::Text(std::mem::take(&mut literal)),
                    });
                }
                let start = i + 1;
                let tag_column = column;
                i += 1;
                column += 1;
                let mut quote = None;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '`' {
                        ensure!(i + 1 < chars.len(), "truncated KAG tag escape");
                        i += 2;
                        column += 2;
                        continue;
                    }
                    if quote == Some(c) {
                        quote = None;
                    } else if quote.is_none() && matches!(c, '\'' | '"') {
                        quote = Some(c);
                    } else if quote.is_none() && c == ']' {
                        break;
                    }
                    i += 1;
                    column += c.len_utf16();
                }
                ensure!(
                    i < chars.len(),
                    "{storage}:{}:{tag_column}: unterminated KAG tag",
                    at.line
                );
                tokens.push(Token {
                    location: SourceLocation {
                        column: tag_column,
                        ..at.clone()
                    },
                    kind: Kind::Tag(tag(&chars[start..i].iter().collect::<String>())?),
                });
                i += 1;
                column += 1;
                if multiline {
                    break;
                }
            }
            if !literal.is_empty() {
                tokens.push(Token {
                    location: at.clone(),
                    kind: Kind::Text(literal),
                });
            }
            if !multiline {
                tokens.push(Token {
                    location: at,
                    kind: Kind::Newline,
                });
            }
            ensure!(tokens.len() <= 1_000_000, "KAG token limit exceeded");
        }
        Ok(Self {
            storage: storage.into(),
            tokens,
            labels,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cursor {
    pub position: usize,
    pub calls: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct Parser {
    scenario: Scenario,
    cursor: Cursor,
}
impl Parser {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            cursor: Cursor {
                position: 0,
                calls: vec![],
            },
        }
    }
    pub fn state(&self) -> Cursor {
        self.cursor.clone()
    }
    pub fn restore(&mut self, state: Cursor) -> Result<()> {
        ensure!(
            state.position <= self.scenario.tokens.len()
                && state.calls.len() <= 128
                && state.calls.iter().all(|p| *p <= self.scenario.tokens.len()),
            "invalid KAG cursor state"
        );
        self.cursor = state;
        Ok(())
    }
    pub fn next_token(&mut self) -> Option<&Token> {
        let token = self.scenario.tokens.get(self.cursor.position)?;
        self.cursor.position += 1;
        Some(token)
    }
    pub fn jump(&mut self, label: &str) -> Result<()> {
        self.cursor.position = *self
            .scenario
            .labels
            .get(label.trim_start_matches('*'))
            .with_context(|| format!("KAG label not found: {label}"))?;
        Ok(())
    }
    pub fn call(&mut self, label: &str) -> Result<()> {
        ensure!(self.cursor.calls.len() < 128, "KAG call depth exceeded");
        let pos = self.cursor.position;
        self.jump(label)?;
        self.cursor.calls.push(pos);
        Ok(())
    }
    pub fn return_from_call(&mut self) -> Result<()> {
        self.cursor.position = self.cursor.calls.pop().context("KAG return without call")?;
        Ok(())
    }
    pub fn evaluate(
        &self,
        expression: &str,
        vm: &mut krkrz_tjs::Vm,
        host: &mut impl krkrz_tjs::Host,
        budget: &mut u64,
    ) -> Result<krkrz_tjs::Value> {
        let program = krkrz_tjs::compile_expression(&self.scenario.storage, expression)?;
        vm.execute(&program, host, budget)
    }
    /// Fail explicitly if asked to execute control tags before framework dispatch exists.
    pub fn require_plain_tag(tag: &Tag) -> Result<()> {
        if [
            "macro",
            "endmacro",
            "if",
            "elsif",
            "else",
            "endif",
            "ignore",
            "endignore",
            "iscript",
            "emb",
            "pmacro",
        ]
        .contains(&tag.name.as_str())
        {
            bail!("KAG control execution is not implemented: {}", tag.name);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_brackets_order_and_multiline() {
        let s = Scenario::parse(
            "test.ks",
            ";comment\n*start|Title\n[[hello[tag a=\"x] y\" b=2]\n@line a=1 \\\n; b=2",
        )
        .unwrap();
        assert_eq!(s.labels["start"], 0);
        assert_eq!(s.tokens[1].kind, Kind::Text("[hello".into()));
        let Kind::Tag(t) = &s.tokens[2].kind else {
            panic!()
        };
        assert_eq!(
            t.attributes,
            [("a".into(), "x] y".into()), ("b".into(), "2".into())]
        );
        assert!(matches!(&s.tokens[4].kind,Kind::Tag(t) if t.attributes.len()==2));
    }
    #[test]
    fn save_call_restore() {
        let s = Scenario::parse("t", "*a\nhello\n*b\nbye").unwrap();
        let mut p = Parser::new(s);
        p.next_token();
        let saved = p.state();
        p.call("b").unwrap();
        p.return_from_call().unwrap();
        assert_eq!(p.state(), saved);
        p.jump("b").unwrap();
        p.restore(saved).unwrap();
        assert_eq!(p.next_token().unwrap().kind, Kind::Text("hello".into()));
        assert!(
            p.restore(Cursor {
                position: usize::MAX,
                calls: vec![]
            })
            .is_err()
        );
    }
}
