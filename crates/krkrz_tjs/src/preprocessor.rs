//! TJS conditional compilation: a separate, eager, signed 32-bit expression language.
//! Reference: Kirikiri tjs2/syntax/tjspp.y and tjsCompileControl.cpp.
use crate::lexer::{Kind, Token, lex_pp};
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Preprocessor {
    values: BTreeMap<String, i32>,
}
impl Default for Preprocessor {
    fn default() -> Self {
        // TJS2 2.4.28, the requested target version. Host flags are registered by the host.
        Self {
            values: BTreeMap::from([("version".into(), 0x0204001c)]),
        }
    }
}
impl Preprocessor {
    pub fn get(&self, name: &str) -> i32 {
        self.values.get(name).copied().unwrap_or(0)
    }
    pub fn set(&mut self, name: impl Into<String>, value: i32) {
        self.values.insert(name.into(), value);
    }
    pub fn evaluate(&mut self, source: &str) -> Result<i32> {
        ensure!(
            !source.contains("/*") && !source.contains("//"),
            "comments in preprocessor expression"
        );
        let tokens = lex_pp(source)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            state: self,
            depth: 0,
        };
        let result = parser.expression(0)?.0;
        ensure!(
            parser.tokens[parser.pos].kind == Kind::Eof,
            "unexpected preprocessor expression token"
        );
        Ok(result)
    }
}
struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    state: &'a mut Preprocessor,
    depth: usize,
}
impl Parser<'_> {
    fn expression(&mut self, min: u8) -> Result<(i32, Option<String>)> {
        self.depth += 1;
        ensure!(
            self.depth <= 128,
            "preprocessor expression nesting exceeds limit"
        );
        let token = self.tokens[self.pos].kind.clone();
        self.pos += 1;
        let mut left = match token {
            Kind::Literal(v @ (crate::Value::Integer(_) | crate::Value::Real(_))) => {
                (v.integer()? as i32, None)
            }
            Kind::Name(n) => (self.state.get(&n), Some(n)),
            Kind::Symbol(s) if s == "(" => {
                let value = self.expression(0)?.0;
                ensure!(
                    self.tokens[self.pos].kind == Kind::Symbol(")".into()),
                    "expected preprocessor closing parenthesis"
                );
                self.pos += 1;
                (value, None)
            }
            Kind::Symbol(s) if ["!", "+", "-"].contains(&s.as_str()) => {
                let value = self.expression(12)?.0;
                (
                    match s.as_str() {
                        "!" => i32::from(value == 0),
                        "-" => value.wrapping_neg(),
                        _ => value,
                    },
                    None,
                )
            }
            _ => bail!("expected preprocessor operand"),
        };
        while let Kind::Symbol(op) = self.tokens[self.pos].kind.clone() {
            let precedence = match op.as_str() {
                "," => 1,
                "||" => 2,
                "&&" => 3,
                "|" => 4,
                "^" => 5,
                "&" => 6,
                "=" => 7,
                "==" | "!=" => 8,
                "<" | ">" | "<=" | ">=" => 9,
                "+" | "-" => 10,
                "*" | "/" | "%" => 11,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.pos += 1;
            // Unlike runtime &&/||, preprocessor operands are both evaluated.
            let right = self.expression(precedence + 1)?.0;
            left.0 = match op.as_str() {
                "=" => {
                    let name = left
                        .1
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("invalid preprocessor assignment"))?;
                    self.state.set(name, right);
                    right
                }
                "," => right,
                "||" => i32::from(left.0 != 0 || right != 0),
                "&&" => i32::from(left.0 != 0 && right != 0),
                "|" => left.0 | right,
                "^" => left.0 ^ right,
                "&" => left.0 & right,
                "==" => i32::from(left.0 == right),
                "!=" => i32::from(left.0 != right),
                "<" => i32::from(left.0 < right),
                ">" => i32::from(left.0 > right),
                "<=" => i32::from(left.0 <= right),
                ">=" => i32::from(left.0 >= right),
                "+" => left.0.wrapping_add(right),
                "-" => left.0.wrapping_sub(right),
                "*" => left.0.wrapping_mul(right),
                "/" | "%" => {
                    ensure!(right != 0, "division by zero in preprocessor expression");
                    if op == "/" {
                        left.0.wrapping_div(right)
                    } else {
                        left.0.wrapping_rem(right)
                    }
                }
                _ => unreachable!(),
            };
            left.1 = None;
        }
        self.depth -= 1;
        Ok(left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_integer_rules_and_eager_operands() {
        let mut p = Preprocessor::default();
        assert_eq!(p.get("version"), 0x0204001c);
        assert_eq!(p.evaluate("7%3").unwrap(), 1);
        assert_eq!(p.evaluate("x=0,0 && (x=7),x").unwrap(), 7);
        assert_eq!(p.evaluate("2147483647+1").unwrap(), i32::MIN);
        assert_eq!(p.evaluate("unknown+2").unwrap(), 2);
        assert_eq!(p.evaluate("a=1|2").unwrap(), 3);
        assert_eq!(p.get("a"), 1); // assignment binds more tightly than bitwise OR
        for source in ["1/0", "1%0", "x=", "(", "1<<2", "\"2\"", "1+/*x*/2"] {
            assert!(p.evaluate(source).is_err(), "{source}");
        }
    }
    #[test]
    fn skipped_code_comments_and_locations() {
        let source = "@set(x=0)\n@if(x)\ninvalid ' code\n/* @endif */\n@if(1/0) @set(x=8) @endif\n@endif\nreturn @'@if(0)';";
        let mut p = Preprocessor::default();
        let tokens = crate::lexer::lex_with_preprocessor("pp", source, &mut p).unwrap();
        assert_eq!(tokens[0].location.line, 7);
        assert_eq!(tokens[0].kind, Kind::Name("return".into()));
        assert_eq!(p.get("x"), 0);
        for source in ["@if(1)", "@if(0)", "@endif", "@set(1/0)", "@if((1)"] {
            assert!(crate::compile("pp", source).is_err(), "{source}");
        }
    }
}
