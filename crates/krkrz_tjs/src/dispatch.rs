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
    pub(crate) fn array_split(
        &mut self,
        receiver: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = self.object_id(receiver)?;
        let ObjectKind::Array(items) = &mut self.objects[id].kind else {
            bail!("Array.split requires an Array native instance");
        };
        ensure!(
            args.len() >= 2,
            "Array.split requires a delimiter and a string"
        );
        items.clear();
        let subject = units(&args[1])?;
        let purge = args.get(3).map(Value::truth).transpose()?.unwrap_or(false);
        let parts = if let Value::Object(reference) = args[0]
            && let Some(regex) = reference.object
            && matches!(
                self.objects.get(regex).map(|o| &o.kind),
                Some(ObjectKind::RegExp { .. })
            ) {
            let result = self.regexp_method(
                &args[0],
                "split",
                &[
                    Value::String(subject),
                    Value::Void,
                    Value::Integer(purge.into()),
                ],
                host,
                budget,
            )?;
            let result = self.object_id(&result)?;
            let ObjectKind::Array(parts) = &mut self.objects[result].kind else {
                unreachable!();
            };
            std::mem::take(parts)
        } else {
            let separators = units(&args[0])?;
            split_characters(&subject, &separators, purge, budget)?
        };
        self.objects[id].kind = ObjectKind::Array(parts);
        Ok(Value::Object(crate::ObjectRef {
            object: Some(id),
            context: Some(id),
        }))
    }
    pub fn dictionary_assign(
        &mut self,
        receiver: &Value,
        args: &[Value],
        clearing: bool,
        budget: &mut u64,
    ) -> Result<Value> {
        let target = self.object_id(receiver)?;
        ensure!(
            matches!(self.objects[target].kind, ObjectKind::Dictionary),
            "Dictionary native instance required"
        );
        let clear = clearing
            || args
                .get(1)
                .filter(|v| !matches!(v, Value::Void))
                .map(Value::integer)
                .transpose()?
                .map(|v| v as i32 != 0)
                .unwrap_or(true);
        let source = if clearing {
            None
        } else {
            let Value::Object(reference) = args
                .first()
                .context("Dictionary.assign: missing argument 0")?
            else {
                bail!("Dictionary.assign requires an Object source");
            };
            Some(
                self.object_id(&Value::object(
                    reference
                        .context
                        .or(reference.object)
                        .context("null Dictionary.assign source")?,
                ))?,
            )
        };
        if clear {
            self.objects[target].members.clear();
            self.objects[target].member_flags.clear();
            self.objects[target].member_layout = Default::default();
        }
        let Some(source) = source else {
            return Ok(Value::Void);
        };
        let mut entries = Vec::new();
        let reserve = match self.objects[source].kind.clone() {
            ObjectKind::Array(items) => {
                let count = items.len();
                for pair in items.as_chunks::<2>().0 {
                    entries.push((pair[0].unary("string")?, pair[1].clone(), 0));
                }
                count
            }
            ObjectKind::Dictionary | ObjectKind::Instance | ObjectKind::Namespace => {
                if self.objects[source].hash_generation != self.hash_generation {
                    let count = self.objects[source].members.len();
                    self.objects[source].member_layout.rehash(count);
                    self.objects[source].hash_generation = self.hash_generation;
                }
                for key in self.objects[source].member_layout.keys() {
                    let flags = self.objects[source]
                        .member_flags
                        .get(&key)
                        .copied()
                        .unwrap_or(0);
                    if flags & crate::scripts_ex::HIDDEN == 0 {
                        entries.push((
                            Value::String(key.clone()),
                            self.objects[source].members[&key].clone(),
                            flags,
                        ));
                    }
                }
                entries.len()
            }
            _ => {
                return Err(unsupported(
                    "Dictionary.assign source enumeration is not implemented for this object type",
                ));
            }
        };
        *budget = budget
            .checked_sub(entries.len() as u64)
            .ok_or_else(|| unsupported("Dictionary.assign execution budget exceeded"))?;
        let count = self.objects[target].members.len() + reserve;
        ensure!(count <= 100_000, "Dictionary.assign member limit exceeded");
        self.objects[target].member_layout.rehash(count);
        for (key, value, flags) in entries {
            self.set_member(receiver, &key, value)?;
            self.objects[target]
                .member_flags
                .insert(units(&key)?, flags);
        }
        Ok(Value::Void)
    }

    pub(crate) fn array_assign(
        &mut self,
        receiver: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let target = self.object_id(receiver)?;
        let source = args.first().context("Array.assign: missing argument 0")?;
        let ObjectKind::Array(items) = &mut self.objects[target].kind else {
            bail!("Array native instance required");
        };
        // Native assign clears before converting the source, including self.
        items.clear();
        let source = if let Value::Object(reference) = source {
            Value::object(
                reference
                    .context
                    .or(reference.object)
                    .context("null Array.assign source")?,
            )
        } else {
            bail!("Array.assign requires an Object source");
        };
        let source_id = self.object_id(&source)?;
        let values = match self.objects[source_id].kind.clone() {
            ObjectKind::Array(items) => items,
            ObjectKind::Dictionary | ObjectKind::Instance | ObjectKind::Namespace => {
                if self.objects[source_id].hash_generation != self.hash_generation {
                    let count = self.objects[source_id].members.len();
                    self.objects[source_id].member_layout.rehash(count);
                    self.objects[source_id].hash_generation = self.hash_generation;
                }
                let mut values = Vec::new();
                for key in self.objects[source_id].member_layout.keys() {
                    if self.objects[source_id]
                        .member_flags
                        .get(&key)
                        .copied()
                        .unwrap_or(0)
                        & crate::scripts_ex::HIDDEN
                        == 0
                    {
                        values.push(Value::String(key.clone()));
                        values.push(self.objects[source_id].members[&key].clone());
                    }
                }
                values
            }
            _ => {
                return Err(unsupported(
                    "Array.assign source enumeration is not implemented for this object type",
                ));
            }
        };
        *budget = budget
            .checked_sub(values.len() as u64)
            .ok_or_else(|| unsupported("Array.assign execution budget exceeded"))?;
        ensure!(
            values.len() <= 1_000_000,
            "Array.assign exceeds array limit"
        );
        self.objects[target].kind = ObjectKind::Array(values);
        Ok(Value::Void)
    }

    pub fn delete_member(&mut self, receiver: &Value, key: &Value) -> Result<Value> {
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
                "repeat",
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
                "add", "push", "pop", "shift", "unshift", "erase", "remove", "insert", "clear",
                "reverse", "join", "find",
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
        result_needed: bool,
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
                    if !result_needed {
                        return Ok(Value::Void);
                    }
                    // Kirikiri trims only nonzero UTF-16 units through U+0020.
                    let mut start = 0;
                    let mut end = s.len();
                    while end > start && (1..=32).contains(&s[end - 1]) {
                        end -= 1;
                    }
                    while start < end && (1..=32).contains(&s[start]) {
                        start += 1;
                    }
                    Ok(Value::String(s[start..end].to_vec()))
                }
                "repeat" => {
                    ensure!(args.len() == 1, "repeat expects one argument");
                    if !result_needed {
                        return Ok(Value::Void);
                    }
                    let count = args[0].integer()? as i32;
                    if count <= 0 || s.is_empty() {
                        return Ok(Value::string(""));
                    }
                    let length = s
                        .len()
                        .checked_mul(count as usize)
                        .filter(|n| *n <= 32_000_000)
                        .ok_or_else(|| unsupported("repeated string exceeds limit"))?;
                    *budget = budget
                        .checked_sub(length as u64)
                        .ok_or_else(|| unsupported("String.repeat execution budget exceeded"))?;
                    Ok(Value::String(s.repeat(count as usize)))
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
                    let purge = args.get(2).map(Value::truth).transpose()?.unwrap_or(false);
                    // TJS accepts a set of delimiter characters, not a substring separator.
                    let parts = split_characters(s, &separators, purge, budget)?;
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
            "remove" => {
                let value = arg(0)?;
                let all = args.get(1).map(Value::truth).transpose()?.unwrap_or(true);
                *budget = budget
                    .checked_sub(items.len() as u64)
                    .ok_or_else(|| unsupported("Array.remove execution budget exceeded"))?;
                let mut removed = 0;
                items.retain(|item| {
                    if (all || removed == 0) && value.strict_equal(item) {
                        removed += 1;
                        false
                    } else {
                        true
                    }
                });
                Ok(Value::Integer(removed))
            }
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

fn split_characters(
    subject: &[u16],
    separators: &[u16],
    purge: bool,
    budget: &mut u64,
) -> Result<Vec<Value>> {
    let subject = &subject[..subject
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(subject.len())];
    let separators = &separators[..separators
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(separators.len())];
    *budget = budget
        .checked_sub((subject.len() + separators.len()) as u64)
        .ok_or_else(|| unsupported("Array.split execution budget exceeded"))?;
    // A bit set avoids quadratic searches through a long delimiter string and
    // preserves individual UTF-16 units, including unmatched surrogates.
    let mut delimiters = [0u64; 1024];
    for unit in separators {
        delimiters[*unit as usize / 64] |= 1u64 << (*unit as usize % 64);
    }
    let mut parts = Vec::new();
    for part in
        subject.split(|unit| delimiters[*unit as usize / 64] & (1u64 << (*unit as usize % 64)) != 0)
    {
        if !purge || !part.is_empty() {
            ensure!(parts.len() < 1_000_000, "Array.split exceeds array limit");
            parts.push(Value::String(part.to_vec()));
        }
    }
    Ok(parts)
}
