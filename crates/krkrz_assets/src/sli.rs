//! Kirikiri sample-frame loop metadata. See upstream sound/WaveLoopManager.cpp.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{cmp::Reverse, collections::BTreeMap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Condition {
    #[default]
    None,
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: u64,
    pub to: u64,
    pub smooth: bool,
    pub condition: Condition,
    pub reference: i32,
    /// -1 bypasses the condition; otherwise this is one of 16 sound flags.
    pub variable: i32,
}
impl Link {
    pub fn matches(&self, flags: &[i32; 16]) -> bool {
        if self.variable == -1 {
            return true;
        }
        let Some(&flag) = flags.get(self.variable as usize) else {
            return false;
        };
        match self.condition {
            Condition::None => true,
            Condition::Equal => flag == self.reference,
            Condition::NotEqual => flag != self.reference,
            Condition::Greater => flag > self.reference,
            Condition::GreaterOrEqual => flag >= self.reference,
            Condition::Less => flag < self.reference,
            Condition::LessOrEqual => flag <= self.reference,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub position: u64,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInfo {
    pub links: Vec<Link>,
    pub labels: Vec<Label>,
}
impl LoopInfo {
    pub fn parse(source: &str) -> Result<Self> {
        ensure!(source.len() <= 4 << 20, "SLI text exceeds limit");
        ensure!(!source.contains('\0'), "NUL in SLI text");
        let mut info = Self::default();
        if !source.starts_with('#') {
            let field = |key| -> Result<u64> {
                let tail = source
                    .split_once(key)
                    .context("missing legacy SLI field")?
                    .1;
                let value: String = tail
                    .trim_start()
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                value.parse().context("invalid legacy SLI integer")
            };
            let to = field("LoopStart=")?;
            let length = field("LoopLength=")?;
            let from = to
                .checked_add(length)
                .context("SLI loop endpoint overflow")?;
            info.links.push(Link {
                from,
                to,
                ..Link::default()
            });
        } else {
            ensure!(
                source.lines().next().unwrap_or("").trim_end() == "#2.00",
                "unsupported SLI version"
            );
            let mut parser = Parser { source, pos: 0 };
            while parser.skip() {
                ensure!(
                    info.links.len() + info.labels.len() < 65_536,
                    "SLI event limit exceeded"
                );
                let kind = parser.word()?.to_ascii_lowercase();
                parser.expect('{')?;
                let mut fields = BTreeMap::new();
                loop {
                    parser.skip();
                    if parser.eat('}') {
                        break;
                    }
                    let key = parser.word()?.to_ascii_lowercase();
                    parser.expect('=')?;
                    parser.skip();
                    let quoted = parser.eat('\'');
                    let start = parser.pos;
                    while let Some(c) = parser.peek() {
                        if (quoted && c == '\'')
                            || (!quoted && (c.is_ascii_whitespace() || c == ';'))
                        {
                            break;
                        }
                        parser.pos += c.len_utf8();
                    }
                    let value = parser.source[start..parser.pos].to_owned();
                    if quoted {
                        ensure!(parser.eat('\''), "unterminated SLI value");
                    }
                    parser.expect(';')?;
                    ensure!(fields.insert(key, value).is_none(), "duplicate SLI field");
                }
                match kind.as_str() {
                    "link" => {
                        let mut link = Link::default();
                        for (key, value) in fields {
                            match key.as_str() {
                                "from" => link.from = value.parse().context("invalid SLI From")?,
                                "to" => link.to = value.parse().context("invalid SLI To")?,
                                "smooth" => {
                                    link.smooth = match value.to_ascii_lowercase().as_str() {
                                        "true" | "yes" => true,
                                        "false" | "no" => false,
                                        _ => bail!("invalid SLI Smooth"),
                                    }
                                }
                                "condition" => {
                                    link.condition = match value.to_ascii_lowercase().as_str() {
                                        "no" => Condition::None,
                                        "eq" => Condition::Equal,
                                        "ne" => Condition::NotEqual,
                                        "gt" => Condition::Greater,
                                        "ge" => Condition::GreaterOrEqual,
                                        "lt" => Condition::Less,
                                        "le" => Condition::LessOrEqual,
                                        _ => bail!("invalid SLI Condition"),
                                    }
                                }
                                "refvalue" => {
                                    link.reference =
                                        value.parse().context("invalid SLI RefValue")?
                                }
                                "condvar" => {
                                    link.variable = value.parse().context("invalid SLI CondVar")?
                                }
                                _ => bail!("unsupported SLI link field {key}"),
                            }
                        }
                        ensure!(
                            (-1..16).contains(&link.variable),
                            "SLI condition flag index out of range"
                        );
                        info.links.push(link);
                    }
                    "label" => {
                        let mut label = Label::default();
                        for (key, value) in fields {
                            match key.as_str() {
                                "position" => {
                                    label.position =
                                        value.parse().context("invalid SLI Position")?
                                }
                                "name" => label.name = value,
                                _ => bail!("unsupported SLI label field {key}"),
                            }
                        }
                        info.labels.push(label);
                    }
                    _ => bail!("unsupported SLI entity {kind}"),
                }
            }
        }
        // The label dictionary uses file order; the decoder sorts labels lazily.
        info.links
            .sort_by_key(|l| (l.from, Reverse(l.condition), Reverse(l.variable)));
        Ok(info)
    }
    /// Conditional links take priority over unconditional links at the same frame.
    pub fn sort(&mut self) {
        self.links
            .sort_by_key(|l| (l.from, Reverse(l.condition), Reverse(l.variable)));
        self.labels.sort_by_key(|l| l.position);
    }
    pub fn validate_frames(&self, frames: u64) -> Result<()> {
        ensure!(
            self.links.len() + self.labels.len() <= 65_536,
            "SLI event limit exceeded"
        );
        for link in &self.links {
            ensure!(
                link.from <= frames && link.to <= frames,
                "SLI link is outside audio frames"
            );
            ensure!(
                (-1..16).contains(&link.variable),
                "SLI condition flag index out of range"
            );
        }
        ensure!(
            self.labels.iter().all(|l| l.position <= frames),
            "SLI label is outside audio frames"
        );
        Ok(())
    }
    /// Call sort after changing links. Conditions compare the flag against RefValue.
    pub fn next_link(&self, position: u64, flags: Option<&[i32; 16]>) -> Option<&Link> {
        self.links[self.links.partition_point(|l| l.from < position)..]
            .iter()
            .find(|l| flags.is_none_or(|f| l.matches(f)))
    }
}
struct Parser<'a> {
    source: &'a str,
    pos: usize,
}
impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn skip(&mut self) -> bool {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.pos += 1;
            }
            if self.peek() != Some('#') {
                return self.pos < self.source.len();
            }
            while self.peek().is_some_and(|c| c != '\n') {
                self.pos += self.peek().unwrap().len_utf8();
            }
        }
    }
    fn word(&mut self) -> Result<&'a str> {
        self.skip();
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        ensure!(
            self.pos != start,
            "expected SLI identifier at byte {}",
            self.pos
        );
        Ok(&self.source[start..self.pos])
    }
    fn expect(&mut self, c: char) -> Result<()> {
        self.skip();
        ensure!(self.eat(c), "expected SLI {c:?} at byte {}", self.pos);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_conditional_priority() {
        let old = LoopInfo::parse("LoopStart=2\nLoopLength=4\n").unwrap();
        assert_eq!((old.links[0].from, old.links[0].to), (6, 2));
        let info = LoopInfo::parse("#2.00\nLink { From=6; To=2; }\nLink { From=6; To=3; Condition=gt; RefValue=1; CondVar=0; }\nLabel { Position=3; Name='日本語; label'; }").unwrap();
        let mut flags = [0; 16];
        assert_eq!(info.next_link(0, Some(&flags)).unwrap().to, 2);
        flags[0] = 2;
        assert_eq!(info.next_link(0, Some(&flags)).unwrap().to, 3);
        assert_eq!(info.labels[0].name, "日本語; label");
        assert!(info.validate_frames(5).is_err());
    }
    #[test]
    fn malformed_metadata() {
        for source in [
            "#3.00",
            "#2.00\nLink { From=-1; }",
            "#2.00\nLink { CondVar=16; }",
            "#2.00\nLabel { Name='unterminated; }",
            "LoopStart=18446744073709551615\nLoopLength=1",
            "#2.00\nLink { From=3; From=4; }",
            "#2.00\nLink { Unknown=0; }",
        ] {
            assert!(LoopInfo::parse(source).is_err(), "{source}");
        }
    }
}
