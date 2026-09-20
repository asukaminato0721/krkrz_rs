use crate::object::ObjectKind;
use crate::{Host, Value, Vm, unsupported};
use anyhow::{Context, Result, bail, ensure};

fn units(value: &Value) -> Result<Vec<u16>> {
    match value.unary("string")? {
        Value::String(s) => Ok(s),
        _ => unreachable!(),
    }
}
fn index(key: &Value) -> Option<i64> {
    match key {
        Value::Integer(n) => Some(*n),
        Value::String(s) => String::from_utf16(s).ok()?.parse().ok(),
        _ => None,
    }
}
impl Vm {
    pub(crate) fn delete_member(&mut self, receiver: &Value, key: &Value) -> Result<Value> {
        let id = self.object_handle(receiver)?;
        if !self.objects[id].valid || matches!(self.objects[id].kind, ObjectKind::ReadOnly(_)) {
            return Ok(Value::Integer(0));
        }
        self.objects[id].member_flags.remove(&units(key)?);
        self.objects[id].member_layout.remove(&units(key)?);
        let removed = if id == 0 {
            self.globals.remove(&key.unary("string")?.text()).is_some()
        } else if let ObjectKind::Array(items) = &mut self.objects[id].kind
            && let Some(index) = index(key)
        {
            if index >= 0 && (index as usize) < items.len() {
                items.remove(index as usize);
                true
            } else {
                false
            }
        } else {
            self.objects[id].members.remove(&units(key)?).is_some()
        };
        Ok(Value::Integer(i64::from(removed)))
    }

