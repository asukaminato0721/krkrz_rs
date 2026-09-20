//! ScriptsEx object operations, following krkr2's scriptsEx/Main.cpp.
//! Registration is explicit: these are plugin extensions, not TJS built-ins.
use crate::{Host, Value, Vm, object::ObjectKind, unsupported};
use anyhow::{Context, Result, bail};

pub(crate) const ENSURE: u32 = 0x200;
const MUST_EXIST: u32 = 0x400;
const IGNORE_PROPERTY: u32 = 0x800;
const HIDDEN: u32 = 0x1000;
pub(crate) const STATIC: u32 = 0x10000;

fn charge(budget: &mut u64, depth: usize) -> Result<()> {
    if *budget == 0 || depth >= 128 {
        return Err(unsupported(
            "ScriptsEx execution budget or recursion depth exceeded",
        ));
    }
    *budget -= 1;
    Ok(())
}

impl Vm {
    pub fn register_scripts_ex(&mut self) -> Result<()> {
        let scripts = self.register_namespace("Scripts")?;
        for name in [
            "clone",
            "equalStruct",
            "equalStructNumericLoose",
            "foreach",
            "getObjectContext",
            "isNullContext",
            "getObjectCount",
            "getObjectKeys",
            "propGet",
            "propSet",
            "getMD5HashString",
            "rehash",
        ] {
            let function = self.allocate(ObjectKind::Native(format!("ScriptsEx.{name}")))?;
            let id = self.object_id(&function)?;
            self.objects[id].native_static = true;
            self.set_member(&scripts, &Value::string(name), function)?;
        }
        for (name, value) in [
            ("pfMemberEnsure", ENSURE),
            ("pfMemberMustExist", MUST_EXIST),
            ("pfIgnoreProp", IGNORE_PROPERTY),
            ("pfHiddenMember", HIDDEN),
            ("pfStaticMember", STATIC),
        ] {
            self.set_member(&scripts, &Value::string(name), Value::Integer(value.into()))?;
        }
        Ok(())
    }

