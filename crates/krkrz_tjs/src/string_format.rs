//! TJSFormatString's restricted printf grammar, with UTF-16 string fields.
use crate::{Value, unsupported};
use anyhow::{Context, Result, ensure};

pub(crate) fn sprintf(format: &[u16], args: &[Value], budget: &mut u64) -> Result<Value> {
    let format = &format[..format.iter().position(|c| *c == 0).unwrap_or(format.len())];
    let mut out = Vec::new();
    let mut pos = 0;
    let mut args = args.iter();
    let mut next = || args.next().context("String.sprintf: not enough arguments");
    while pos < format.len() {
        if format[pos] != b'%' as u16 {
            append(&mut out, &format[pos..pos + 1], budget)?;
            pos += 1;
            continue;
        }
        pos += 1;
        let mut flag = 0;
        if matches!(format.get(pos), Some(45 | 43 | 35)) {
            flag = format[pos];
            pos += 1;
        }
        let zero = format.get(pos) == Some(&(b'0' as u16));
        pos += usize::from(zero);
        let width_arg = format.get(pos) == Some(&(b'*' as u16));
        let mut width = if width_arg {
            pos += 1;
            0
        } else {
            number(format, &mut pos)?
        };
        let mut precision = None;
        let mut prec_arg = false;
        if format.get(pos) == Some(&(b'.' as u16)) {
            pos += 1;
            prec_arg = format.get(pos) == Some(&(b'*' as u16));
            precision = Some(if prec_arg {
                pos += 1;
                0
            } else {
                ensure!(
                    matches!(format.get(pos), Some(48..=57)),
                    "invalid sprintf precision"
                );
                number(format, &mut pos)?
            });
        }
        let kind = *format.get(pos).context("incomplete sprintf format")?;
        pos += 1;
        if kind == b'%' as u16 {
            append(&mut out, &[kind], budget)?;
            continue;
        }
        ensure!(
            b"diouxXfegEGcs".iter().any(|c| *c as u16 == kind),
            "invalid sprintf conversion"
        );
        if width_arg {
            width = next()?.integer()? as i32;
        }
        if prec_arg {
            precision = Some(next()?.integer()? as i32);
        }
        let value = next()?;
        if matches!(kind, 99 | 115) {
            let Value::String(mut text) = value.unary("string")? else {
                unreachable!()
            };
            if text.is_empty() {
                continue;
            }
            let mut count = precision.filter(|p| *p != 0).unwrap_or(text.len() as i32);
            if kind == 99 {
                count = count.min(1);
            }
            ensure!(
                width >= 0 && count >= 0,
                "invalid sprintf string width or precision"
            );
            let length = width.max(count) as usize;
            charge(length, out.len(), budget)?;
            text.resize(count as usize, 0);
            let padding = length - text.len();
            if flag != 45 {
                out.resize(out.len() + padding, 32);
            }
            out.extend(text);
            if flag == 45 {
                out.resize(out.len() + padding, 32);
            }
            continue;
        }
        // The original formatter uses a 1024-unit temporary numeric buffer.
        ensure!(
            width.unsigned_abs() as u64 + precision.unwrap_or(0).max(0) as u64 <= 900,
            "sprintf numeric field exceeds 900 characters"
        );
        let left = flag == 45 || width < 0;
        let (prefix, digits) = if matches!(kind, 100 | 105 | 111 | 117 | 120 | 88) {
            integer(value.integer()?, kind, flag, precision)
        } else {
            real(value.real()?, kind, flag, precision)
        };
        let padding = (width.unsigned_abs() as usize).saturating_sub(prefix.len() + digits.len());
        let zero =
            zero && !left && (precision.is_none() || matches!(kind, 102 | 101 | 103 | 69 | 71));
        let mut text = String::new();
        if !left && !zero {
            text.extend(std::iter::repeat_n(' ', padding));
        }
        text.push_str(&prefix);
        if zero {
            text.extend(std::iter::repeat_n('0', padding));
        }
        text.push_str(&digits);
        if left {
            text.extend(std::iter::repeat_n(' ', padding));
        }
        append(&mut out, &text.encode_utf16().collect::<Vec<_>>(), budget)?;
    }
    // Native FixLength terminates at embedded NUL, including string precision padding.
    out.truncate(out.iter().position(|c| *c == 0).unwrap_or(out.len()));
    Ok(Value::String(out))
}

fn number(format: &[u16], pos: &mut usize) -> Result<i32> {
    let mut result = 0i32;
    while let Some(c @ 48..=57) = format.get(*pos) {
        result = result
            .checked_mul(10)
            .and_then(|n| n.checked_add((*c - 48).into()))
            .context("sprintf field size overflow")?;
        *pos += 1;
    }
    Ok(result)
}

fn charge(length: usize, existing: usize, budget: &mut u64) -> Result<()> {
    if length > 32_000_000usize.saturating_sub(existing) {
        return Err(unsupported("String.sprintf output exceeds limit"));
    }
    *budget = budget
        .checked_sub(length as u64)
        .ok_or_else(|| unsupported("String.sprintf execution budget exceeded"))?;
    Ok(())
}

