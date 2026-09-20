use crate::Value;
use anyhow::{Result, bail, ensure};
use krkrz_core::SourceLocation;
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Name(String),
    Literal(Value),
    Symbol(String),
    Eof,
}
#[derive(Clone, Debug)]
pub struct Token {
    pub kind: Kind,
    pub location: SourceLocation,
}
struct Lexer<'a> {
    source: &'a str,
    storage: &'a str,
    pos: usize,
    line: usize,
    column: usize,
}
impl Lexer<'_> {
    fn location(&self) -> SourceLocation {
        SourceLocation {
            storage: self.storage.into(),
            line: self.line,
            column: self.column,
        }
    }
    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += c.len_utf16();
        }
        Some(c)
    }
    fn skip(&mut self) -> Result<()> {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.source[self.pos..].starts_with("//") {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.bump();
                }
            } else if self.source[self.pos..].starts_with("/*") {
                self.bump();
                self.bump();
                let mut depth = 1;
                while depth > 0 {
                    if self.source[self.pos..].starts_with("/*") {
                        self.bump();
                        self.bump();
                        depth += 1;
                    } else if self.source[self.pos..].starts_with("*/") {
                        self.bump();
                        self.bump();
                        depth -= 1;
                    } else {
                        ensure!(self.bump().is_some(), "unterminated TJS comment");
                    }
                }
            } else {
                return Ok(());
            }
        }
    }
    fn next(&mut self) -> Result<Token> {
        self.skip()?;
        let location = self.location();
        let Some(c) = self.peek() else {
            return Ok(Token {
                kind: Kind::Eof,
                location,
            });
        };
        let kind = if c == '_' || c == '$' || c.is_alphabetic() {
            let start = self.pos;
            while self
                .peek()
                .is_some_and(|c| c == '_' || c == '$' || c.is_alphanumeric())
            {
                self.bump();
            }
            match &self.source[start..self.pos] {
                "void" => Kind::Literal(Value::Void),
                "true" => Kind::Literal(Value::Integer(1)),
                "false" => Kind::Literal(Value::Integer(0)),
                s => Kind::Name(s.into()),
            }
        } else if c.is_ascii_digit()
            || (c == '.'
                && self.source[self.pos + 1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit()))
        {
            let start = self.pos;
            self.bump();
            if c == '0'
                && self
                    .peek()
                    .is_some_and(|c| matches!(c, 'x' | 'X' | 'b' | 'B'))
            {
                self.bump();
                while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.bump();
                }
            } else {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
                if self.peek() == Some('.') {
                    self.bump();
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.bump();
                    }
                }
                if self.peek().is_some_and(|c| c == 'e' || c == 'E') {
                    self.bump();
                    if self.peek().is_some_and(|c| c == '+' || c == '-') {
                        self.bump();
                    }
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.bump();
                    }
                }
            }
            Kind::Literal(crate::value::parse_number(&self.source[start..self.pos]))
        } else if c == '\'' || c == '"' {
            self.bump();
            let mut units = Vec::new();
            loop {
                let ch = self
                    .bump()
                    .ok_or_else(|| anyhow::anyhow!("unterminated TJS string"))?;
                if ch == c {
                    break;
                }
                if ch == '\\' {
                    let ch = self
                        .bump()
                        .ok_or_else(|| anyhow::anyhow!("truncated string escape"))?;
                    if ch == 'x' {
                        let mut n = 0;
                        let mut count = 0;
                        while count < 4 {
                            let Some(d) = self.peek().and_then(|c| c.to_digit(16)) else {
                                break;
                            };
                            self.bump();
                            n = n * 16 + d;
                            count += 1;
                        }
                        ensure!(count > 0, "empty TJS hex escape");
                        units.push(n as u16);
                    } else {
                        let ch = match ch {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'b' => '\u{8}',
                            'f' => '\u{c}',
                            'v' => '\u{b}',
                            'a' => '\u{7}',
                            '0' => '\0',
                            v => v,
                        };
                        units.extend(ch.encode_utf16(&mut [0; 2]).iter().copied());
                    }
                } else {
                    units.extend(ch.encode_utf16(&mut [0; 2]).iter().copied());
                }
            }
            Kind::Literal(Value::String(units))
        } else {
            let mut symbol = None;
            for op in [
                ">>>=", "===", "!==", ">>>", "<<=", ">>=", "...", "==", "!=", "<=", ">=", "&&",
                "||", "<<", ">>", "+=", "-=", "*=", "/=", "++", "--", "=>", "%[", "<%", "%>",
            ] {
                if self.source[self.pos..].starts_with(op) {
                    for _ in op.chars() {
                        self.bump();
                    }
                    symbol = Some(op.to_string());
                    break;
                }
            }
            if let Some(s) = symbol {
                Kind::Symbol(s)
            } else if "{}[]();,.?:+-*/%\\!~&|^=<>@#".contains(c) {
                self.bump();
                Kind::Symbol(c.to_string())
            } else {
                bail!("unsupported TJS character {c:?}")
            }
        };
        Ok(Token { kind, location })
    }
}
pub fn lex(storage: &str, source: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer {
        source,
        storage,
        pos: 0,
        line: 1,
        column: 1,
    };
    let mut result = Vec::new();
    loop {
        let token = lexer
            .next()
            .map_err(|e| anyhow::anyhow!("{storage}:{}:{}: {e}", lexer.line, lexer.column))?;
        let done = token.kind == Kind::Eof;
        result.push(token);
        ensure!(result.len() <= 1_000_000, "TJS token budget exceeded");
        if done {
            return Ok(result);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comments_and_utf16() {
        let t = lex("t", "/* a /* b */ */\n'\\xd800' // x\n42").unwrap();
        assert_eq!(t[0].kind, Kind::Literal(Value::String(vec![0xd800])));
        assert_eq!(t[0].location.line, 2);
        assert_eq!(t[1].location.line, 3);
    }
    #[test]
    fn malformed() {
        for s in ["'bad", "/* missing", "'\\x'"] {
            assert!(lex("t", s).is_err());
        }
    }
}
