//! RegExp bindings using a Rust matcher; offsets exposed to TJS are UTF-16 units.
use crate::{Host, Value, Vm, object::ObjectKind, unsupported};
use anyhow::{Context, Result, ensure};
use fancy_regex::{Regex, RegexBuilder};
use std::sync::Arc;

fn text(value: &Value) -> Result<String> {
    if let Value::String(units) = value {
        String::from_utf16(units)
            .map_err(|_| unsupported("RegExp with isolated UTF-16 surrogates is not implemented"))
    } else {
        Ok(value.text())
    }
}
fn charge(budget: &mut u64) -> Result<()> {
    if *budget == 0 {
        return Err(unsupported("RegExp execution budget exhausted"));
    }
    *budget -= 1;
    Ok(())
}
fn captures(regex: &Regex, subject: &str, start: usize) -> Result<Option<Vec<(usize, usize)>>> {
    let mut position = start;
    while position <= subject.len() {
        let Some(captures) = regex
            .captures_from_pos(subject, position)
            .map_err(|e| unsupported(format!("RegExp matcher failed: {e}")))?
        else {
            return Ok(None);
        };
        let matched = captures.get(0).unwrap();
        // Kirikiri uses ONIG_OPTION_FIND_NOT_EMPTY.
        if matched.start() != matched.end() {
            return Ok(Some(
                (0..captures.len())
                    .map(|i| {
                        captures
                            .get(i)
                            .map(|m| (m.start(), m.end()))
                            .unwrap_or((0, 0))
                    })
                    .collect(),
            ));
        }
        let Some(ch) = subject[matched.end()..].chars().next() else {
            return Ok(None);
        };
        position = matched.end() + ch.len_utf8();
    }
    Ok(None)
}
impl Vm {
    pub(crate) fn regexp_new(&mut self, args: &[Value]) -> Result<Value> {
        let pattern = args
            .first()
            .map(text)
            .transpose()?
            .unwrap_or_default()
            .replace("\\/", "/");
        let flags = args.get(1).map(Value::text).unwrap_or_default();
        let compiled = RegexBuilder::new(&pattern)
            .case_insensitive(flags.contains('i'))
            .backtrack_limit(100_000)
            .build()?;
        let value = self.allocate(ObjectKind::RegExp {
            compiled: Arc::new(compiled),
            global: flags.contains('g'),
        })?;
        for key in ["start", "index", "lastIndex"] {
            self.set_member(&value, &Value::string(key), Value::Integer(0))?;
        }
        for key in [
            "input",
            "lastMatch",
            "lastParen",
            "leftContext",
            "rightContext",
        ] {
            self.set_member(&value, &Value::string(key), Value::string(""))?;
        }
        let matches = self.allocate(ObjectKind::Array(vec![]))?;
        self.set_member(&value, &Value::string("matches"), matches)?;
        Ok(value)
    }
    pub(crate) fn regexp_method(
        &mut self,
        receiver: &Value,
        name: &str,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = self.object_id(receiver)?;
        let ObjectKind::RegExp { compiled, global } = self.objects[id].kind.clone() else {
            return Err(unsupported("string operation requires a RegExp object"));
        };
        let subject = text(args.first().context("RegExp requires a target string")?)?;
        ensure!(subject.len() <= 32_000_000, "RegExp subject exceeds limit");
        charge(budget)?;
        if name == "replace" || name == "split" {
            let replacement = if name == "replace" {
                Some(
                    args.get(1)
                        .context("RegExp.replace requires a replacement")?,
                )
            } else {
                None
            };
            let purge = args.get(2).is_some_and(|v| v.truth().unwrap_or(false));
            let mut result = String::new();
            let mut parts = vec![];
            let mut offset = 0;
            while let Some(groups) = captures(&compiled, &subject[offset..], 0)? {
                charge(budget)?;
                let (begin, end) = groups[0];
                let prefix = &subject[offset..offset + begin];
                if let Some(replacement) = replacement {
                    result.push_str(prefix);
                    if matches!(replacement, Value::Object(_)) {
                        let matches = groups
                            .iter()
                            .map(|(a, b)| Value::string(&subject[offset + a..offset + b]))
                            .collect();
                        let matches = self.allocate(ObjectKind::Array(matches))?;
                        result.push_str(&text(&self.invoke(
                            replacement,
                            receiver,
                            &[matches],
                            host,
                            budget,
                        )?)?);
                    } else {
                        result.push_str(&text(replacement)?);
                    }
                } else if !purge || !prefix.is_empty() {
                    parts.push(Value::string(prefix));
                }
                offset += end;
                ensure!(
                    result.len() <= 32_000_000 && parts.len() <= 1_000_000,
                    "RegExp output exceeds limit"
                );
                if (name == "replace" && !global) || offset == subject.len() {
                    break;
                }
            }
            let tail = &subject[offset..];
            if name == "replace" {
                result.push_str(tail);
                return Ok(Value::string(&result));
            }
            if !purge || !tail.is_empty() {
                parts.push(Value::string(tail));
            }
            return self.allocate(ObjectKind::Array(parts));
        }
        let start = self
            .get_member(receiver, &Value::string("start"), false)?
            .integer()?
            .max(0) as usize;
        let mut units = 0;
        let mut byte_start = subject.len();
        for (i, ch) in subject.char_indices() {
            if units >= start {
                byte_start = i;
                break;
            }
            units += ch.len_utf16();
        }
        let groups = captures(&compiled, &subject, byte_start)?;
        let matches = self.allocate(ObjectKind::Array(
            groups
                .as_ref()
                .map(|groups| {
                    groups
                        .iter()
                        .map(|(a, b)| Value::string(&subject[*a..*b]))
                        .collect()
                })
                .unwrap_or_default(),
        ))?;
        if name == "match" {
            return Ok(matches);
        }
        if name != "exec" && name != "test" {
            return Err(unsupported(format!("unsupported RegExp operation: {name}")));
        }
        self.set_member(receiver, &Value::string("input"), Value::string(&subject))?;
        self.set_member(receiver, &Value::string("matches"), matches.clone())?;
        let (begin, end) = groups
            .as_ref()
            .map(|g| g[0])
            .unwrap_or((byte_start, byte_start));
        let end_units = subject[..end].encode_utf16().count();
        for (key, value) in [
            (
                "index",
                Value::Integer(subject[..begin].encode_utf16().count() as i64),
            ),
            ("lastIndex", Value::Integer(end_units as i64)),
            ("lastMatch", Value::string(&subject[begin..end])),
            (
                "lastParen",
                Value::string(
                    groups
                        .as_ref()
                        .and_then(|g| g.last())
                        .map(|(a, b)| &subject[*a..*b])
                        .unwrap_or(""),
                ),
            ),
            ("leftContext", Value::string(&subject[..begin])),
            ("rightContext", Value::string(&subject[end..])),
        ] {
            self.set_member(receiver, &Value::string(key), value)?;
        }
        if global && groups.is_some() {
            self.set_member(
                receiver,
                &Value::string("start"),
                Value::Integer(end_units as i64),
            )?;
        }
        if let Some(class) = self.globals.get("RegExp").cloned() {
            self.set_member(&class, &Value::string("lastRegExp"), receiver.clone())?;
        }
        if name == "test" {
            Ok(Value::Integer(i64::from(groups.is_some())))
        } else {
            Ok(matches)
        }
    }
}