fn append(out: &mut Vec<u16>, text: &[u16], budget: &mut u64) -> Result<()> {
    charge(text.len(), out.len(), budget)?;
    out.extend_from_slice(text);
    Ok(())
}

fn integer(n: i64, kind: u16, flag: u16, precision: Option<i32>) -> (String, String) {
    // The installed 1.2.0.3 formatter passes a 32-bit integer to the CRT.
    let n = n as i32;
    let signed = matches!(kind, 100 | 105);
    let mut digits = match kind {
        111 => format!("{:o}", n as u32),
        120 => format!("{:x}", n as u32),
        88 => format!("{:X}", n as u32),
        _ if signed => n.unsigned_abs().to_string(),
        _ => (n as u32).to_string(),
    };
    if precision == Some(0) && n == 0 {
        digits.clear();
    }
    let zeroes = precision.unwrap_or(0).max(0) as usize;
    if digits.len() < zeroes {
        digits = "0".repeat(zeroes - digits.len()) + &digits;
    }
    let prefix = if signed && n < 0 {
        "-"
    } else if signed && flag == 43 {
        "+"
    } else if flag == 35 && kind == 120 && n != 0 {
        "0x"
    } else if flag == 35 && kind == 88 && n != 0 {
        "0X"
    } else if flag == 35 && kind == 111 && !digits.starts_with('0') {
        "0"
    } else {
        ""
    };
    (prefix.into(), digits)
}

fn real(n: f64, kind: u16, flag: u16, precision: Option<i32>) -> (String, String) {
    let prefix = if n.is_sign_negative() {
        "-"
    } else if flag == 43 {
        "+"
    } else {
        ""
    };
    let n = n.abs();
    let precision = precision.filter(|p| *p >= 0).unwrap_or(6) as usize;
    let compact = matches!(kind, 103 | 71);
    let alt = flag == 35;
    let mut digits = if !n.is_finite() {
        // The old Windows CRT treats the marker as decimal digits, including
        // its truncation/rounding and zero padding at the requested precision.
        let count = if compact {
            precision.max(1) - 1
        } else {
            precision
        };
        let marker = if n.is_nan() {
            b"#QNAN".as_slice()
        } else {
            b"#INF".as_slice()
        };
        let mut tail = marker[..count.min(marker.len())].to_vec();
        tail.resize(count, b'0');
        if count > 0 && marker.get(count).is_some_and(|c| *c >= b'5') {
            *tail.last_mut().unwrap() += 1;
        }
        let mut text = "1".to_owned();
        if count > 0 || alt {
            text.push('.');
        }
        text.push_str(std::str::from_utf8(&tail).unwrap());
        if compact && !alt {
            text = trim_decimal(&text).into();
        }
        if matches!(kind, 101 | 69) {
            text.push_str(if kind == 69 { "E+000" } else { "e+000" });
        }
        text
    } else if kind == 102 {
        format!("{:.precision$}", rounding_input(n, precision as i32))
    } else {
        let significant = if compact {
            precision.max(1) - 1
        } else {
            precision
        };
        let exponent: i32 = format!("{n:e}").split_once('e').unwrap().1.parse().unwrap();
        let rounded = rounding_input(n, significant as i32 - exponent);
        let scientific = format!("{rounded:.significant$e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap();
        let exponent: i32 = exponent.parse().unwrap();
        if compact && exponent >= -4 && exponent < precision.max(1) as i32 {
            let decimals = (precision.max(1) as i32 - 1 - exponent).max(0) as usize;
            let fixed = format!("{:.decimals$}", rounding_input(n, decimals as i32));
            if alt {
                fixed
            } else {
                trim_decimal(&fixed).into()
            }
        } else {
            let mut mantissa = if compact && !alt {
                trim_decimal(mantissa).to_owned()
            } else {
                mantissa.to_owned()
            };
            if alt && !mantissa.contains('.') {
                mantissa.push('.');
            }
            let e = if matches!(kind, 69 | 71) { 'E' } else { 'e' };
            format!("{mantissa}{e}{exponent:+04}")
        }
    };
    if alt && !digits.contains(['.', 'e', 'E']) {
        digits.push('.');
    }
    (prefix.into(), digits)
}

fn trim_decimal(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

// MSVCRT rounds exact decimal ties away from zero; Rust uses ties-to-even.
// Detect ties from the binary representation without rounding a scaled float.
fn rounding_input(n: f64, decimals: i32) -> f64 {
    if n == 0.0 {
        return n;
    }
    let bits = n.to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    let mut mantissa = bits & ((1 << 52) - 1);
    let exponent = if encoded_exponent == 0 {
        -1074
    } else {
        mantissa |= 1 << 52;
        encoded_exponent - 1023 - 52
    };
    if decimals < 0 {
        let Some(divisor) = 5u64.checked_pow(decimals.unsigned_abs()) else {
            return n;
        };
        if !mantissa.is_multiple_of(divisor) {
            return n;
        }
        mantissa /= divisor;
    }
    let binary_shift = exponent + decimals;
    if binary_shift < 0 && mantissa.trailing_zeros() as i32 == -binary_shift - 1 {
        n.next_up()
    } else {
        n
    }
}