    fn reflection_keys(&self, value: &Value, visible_only: bool) -> Result<Vec<Vec<u16>>> {
        let id = self.object_id(value)?;
        if let ObjectKind::ReadOnly(view) = &self.objects[id].kind {
            return view
                .keys()?
                .into_iter()
                .map(|key| match key.unary("string")? {
                    Value::String(units) => Ok(units),
                    _ => unreachable!(),
                })
                .collect();
        }
        if matches!(self.objects[id].kind, ObjectKind::Array(_)) {
            return Err(unsupported(
                "ScriptsEx Array member reflection requires the complete native Array member table",
            ));
        }
        let keys: Vec<_> = if id == 0 {
            self.globals
                .keys()
                .map(|key| key.encode_utf16().collect())
                .collect()
        } else {
            self.objects[id].members.keys().cloned().collect()
        };
        Ok(keys
            .into_iter()
            .filter(|key| {
                !visible_only
                    || self.objects[id].member_flags.get(key).copied().unwrap_or(0) & HIDDEN == 0
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scripts_ex(
        &mut self,
        name: &str,
        context: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
        result_needed: bool,
    ) -> Result<Value> {
        charge(budget, 0)?;
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("Scripts.{name}: missing argument {i}"))
        };
        if !result_needed && matches!(name, "getObjectKeys" | "getObjectCount") {
            arg(0)?;
            return Ok(Value::Void);
        }
        match name {
            "rehash" => {
                self.hash_generation = self.hash_generation.wrapping_add(1);
                Ok(Value::Void)
            }
            "getObjectContext" | "isNullContext" => {
                let Value::Object(reference) = arg(0)? else {
                    bail!("object closure required")
                };
                if name == "isNullContext" {
                    return Ok(Value::Integer(i64::from(reference.context.is_none())));
                }
                Ok(Value::Object(crate::ObjectRef {
                    object: reference.context,
                    context: reference.context,
                }))
            }
            "getObjectKeys" => {
                let keys = self.reflection_keys(arg(0)?, true)?;
                self.allocate(ObjectKind::Array(
                    keys.into_iter().map(Value::String).collect(),
                ))
            }
            "getObjectCount" => Ok(Value::Integer(
                self.reflection_keys(arg(0)?, false)?.len() as i64
            )),
            "propGet" | "propSet" => {
                let receiver = arg(0)?;
                let key = arg(1)?;
                self.object_id(receiver)?;
                let writing = name == "propSet";
                let flags = args
                    .get(if writing { 3 } else { 2 })
                    .map(Value::integer)
                    .transpose()?
                    .map(|v| v as u32)
                    .unwrap_or(if writing { ENSURE } else { MUST_EXIST });
                if flags & !(ENSURE | MUST_EXIST | IGNORE_PROPERTY | HIDDEN | STATIC) != 0 {
                    return Err(unsupported("unsupported ScriptsEx property flags"));
                }
                let present = self.has_member(receiver, key)?;
                if !present {
                    if flags & ENSURE != 0 {
                        self.set_member(receiver, key, Value::Void)?;
                    } else if writing || flags & MUST_EXIST != 0 {
                        bail!("member not found: {}", key.text());
                    }
                }
                if writing {
                    let value = arg(2)?.clone();
                    self.set_property_flags(
                        receiver,
                        key,
                        value,
                        flags & IGNORE_PROPERTY != 0,
                        flags & (HIDDEN | STATIC),
                        host,
                        budget,
                    )?;
                    Ok(Value::Void)
                } else {
                    let raw = self.get_member(receiver, key, false)?;
                    let accessor = flags & IGNORE_PROPERTY == 0
                        && self.object_id(&raw).ok().is_some_and(|id| {
                            matches!(self.objects[id].kind, ObjectKind::Property { .. })
                        });
                    let value = self.get_property(
                        receiver,
                        key,
                        false,
                        flags & IGNORE_PROPERTY != 0,
                        host,
                        budget,
                    )?;
                    let missing_optional = !present && flags & ENSURE == 0;
                    if !result_needed && !missing_optional && !accessor {
                        bail!("Invalid argument: discarded Scripts.propGet result");
                    }
                    Ok(value)
                }
            }
            "equalStruct" | "equalStructNumericLoose" => {
                Ok(Value::Integer(i64::from(self.equal_structure(
                    arg(0)?,
                    arg(1)?,
                    name == "equalStructNumericLoose",
                    host,
                    budget,
                    0,
                )?)))
            }
            "clone" => self.clone_structure(arg(0)?, host, budget, 0),
            "foreach" => {
                let receiver = arg(0)?;
                let callback = arg(1)?;
                let id = self.object_id(receiver)?;
                let keys: Vec<Value> = match &self.objects[id].kind {
                    ObjectKind::ReadOnly(view) => view.keys()?,
                    ObjectKind::Array(items) => (0..items.len())
                        .map(|index| Value::Integer(index as i64))
                        .collect(),
                    ObjectKind::Dictionary | ObjectKind::Instance => self.objects[id]
                        .member_layout
                        .keys()
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                    _ => {
                        return Err(unsupported(
                            "Scripts.foreach currently requires Array, Dictionary, or a script instance",
                        ));
                    }
                };
                for key in keys {
                    charge(budget, 0)?;
                    if let Value::String(units) = &key
                        && self.objects[id]
                            .member_flags
                            .get(units)
                            .copied()
                            .unwrap_or(0)
                            == HIDDEN
                    {
                        continue;
                    }
                    let value = if matches!(self.objects[id].kind, ObjectKind::ReadOnly(_)) {
                        self.get_member(receiver, &key, false)?
                    } else if let Value::String(units) = &key {
                        self.objects[id]
                            .members
                            .get(units)
                            .cloned()
                            .unwrap_or(Value::Void)
                    } else {
                        self.get_member(receiver, &key, false)?
                    };
                    let mut call_args = vec![key, value];
                    call_args.extend_from_slice(&args[2..]);
                    let result = self.invoke(callback, context, &call_args, host, budget)?;
                    if !matches!(result, Value::Void) {
                        return Ok(result);
                    }
                }
                Ok(Value::Void)
            }
            _ => Err(unsupported(format!("Scripts.{name} is not implemented"))),
        }
    }

    fn equal_structure(
        &mut self,
        left: &Value,
        right: &Value,
        numeric_loose: bool,
        host: &mut impl Host,
        budget: &mut u64,
        depth: usize,
    ) -> Result<bool> {
        charge(budget, depth)?;
        if let (Value::Object(a), Value::Object(b)) = (left, right) {
            // Upstream compares dispatch identity before considering bound contexts.
            if a.object == b.object {
                return Ok(true);
            }
            if a.object.is_none() || b.object.is_none() {
                return Ok(false);
            }
            let aid = self.object_id(left)?;
            let bid = self.object_id(right)?;
            if matches!(self.objects[aid].kind, ObjectKind::ReadOnly(_))
                || matches!(self.objects[bid].kind, ObjectKind::ReadOnly(_))
            {
                let left = self.clone_structure(left, host, budget, depth + 1)?;
                let right = self.clone_structure(right, host, budget, depth + 1)?;
                return self.equal_structure(&left, &right, numeric_loose, host, budget, depth + 1);
            }
            match (
                self.objects[aid].kind.clone(),
                self.objects[bid].kind.clone(),
            ) {
                (ObjectKind::Array(a), ObjectKind::Array(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    for (a, b) in a.iter().zip(&b) {
                        if !self.equal_structure(a, b, numeric_loose, host, budget, depth + 1)? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                (ObjectKind::Dictionary, ObjectKind::Dictionary) => {
                    let keys = self.objects[aid].member_layout.keys();
                    if numeric_loose {
                        if keys.len() != self.reflection_keys(right, false)?.len() {
                            return Ok(false);
                        }
                    } else if self.reflection_keys(left, true)?
                        != self.reflection_keys(right, true)?
                    {
                        return Ok(false);
                    }
                    for key in keys {
                        // ScriptsEx checks equality with HIDDEN, rather than a bit mask.
                        if self.objects[aid]
                            .member_flags
                            .get(&key)
                            .copied()
                            .unwrap_or(0)
                            == HIDDEN
                        {
                            continue;
                        }
                        let a = self.objects[aid].members[&key].clone();
                        let key = Value::String(key);
                        if !numeric_loose && !self.has_member(right, &key)? {
                            return Ok(false);
                        }
                        let b = self.get_property(right, &key, false, false, host, budget)?;
                        if !self.equal_structure(&a, &b, numeric_loose, host, budget, depth + 1)? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                _ => (),
            }
        }
        if numeric_loose
            && matches!(left, Value::Integer(_) | Value::Real(_))
            && matches!(right, Value::Integer(_) | Value::Real(_))
        {
            Ok(left.equal(right))
        } else {
            Ok(left.strict_equal(right))
        }
    }

    fn clone_structure(
        &mut self,
        source: &Value,
        host: &mut impl Host,
        budget: &mut u64,
        depth: usize,
    ) -> Result<Value> {
        charge(budget, depth)?;
        if let Value::Object(reference) = source
            && reference.object.is_some()
        {
            let id = self.object_id(source)?;
            match self.objects[id].kind.clone() {
                ObjectKind::ReadOnly(view) => {
                    let result = if matches!(view.data.as_ref(), crate::ReadOnlyData::Array(_)) {
                        self.new_array(vec![])?
                    } else {
                        self.new_dictionary()?
                    };
                    for key in view.keys()? {
                        let value = self.get_member(source, &key, false)?;
                        let value = self.clone_structure(&value, host, budget, depth + 1)?;
                        self.set_member(&result, &key, value)?;
                    }
                    return Ok(result);
                }
                ObjectKind::Array(items) => {
                    let mut result = Vec::with_capacity(items.len());
                    for item in items {
                        result.push(self.clone_structure(&item, host, budget, depth + 1)?);
                    }
                    return self.allocate(ObjectKind::Array(result));
                }
                ObjectKind::Dictionary => {
                    let result = self.new_dictionary()?;
                    let result_id = self.object_id(&result)?;
                    for key in self.objects[id].member_layout.keys() {
                        let value = self.objects[id].members[&key].clone();
                        let value = self.clone_structure(&value, host, budget, depth + 1)?;
                        self.set_member(&result, &Value::String(key.clone()), value)?;
                        let flags = self.objects[id]
                            .member_flags
                            .get(&key)
                            .copied()
                            .unwrap_or(0);
                        self.objects[result_id].member_flags.insert(key, flags);
                    }
                    return Ok(result);
                }
                _ => {
                    let method = self.get_member(source, &Value::string("clone"), true)?;
                    if !matches!(method, Value::Void) {
                        // Main.cpp tests for TJS_S_TRUE; a script function returns
                        // TJS_S_OK. The installed PackinOne confirms the result is ignored.
                        self.invoke(&method, source, &[], host, budget)?;
                    }
                }
            }
        }
        Ok(source.clone())
    }
}
