use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
/// An arena-owned dispatch object and its optional bound `this` context.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectRef {
    pub object: Option<usize>,
    pub context: Option<usize>,
}
/// TJS strings retain UTF-16 code units, including isolated surrogates.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Value {
    Void,
    Integer(i64),
    Real(f64),
    String(Vec<u16>),
    Object(ObjectRef),
}
impl Value {
    pub const NULL: Self = Self::Object(ObjectRef {
        object: None,
        context: None,
    });
    /// Unbound object handle. The VM validates the ID when it is used.
    pub fn object(id: usize) -> Self {
        Self::Object(ObjectRef {
            object: Some(id),
            context: None,
        })
    }
    pub fn string(s: &str) -> Self {
        Self::String(s.encode_utf16().collect())
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Integer(_) => "Integer",
            Self::Real(_) => "Real",
            Self::String(_) => "String",
            Self::Object(_) => "Object",
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Void => String::new(),
            Self::Integer(n) => n.to_string(),
            Self::Real(n) => n.to_string(),
            Self::String(s) => String::from_utf16_lossy(s),
            Self::Object(o) => {
                if o.object.is_none() {
                    "(object)(0x00000000)".into()
                } else {
                    format!("(object)({:?})", o.object)
                }
            }
        }
    }
    pub fn number(&self) -> Result<Self> {
        Ok(match self {
            Self::Void => Self::Integer(0),
            Self::String(s) => parse_number(&String::from_utf16_lossy(s)),
            Self::Object(_) => bail!("cannot convert TJS Object to number"),
            v => v.clone(),
        })
    }
    pub fn integer(&self) -> Result<i64> {
        match self.number()? {
            Self::Integer(n) => Ok(n),
            Self::Real(n) => Ok(n as i64),
            _ => unreachable!(),
        }
    }
    pub fn real(&self) -> Result<f64> {
        match self.number()? {
            Self::Integer(n) => Ok(n as f64),
            Self::Real(n) => Ok(n),
            _ => unreachable!(),
        }
    }
    pub fn truth(&self) -> Result<bool> {
        Ok(match self {
            Self::Real(n) => *n != 0.0,
            Self::Object(o) => o.object.is_some(),
            _ => self.integer()? != 0,
        })
    }
    pub fn strict_equal(&self, b: &Self) -> bool {
        self == b
    }
    pub fn equal(&self, b: &Self) -> bool {
        match (self, b) {
            (Self::Integer(a), Self::Integer(b)) => return a == b,
            (Self::String(a), Self::String(b)) => return a == b,
            (Self::Object(a), Self::Object(b)) => return a == b,
            (Self::Object(_), _) | (_, Self::Object(_)) => return false,
            _ => (),
        }
        if matches!(self, Self::String(_)) || matches!(b, Self::String(_)) {
            return self.text() == b.text();
        }
        self.real()
            .ok()
            .zip(b.real().ok())
            .is_some_and(|(a, b)| a == b)
    }
    pub fn unary(&self, op: &str) -> Result<Self> {
        Ok(match op {
            "+" => self.number()?,
            "-" => match self.number()? {
                Self::Integer(n) => Self::Integer(n.wrapping_neg()),
                Self::Real(n) => Self::Real(-n),
                _ => unreachable!(),
            },
            "!" => Self::Integer(i64::from(!self.truth()?)),
            "~" => Self::Integer(!self.integer()?),
            "int" => Self::Integer(self.integer()?),
            "real" => Self::Real(self.real()?),
            "string" => Self::String(match self {
                Self::String(s) => s.clone(),
                _ => self.text().encode_utf16().collect(),
            }),
            "typeof" => Self::string(self.type_name()),
            "#" => match self {
                Self::String(s) => Self::Integer(s.first().copied().unwrap_or(0) as i64),
                _ => Self::Integer(self.text().encode_utf16().next().unwrap_or(0) as i64),
            },
            "$" => Self::String(vec![self.integer()? as u16]),
            _ => bail!("unsupported TJS unary operator {op}"),
        })
    }
    pub fn binary(&self, op: &str, b: &Self) -> Result<Self> {
        let boolean = |v| Ok(Self::Integer(i64::from(v)));
        match op {
            "==" => return boolean(self.equal(b)),
            "!=" => return boolean(!self.equal(b)),
            "===" => return boolean(self.strict_equal(b)),
            "!==" => return boolean(!self.strict_equal(b)),
            "+" if matches!(self, Self::String(_)) || matches!(b, Self::String(_)) => {
                let mut units = match self {
                    Self::String(s) => s.clone(),
                    _ => self.text().encode_utf16().collect(),
                };
                units.extend(match b {
                    Self::String(s) => s.clone(),
                    _ => b.text().encode_utf16().collect(),
                });
                return Ok(Self::String(units));
            }
            "<" | ">" | "<=" | ">=" => {
                let order = match (self, b) {
                    (Self::String(a), Self::String(b)) => a.partial_cmp(b),
                    (Self::Integer(a), Self::Integer(b)) => a.partial_cmp(b),
                    _ => self.real()?.partial_cmp(&b.real()?),
                };
                return boolean(match op {
                    "<" => order.is_some_and(|o| o.is_lt()),
                    ">" => order.is_some_and(|o| o.is_gt()),
                    "<=" => order.is_some_and(|o| o.is_le()),
                    _ => order.is_some_and(|o| o.is_ge()),
                });
            }
            "&" | "|" | "^" | "<<" | ">>" | ">>>" | "\\" | "%" => {
                let (a, b) = (self.integer()?, b.integer()?);
                let n = match op {
                    "&" => a & b,
                    "|" => a | b,
                    "^" => a ^ b,
                    "<<" => a.wrapping_shl(b as u32),
                    ">>" => a.wrapping_shr(b as u32),
                    ">>>" => (a as u64).wrapping_shr(b as u32) as i64,
                    "\\" | "%" => {
                        if b == 0 {
                            bail!("TJS integer division by zero");
                        }
                        if op == "%" {
                            a.wrapping_rem(b)
                        } else {
                            a.wrapping_div(b)
                        }
                    }
                    _ => unreachable!(),
                };
                return Ok(Self::Integer(n));
            }
            _ => (),
        }
        let (a, b) = (self.number()?, b.number()?);
        if let (Self::Integer(a), Self::Integer(b)) = (&a, &b) {
            match op {
                "+" => return Ok(Self::Integer(a.wrapping_add(*b))),
                "-" => return Ok(Self::Integer(a.wrapping_sub(*b))),
                "*" => return Ok(Self::Integer(a.wrapping_mul(*b))),
                _ => (),
            }
        }
        let (a, b) = (a.real()?, b.real()?);
        Ok(Self::Real(match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            _ => bail!("unsupported TJS binary operator {op}"),
        }))
    }
}
/// Numeric prefix conversion, as used by TJS strings rather than JS Number().
pub fn parse_number(text: &str) -> Value {
    let mut s = text.trim_start();
    let negative = s.starts_with('-');
    if s.starts_with(['+', '-']) {
        s = s[1..].trim_start();
    }
    let sign = if negative { -1i64 } else { 1 };
    let (radix, prefix) = if s.starts_with("0x") || s.starts_with("0X") {
        (16, 2)
    } else if s.starts_with("0b") || s.starts_with("0B") {
        (2, 2)
    } else if s.len() > 1 && s.starts_with('0') && s.as_bytes()[1].is_ascii_digit() {
        (8, 1)
    } else {
        (10, 0)
    };
    if radix != 10 {
        let digits = s[prefix..]
            .chars()
            .take_while(|c| c.is_digit(radix))
            .collect::<String>();
        let n = digits.chars().fold(0u64, |n, c| {
            n.wrapping_mul(radix as u64)
                .wrapping_add(c.to_digit(radix).unwrap() as u64)
        });
        return Value::Integer((n as i64).wrapping_mul(sign));
    }
    let mut end = 0;
    let mut real = false;
    let bytes = s.as_bytes();
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        real = true;
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }
    if bytes.get(end).is_some_and(|c| matches!(c, b'e' | b'E')) {
        let before = end;
        end += 1;
        if bytes.get(end).is_some_and(|c| matches!(c, b'+' | b'-')) {
            end += 1;
        }
        let first = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == first {
            end = before;
        } else {
            real = true;
        }
    }
    let s = &s[..end];
    if real {
        Value::Real(s.parse::<f64>().unwrap_or(0.0) * sign as f64)
    } else {
        let n = s.bytes().fold(0u64, |n, digit| {
            n.wrapping_mul(10).wrapping_add((digit - b'0') as u64)
        });
        Value::Integer(n as i64).unary_sign(sign)
    }
}
impl Value {
    fn unary_sign(self, sign: i64) -> Self {
        match self {
            Self::Integer(n) => Self::Integer(n.wrapping_mul(sign)),
            _ => self,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tjs_coercions() {
        assert!(!Value::string("hello").truth().unwrap());
        assert!(!Value::string("0.5").truth().unwrap());
        assert!(Value::string("2tail").truth().unwrap());
        assert_eq!(Value::string("0xff").integer().unwrap(), 255);
        assert!(!Value::string("01").equal(&Value::Integer(1)));
        assert!(Value::Void.equal(&Value::Integer(0)));
        assert!(Value::Void.equal(&Value::string("")));
        assert!(!Value::Integer(1).strict_equal(&Value::Real(1.0)));
        assert_eq!(
            Value::Integer(3).binary("/", &Value::Integer(2)).unwrap(),
            Value::Real(1.5)
        );
        assert!(Value::Integer(1).binary("%", &Value::Integer(0)).is_err());
    }
    #[test]
    fn utf16_not_utf8() {
        let Value::String(s) = Value::string("a😀") else {
            panic!()
        };
        assert_eq!(s.len(), 3);
        assert_eq!(
            Value::String(vec![0xd800])
                .binary("+", &Value::string("x"))
                .unwrap(),
            Value::String(vec![0xd800, 120])
        );
    }
    #[test]
    fn integers_preserve_all_bits() {
        let low = Value::Integer(9_007_199_254_740_992);
        let high = Value::Integer(9_007_199_254_740_993);
        assert!(!low.equal(&high));
        assert_eq!(low.binary("<", &high).unwrap(), Value::Integer(1));
        assert_eq!(parse_number("18446744073709551617"), Value::Integer(1));
        assert_eq!(parse_number("- 18446744073709551617"), Value::Integer(-1));
        assert!(!Value::String(vec![0xd800]).equal(&Value::String(vec![0xd801])));
    }
}
