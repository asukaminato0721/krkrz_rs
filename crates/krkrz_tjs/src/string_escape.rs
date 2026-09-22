//! TJS string.escape(), following tTJSString::EscapeC in tjsString.cpp.
use crate::{Value, unsupported};
use anyhow::Result;

pub(crate) fn escape(units: &[u16], budget: &mut u64) -> Result<Value> {
    let mut output = Vec::new();
    let mut hex = false;
    for &unit in units.iter().take_while(|unit| **unit != 0) {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| unsupported("String.escape execution budget exceeded"))?;
        let short = match unit {
            7 => Some(b'a'),
            8 => Some(b'b'),
            12 => Some(b'f'),
            10 => Some(b'n'),
            13 => Some(b'r'),
            9 => Some(b't'),
            11 => Some(b'v'),
            92 => Some(b'\\'),
            39 => Some(b'\''),
            34 => Some(b'"'),
            _ => None,
        };
        if let Some(short) = short {
            output.extend([b'\\' as u16, short as u16]);
            hex = false;
        } else if unit < 32 || hex && matches!(unit, 48..=57 | 65..=70 | 97..=102) {
            // Escape following hex digits too, so the lexer cannot absorb them
            // into the preceding variable-length hexadecimal escape.
            const DIGITS: &[u8] = b"0123456789abcdef";
            output.extend([
                b'\\' as u16,
                b'x' as u16,
                DIGITS[(unit >> 4) as usize] as u16,
                DIGITS[(unit & 15) as usize] as u16,
            ]);
            hex = true;
        } else {
            output.push(unit);
            hex = false;
        }
        if output.len() > 32_000_000 {
            return Err(unsupported("escaped string exceeds limit"));
        }
    }
    Ok(Value::String(output))
}
