//! UTF-16 scenario storage and incremental KAGParserEx lexical state.
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Source {
    pub lines: Vec<Vec<u16>>,
    pub aliases: Vec<String>,
    pub labels: BTreeMap<String, usize>,
}
impl Source {
    pub fn parse(text: &[u16]) -> Result<Self> {
        ensure!(
            text.len() <= 8 * 1024 * 1024,
            "KAG scenario exceeds text limit"
        );
        let text = &text[..text.iter().position(|c| *c == 0).unwrap_or(text.len())];
        ensure!(!text.is_empty(), "KAG scenario has no lines");
        let mut lines = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < text.len() {
            if matches!(text[i], 10 | 13) {
                lines.push(trim_tabs(&text[start..i]));
                if text[i] == 13 && text.get(i + 1) == Some(&10) {
                    i += 1;
                }
                start = i + 1;
            }
            i += 1;
        }
        if start < text.len() {
            let last = trim_tabs(&text[start..]);
            if !last.is_empty() {
                lines.push(last);
            }
        }
        ensure!(lines.len() <= 1_000_000, "KAG line count exceeds limit");
        Ok(Self {
            aliases: vec![String::new(); lines.len()],
            lines,
            labels: BTreeMap::new(),
        })
    }
    pub fn index_labels(&mut self) -> Result<()> {
        self.labels.clear();
        self.aliases.fill(String::new());
        let mut previous = String::new();
        let mut counts = BTreeMap::<String, u32>::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.len() < 2 || line[0] != 42 {
                continue;
            }
            let end = line.iter().position(|c| *c == 124).unwrap_or(line.len());
            let mut name = String::from_utf16_lossy(&line[..end]);
            if name == "*" {
                ensure!(!previous.is_empty(), "cannot omit first KAG label name");
                name = previous.clone();
            }
            previous = name.clone();
            let count = counts.entry(name.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                name = format!("{name}:{count}");
            }
            self.labels.insert(name.clone(), i);
            self.aliases[i] = name;
        }
        Ok(())
    }
}
fn trim_tabs(line: &[u16]) -> Vec<u16> {
    line.iter().skip_while(|c| **c == 9).copied().collect()
}
#[derive(Clone, Debug, Default)]
pub struct Position {
    pub line: usize,
    pub pos: usize,
    pub buffer: Option<Vec<u16>>,
}
impl Position {
    pub fn line<'a>(&'a self, source: &'a Source) -> &'a [u16] {
        self.buffer.as_deref().unwrap_or_else(|| {
            source
                .lines
                .get(self.line)
                .map(Vec::as_slice)
                .unwrap_or_default()
        })
    }
    pub fn next_line(&mut self) {
        self.line += 1;
        self.pos = 0;
        self.buffer = None;
    }
}
#[derive(Clone, Debug)]
pub struct Attribute {
    pub name: String,
    pub value: Vec<u16>,
    pub entity: bool,
    pub macro_arg: bool,
    pub all: bool,
    pub end: usize,
}
#[derive(Clone, Debug)]
pub struct Tag {
    pub name: String,
    pub attributes: Vec<Attribute>,
    pub raw: Vec<u16>,
    pub start: usize,
    pub delimiter: usize,
    pub line_command: bool,
    pub multiline_end: Option<usize>,
}
impl Tag {
    pub fn advance(&self, position: &mut Position) {
        if let Some(line) = self.multiline_end {
            position.line = line + 1;
            position.pos = 0;
            position.buffer = None;
        } else if self.line_command {
            position.next_line();
        } else {
            position.pos = self.delimiter + 1;
        }
    }
}
fn ws(c: u16) -> bool {
    matches!(c, 9 | 32)
}
fn at(s: &[u16], p: usize) -> u16 {
    s.get(p).copied().unwrap_or(0)
}
fn lower(s: &[u16]) -> String {
    String::from_utf16_lossy(s).to_lowercase()
}
/// Scan one tag. Evaluation remains incremental in the native host; `end` gives
/// the cursor position at which each attribute's expression is evaluated.
pub fn scan_tag(source: &Source, position: &mut Position, multiline: bool) -> Result<Tag> {
    let start = position.pos;
    let mut line = position.line(source).to_vec();
    let line_command = position.buffer.is_none() && start == 0 && line.first() == Some(&64);
    let delim = if line_command { 0 } else { 93 };
    let mut p = start + 1;
    while ws(at(&line, p)) {
        p += 1;
    }
    let name_start = p;
    while at(&line, p) != 0 && !ws(at(&line, p)) && at(&line, p) != delim {
        p += 1;
    }
    ensure!(p > name_start, "missing KAG tag name");
    let name = lower(&line[name_start..p]);
    let mut attributes = Vec::new();
    let mut multi = position.line;
    loop {
        while ws(at(&line, p)) {
            p += 1;
        }
        if at(&line, p) == delim {
            break;
        }
        if multiline && at(&line, p) == 92 && at(&line, p + 1) == 0 {
            multi += 1;
            let next = source
                .lines
                .get(multi)
                .context("missing KAG continuation line")?;
            ensure!(
                next.first() == Some(&59),
                "KAG continuation must start with ';'"
            );
            line.pop();
            line.extend_from_slice(&next[1..]);
            position.buffer = Some(line.clone());
        }
        ensure!(at(&line, p) != 0, "unterminated KAG tag");
        if at(&line, p) == 42 {
            p += 1;
            attributes.push(Attribute {
                name: String::new(),
                value: Vec::new(),
                entity: false,
                macro_arg: false,
                all: true,
                end: p,
            });
            continue;
        }
        let begin = p;
        while at(&line, p) != 0 && !ws(at(&line, p)) && at(&line, p) != 61 && at(&line, p) != delim
        {
            p += 1;
        }
        let key = lower(&line[begin..p]);
        while ws(at(&line, p)) {
            p += 1;
        }
        let mut entity = false;
        let mut macro_arg = false;
        let mut value = Vec::new();
        if at(&line, p) != 61 {
            value.extend("true".encode_utf16());
        } else {
            p += 1;
            while ws(at(&line, p)) {
                p += 1;
            }
            ensure!(at(&line, p) != 0, "missing KAG attribute value");
            match at(&line, p) {
                38 => {
                    entity = true;
                    p += 1;
                }
                37 => {
                    macro_arg = true;
                    p += 1;
                }
                _ => {}
            }
            let quote = if matches!(at(&line, p), 34 | 39) {
                let c = line[p];
                p += 1;
                c
            } else {
                0
            };
            let begin = p;
            while at(&line, p) != 0
                && if quote != 0 {
                    at(&line, p) != quote
                } else {
                    at(&line, p) != delim && !ws(at(&line, p))
                }
            {
                if at(&line, p) == 96 {
                    p += 1;
                    ensure!(at(&line, p) != 0, "truncated KAG escape");
                }
                p += 1;
            }
            ensure!(
                quote == 0 || at(&line, p) == quote,
                "unterminated KAG attribute quote"
            );
            ensure!(line_command || at(&line, p) != 0, "unterminated KAG tag");
            let mut q = begin;
            if !entity && q < p && line[q] == 38 {
                entity = true;
                q += 1;
            }
            if !macro_arg && q < p && line[q] == 37 {
                macro_arg = true;
                q += 1;
            }
            while q < p {
                if line[q] == 96 {
                    q += 1;
                }
                if q < p {
                    value.push(line[q]);
                    q += 1;
                }
            }
            if quote != 0 {
                p += 1;
            }
        }
        attributes.push(Attribute {
            name: key,
            value,
            entity,
            macro_arg,
            all: false,
            end: p,
        });
        ensure!(
            attributes.len() <= 65536,
            "KAG tag attribute limit exceeded"
        );
    }
    position.pos = p;
    let mut raw = Vec::new();
    raw.push(91);
    raw.extend_from_slice(&line[start + 1..p]);
    raw.push(93);
    Ok(Tag {
        name,
        attributes,
        raw,
        start,
        delimiter: p,
        line_command,
        multiline_end: (multi > position.line).then_some(multi),
    })
}