    /// Check whether a member exists without invoking its getter.
    pub fn has_member(&self, receiver: &Value, key: &Value) -> Result<bool> {
        if matches!(receiver, Value::String(_)) {
            return Ok(false);
        }
        let id = self.object_id(receiver)?;
        if id == 0 {
            return Ok(self.globals.contains_key(&key.unary("string")?.text()));
        }
        if self.objects[id].members.contains_key(&units(key)?) {
            return Ok(true);
        }
        match &self.objects[id].kind {
            ObjectKind::ReadOnly(view) => view.has(key),
            ObjectKind::Class { .. } => Ok(self.class_member(id, &units(key)?, 0)?.is_some()),
            ObjectKind::Super { bases, .. } => {
                for base in bases {
                    if self.class_member(*base, &units(key)?, 0)?.is_some() {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            ObjectKind::Array(items) => {
                Ok(index(key).is_some_and(|i| i >= 0 && (i as usize) < items.len()))
            }
            _ => Ok(false),
        }
    }
    pub fn get_member(&mut self, receiver: &Value, key: &Value, optional: bool) -> Result<Value> {
        let name = key.unary("string")?.text();
        if let Value::Octet(bytes) = receiver {
            if name == "length" {
                return Ok(Value::Integer(bytes.len() as i64));
            }
            let index = match key {
                Value::Integer(_) | Value::Real(_) => Some(key.integer()? as i32),
                Value::String(_) if name.starts_with(|c: char| c.is_ascii_digit()) => Some(
                    name.chars()
                        .take_while(char::is_ascii_digit)
                        .collect::<String>()
                        .parse::<i64>()? as i32,
                ),
                _ => None,
            };
            if let Some(index) = index {
                ensure!(
                    index >= 0 && (index as usize) < bytes.len(),
                    "octet index out of range"
                );
                return Ok(Value::Integer(bytes[index as usize].into()));
            }
            if optional {
                return Ok(Value::Void);
            }
            bail!("octet member not found: {name}");
        }
        if let Value::String(s) = receiver {
            if name == "length" {
                return Ok(Value::Integer(s.len() as i64));
            }
            if let Some(i) = index(key) {
                ensure!(i >= 0 && i as usize <= s.len(), "string index out of range");
                return Ok(Value::String(
                    s.get(i as usize).copied().into_iter().collect(),
                ));
            }
            if [
                "replace",
                "match",
                "reverse",
                "trim",
                "split",
                "substring",
                "substr",
                "indexOf",
                "charAt",
                "charCodeAt",
                "toLowerCase",
                "toUpperCase",
            ]
            .contains(&name.as_str())
            {
                return self.allocate(ObjectKind::Method {
                    receiver: receiver.clone(),
                    name,
                });
            }
            if optional {
                return Ok(Value::Void);
            }
            bail!("string member not found: {name}");
        }
        let id = self.object_id(receiver)?;
        if id == 0 {
            return self
                .globals
                .get(&name)
                .cloned()
                .or_else(|| optional.then_some(Value::Void))
                .with_context(|| format!("member not found: {name}"));
        }
        if let ObjectKind::ReadOnly(view) = &self.objects[id].kind {
            return self.readonly_member(view.clone(), key, optional);
        }
        if self.objects[id].hash_generation != self.hash_generation {
            let count = self.objects[id].members.len();
            self.objects[id].member_layout.rehash(count);
            self.objects[id].hash_generation = self.hash_generation;
        }
        self.objects[id].member_layout.touch(&units(key)?);
        let object = &self.objects[id];
        if let Some(value) = object.members.get(&units(key)?) {
            return Ok(value.clone());
        }
        if let ObjectKind::Class { .. } = &object.kind
            && let Some(value) = self.class_member(id, &units(key)?, 0)?
        {
            return Ok(value);
        }
        if let ObjectKind::Super { bases, context } = &object.kind {
            for base in bases.iter().rev() {
                if let Some(mut value) = self.class_member(*base, &units(key)?, 0)? {
                    if let Value::Object(reference) = &mut value {
                        reference.context = Some(*context);
                    }
                    return Ok(value);
                }
            }
        }
        if matches!(object.kind, ObjectKind::RegExp { .. })
            && ["replace", "split", "match", "test", "exec"].contains(&name.as_str())
        {
            return self.allocate(ObjectKind::Method {
                receiver: receiver.clone(),
                name,
            });
        }
        if let ObjectKind::Array(items) = &object.kind {
            if name == "count" || name == "length" {
                return Ok(Value::Integer(items.len() as i64));
            }
            if let Some(i) = index(key) {
                ensure!(i >= 0, "negative array index");
                return Ok(items.get(i as usize).cloned().unwrap_or(Value::Void));
            }
            if [
                "add", "push", "pop", "shift", "unshift", "erase", "insert", "clear", "reverse",
                "join", "find",
            ]
            .contains(&name.as_str())
            {
                return self.allocate(ObjectKind::Method {
                    receiver: receiver.clone(),
                    name,
                });
            }
        }
        if matches!(object.kind, ObjectKind::Dictionary) || optional {
            return Ok(Value::Void);
        }
        bail!("member not found: {name}")
    }
    pub fn set_member(&mut self, receiver: &Value, key: &Value, value: Value) -> Result<()> {
        let id = self.object_id(receiver)?;
        ensure!(
            !matches!(self.objects[id].kind, ObjectKind::ReadOnly(_)),
            "native data is read-only"
        );
        self.objects[id].member_flags.remove(&units(key)?);
        if id == 0 {
            self.globals.insert(key.unary("string")?.text(), value);
            return Ok(());
        }
        if let ObjectKind::Array(items) = &mut self.objects[id].kind {
            let name = key.unary("string")?.text();
            if name == "count" || name == "length" {
                let len = value.integer()?;
                ensure!((0..=1_000_000).contains(&len), "array length exceeds limit");
                items.resize(len as usize, Value::Void);
                return Ok(());
            }
            if let Some(index) = index(key) {
                ensure!((0..1_000_000).contains(&index), "array index exceeds limit");
                let index = index as usize;
                if items.len() <= index {
                    items.resize(index + 1, Value::Void);
                }
                items[index] = value;
                return Ok(());
            }
        }
        ensure!(
            self.objects[id].members.len() < 100_000,
            "object member limit exceeded"
        );
        self.objects[id].member_layout.insert(&units(key)?);
        self.objects[id].members.insert(units(key)?, value);
        Ok(())
    }
    pub(crate) fn method(
        &mut self,
        receiver: &Value,
        name: &str,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("{name}: missing argument {i}"))
        };
        if let Value::Object(_) = receiver
            && matches!(
                self.objects[self.object_id(receiver)?].kind,
                ObjectKind::RegExp { .. }
            )
        {
            return self.regexp_method(receiver, name, args, host, budget);
        }
        if let Value::String(s) = receiver {
            if ["replace", "match", "split"].contains(&name)
                && matches!(args.first(), Some(Value::Object(_)))
            {
                let mut forwarded = vec![receiver.clone()];
                forwarded.extend_from_slice(&args[1..]);
                return self.regexp_method(&args[0], name, &forwarded, host, budget);
            }
            return match name {
                "trim" => {
                    ensure!(args.is_empty(), "trim expects no arguments");
                    // Kirikiri trims only nonzero UTF-16 units through U+0020.
                    let mut start = 0;
                    let mut end = s.len();
                    while end > start && (1..=32).contains(&s[end - 1]) { end -= 1; }
                    while start < end && (1..=32).contains(&s[start]) { start += 1; }
                    Ok(Value::String(s[start..end].to_vec()))
                }
                "reverse" => Ok(Value::String(s.iter().copied().rev().collect())),
                "toLowerCase" | "toUpperCase" => Ok(Value::String(
                    s.iter()
                        .map(|c| {
                            if name == "toLowerCase" && (65..=90).contains(c) {
                                c + 32
                            } else if name == "toUpperCase" && (97..=122).contains(c) {
                                c - 32
                            } else {
                                *c
                            }
                        })
                        .collect(),
                )),
                "charAt" | "charCodeAt" => {
                    let i = arg(0)?.integer()?;
                    let unit = if i >= 0 {
                        s.get(i as usize).copied()
                    } else {
                        None
                    };
                    Ok(if name == "charCodeAt" {
                        Value::Integer(unit.unwrap_or(0) as i64)
                    } else {
                        Value::String(unit.into_iter().collect())
                    })
                }
                "substring" | "substr" => {
                    let start = arg(0)?.integer()?.clamp(0, s.len() as i64) as usize;
                    let end = if let Some(value) = args.get(1).filter(|v| !matches!(v, Value::Void))
                    {
                        let n = value.integer()?.max(0) as usize;
                        if name == "substr" {
                            start.saturating_add(n).min(s.len())
                        } else {
                            n.min(s.len())
                        }
                    } else {
                        s.len()
                    };
                    Ok(Value::String(s[start.min(end)..start.max(end)].to_vec()))
                }
                "indexOf" => {
                    let needle = units(arg(0)?)?;
                    let start = args
                        .get(1)
                        .unwrap_or(&Value::Integer(0))
                        .integer()?
                        .clamp(0, s.len() as i64) as usize;
                    let found = if needle.is_empty() {
                        Some(start)
                    } else {
                        s[start..]
                            .windows(needle.len())
                            .position(|w| w == needle)
                            .map(|i| start + i)
                    };
                    Ok(Value::Integer(found.map(|n| n as i64).unwrap_or(-1)))
                }
                "split" => {
                    let separators = units(arg(0)?)?;
                    let purge = args.get(2).is_some_and(|v| v.truth().unwrap_or(false));
                    // TJS accepts a set of delimiter characters, not a substring separator.
                    let parts = if separators.is_empty() {
                        s.iter().map(|c| Value::String(vec![*c])).collect()
                    } else {
                        s.split(|c| separators.contains(c))
                            .filter(|part| !purge || !part.is_empty())
                            .map(|part| Value::String(part.to_vec()))
                            .collect()
                    };
                    self.allocate(ObjectKind::Array(parts))
                }
                _ => Err(unsupported(format!("unsupported string method: {name}"))),
            };
        }
        let id = self.object_id(receiver)?;
        let ObjectKind::Array(items) = &mut self.objects[id].kind else {
            bail!("array method on non-array object")
        };
        match name {
            "add" => {
                ensure!(items.len() < 1_000_000, "array exceeds limit");
                let i = items.len();
                items.push(arg(0)?.clone());
                Ok(Value::Integer(i as i64))
            }
            "push" | "unshift" => {
                ensure!(items.len() + args.len() <= 1_000_000, "array exceeds limit");
                if name == "push" {
                    items.extend_from_slice(args);
                } else {
                    items.splice(0..0, args.iter().cloned());
                }
                Ok(Value::Integer(items.len() as i64))
            }
            "pop" => Ok(items.pop().unwrap_or(Value::Void)),
            "shift" => Ok(if items.is_empty() {
                Value::Void
            } else {
                items.remove(0)
            }),
            "reverse" => {
                items.reverse();
                Ok(Value::Void)
            }
            "clear" => {
                items.clear();
                Ok(Value::Void)
            }
            "erase" | "insert" => {
                let i = arg(0)?.integer()?;
                ensure!(
                    i >= 0 && i as usize <= items.len(),
                    "array index out of range"
                );
                if name == "erase" {
                    ensure!((i as usize) < items.len(), "array index out of range");
                    items.remove(i as usize);
                } else {
                    ensure!(items.len() < 1_000_000, "array exceeds limit");
                    items.insert(i as usize, arg(1)?.clone());
                }
                Ok(Value::Void)
            }
            "find" => Ok(Value::Integer(
                items
                    .iter()
                    .position(|v| v.strict_equal(args.first().unwrap_or(&Value::Void)))
                    .map(|i| i as i64)
                    .unwrap_or(-1),
            )),
            "join" => {
                let separator = units(arg(0)?)?;
                let purge = args.get(2).is_some_and(|v| v.truth().unwrap_or(false));
                let mut result = vec![];
                let mut first = true;
                for value in items {
                    if purge && matches!(value, Value::Void) {
                        continue;
                    }
                    if !first {
                        result.extend_from_slice(&separator);
                    }
                    first = false;
                    result.extend(units(value)?);
                    ensure!(result.len() <= 32_000_000, "joined string exceeds limit");
                }
                Ok(Value::String(result))
            }
            _ => Err(unsupported(format!("unsupported array method: {name}"))),
        }
    }
}
