use crate::object::ObjectKind;
use crate::vm::Frame;
use crate::{Class, Host, Property, Value, Vm};
use anyhow::{Context, Result, bail, ensure};
use std::sync::Arc;

impl Vm {
    pub(crate) fn instance_of(&self, value: &Value, name: &str) -> Result<Value> {
        let result = if matches!(value, Value::Void) {
            false
        } else if name == "Object" {
            true
        } else {
            match value {
                Value::Integer(_) | Value::Real(_) => name == "Number",
                Value::String(_) => name == "String",
                Value::Octet(_) => name == "Octet",
                Value::Object(r) if r.object.is_some() => {
                    let object = &self.objects[self.object_id(value)?];
                    object.classes.iter().any(|n| n == name)
                        || match &object.kind {
                            ObjectKind::Array(_) => name == "Array",
                            ObjectKind::Dictionary => name == "Dictionary",
                            ObjectKind::RegExp { .. } => name == "RegExp",
                            ObjectKind::Class { .. } => name == "Class",
                            ObjectKind::Property { .. } => name == "Property",
                            ObjectKind::Function(_)
                            | ObjectKind::Native(_)
                            | ObjectKind::Method { .. } => name == "Function",
                            _ => false,
                        }
                }
                _ => false,
            }
        };
        Ok(Value::Integer(i64::from(result)))
    }
    pub(crate) fn define_property(
        &mut self,
        definition: &Property,
        owner: Option<usize>,
    ) -> Result<Value> {
        let mut make = |function: &Option<crate::Function>| -> Result<Option<Value>> {
            if let Some(function) = function {
                let value = self.allocate(ObjectKind::Function(Arc::new(function.clone())))?;
                let id = self.object_id(&value)?;
                self.objects[id].owner = owner;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        };
        let getter = make(&definition.getter)?;
        let setter = make(&definition.setter)?;
        self.allocate(ObjectKind::Property { getter, setter })
    }
    pub(crate) fn define_class(&mut self, definition: &Class) -> Result<Value> {
        let value = self.allocate(ObjectKind::Class {
            definition: Arc::new(definition.clone()),
            native_initializer: None,
        })?;
        let id = self.object_id(&value)?;
        for function in &definition.methods {
            let method = self.allocate(ObjectKind::Function(Arc::new(function.clone())))?;
            let mid = self.object_id(&method)?;
            self.objects[mid].owner = Some(id);
            self.set_member(&value, &Value::string(&function.name), method)?;
        }
        for property in &definition.properties {
            let member = self.define_property(property, Some(id))?;
            self.set_member(&value, &Value::string(&property.name), member)?;
        }
        Ok(value)
    }
    pub(crate) fn class_member(
        &self,
        class: usize,
        key: &[u16],
        depth: usize,
    ) -> Result<Option<Value>> {
        ensure!(depth < 128, "class inheritance depth exceeded");
        self.object_id(&Value::object(class))?;
        if let Some(value) = self.objects[class].members.get(key) {
            return Ok(Some(value.clone()));
        }
        Ok(None)
    }
    fn resolve_class_member(
        &mut self,
        class: usize,
        key: &[u16],
        host: &mut impl Host,
        budget: &mut u64,
        depth: usize,
    ) -> Result<Option<Value>> {
        ensure!(depth < 128, "class inheritance depth exceeded");
        if let Some(value) = self.class_member(class, key, depth)? {
            return Ok(Some(value));
        }
        if let ObjectKind::Class { definition, .. } = &self.objects[class].kind {
            let bases = definition.bases.clone();
            for expression in bases.iter().rev() {
                let value = self.run(expression, host, budget, Frame::global())?;
                let base = self.object_id(&value)?;
                if let Some(value) =
                    self.resolve_class_member(base, key, host, budget, depth + 1)?
                {
                    return Ok(Some(value));
                }
            }
        }
        Ok(None)
    }
    fn resolve_member(
        &mut self,
        receiver: &Value,
        key: &Value,
        optional: bool,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        if let Value::Object(_) = receiver {
            let id = self.object_id(receiver)?;
            let key_units = key.unary("string")?;
            let Value::String(units) = &key_units else {
                unreachable!()
            };
            let kind = self.objects[id].kind.clone();
            let resolved = match kind {
                ObjectKind::Class { .. } => {
                    Some(self.resolve_class_member(id, units, host, budget, 0)?)
                }
                ObjectKind::Super { bases, context } => {
                    let mut found = None;
                    for base in bases.iter().rev() {
                        if let Some(mut value) =
                            self.resolve_class_member(*base, units, host, budget, 0)?
                        {
                            if let Value::Object(reference) = &mut value {
                                reference.context = Some(context);
                            }
                            found = Some(value);
                            break;
                        }
                    }
                    Some(found)
                }
                _ => None,
            };
            if let Some(value) = resolved {
                return match value {
                    Some(value) => Ok(value),
                    None if optional => Ok(Value::Void),
                    None => bail!("member not found: {}", key.text()),
                };
            }
        }
        self.get_member(receiver, key, optional)
    }
    fn initialize_instance(
        &mut self,
        class: usize,
        instance: &Value,
        host: &mut impl Host,
        budget: &mut u64,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth < 128, "class inheritance depth exceeded");
        self.object_id(&Value::object(class))?;
        let ObjectKind::Class {
            definition,
            native_initializer,
        } = self.objects[class].kind.clone()
        else {
            bail!("invalid class object")
        };
        for expression in &definition.bases {
            let mut frame = Frame::global();
            frame.context = self.object_id(instance)?;
            let base = self.run(expression, host, budget, frame)?;
            let base = self.object_id(&base)?;
            self.initialize_instance(base, instance, host, budget, depth + 1)?;
        }
        let context = self.object_id(instance)?;
        self.objects[context].classes.push(definition.name.clone());
        if let Some(initializer) = native_initializer {
            host.call_with_context(self, &initializer, instance, &[], budget)?;
            self.objects[context]
                .native_finalizers
                .push(format!("{}.@invalidate", definition.name));
        }
        for (key, mut value) in self.objects[class].members.clone() {
            let flags = self.objects[class]
                .member_flags
                .get(&key)
                .copied()
                .unwrap_or(0);
            if flags & crate::scripts_ex::STATIC != 0 {
                continue;
            }
            if let Value::Object(object) = &mut value {
                object.context = Some(context);
            }
            self.set_member(instance, &Value::String(key.clone()), value)?;
            self.objects[context].member_flags.insert(key, flags);
        }
        if let Some(initializer) = &definition.initializer {
            let mut frame = Frame::global();
            frame.context = context;
            self.run(initializer, host, budget, frame)?;
        }
        Ok(())
    }
    pub(crate) fn construct(
        &mut self,
        callee: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let class = self.object_id(callee)?;
        match self.objects[class].kind.clone() {
            ObjectKind::Class { definition, .. } => {
                let instance = self.allocate(ObjectKind::Instance)?;
                self.initialize_instance(class, &instance, host, budget, 0)?;
                let key: Vec<_> = definition.name.encode_utf16().collect();
                if let Some(constructor) =
                    self.resolve_class_member(class, &key, host, budget, 0)?
                {
                    self.invoke(&constructor, &instance, args, host, budget)?;
                }
                Ok(instance)
            }
            ObjectKind::Native(name)
                if ["Array", "Dictionary", "Exception", "RegExp"].contains(&name.as_str()) =>
            {
                self.invoke(callee, &Value::object(0), args, host, budget)
            }
            _ => bail!("TJS object is not a constructor"),
        }
    }
    pub(crate) fn get_property(
        &mut self,
        receiver: &Value,
        key: &Value,
        optional: bool,
        raw: bool,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let mut value = self.resolve_member(receiver, key, optional, host, budget)?;
        if let Value::Object(object) = &mut value
            && let Some(id) = object.object
            && self
                .objects
                .get(id)
                .is_some_and(|o| matches!(o.kind, ObjectKind::Property { .. }))
        {
            if object.context.is_none() {
                object.context = Some(self.object_id(receiver)?);
            }
            if !raw {
                return self.read_property(&value, receiver, host, budget);
            }
        }
        Ok(value)
    }
    pub(crate) fn set_property(
        &mut self,
        receiver: &Value,
        key: &Value,
        value: Value,
        raw: bool,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<()> {
        self.set_property_flags(receiver, key, value, raw, 0, host, budget)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_property_flags(
        &mut self,
        receiver: &Value,
        key: &Value,
        value: Value,
        raw: bool,
        flags: u32,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<()> {
        let id = self.object_id(receiver)?;
        let units = match key {
            Value::String(s) => s.clone(),
            _ => key.text().encode_utf16().collect(),
        };
        self.objects[id].member_flags.insert(units.clone(), flags);
        if !raw {
            let existing = self.resolve_member(receiver, key, true, host, budget)?;
            if let Value::Object(object) = &existing
                && let Some(id) = object.object
                && self
                    .objects
                    .get(id)
                    .is_some_and(|o| matches!(o.kind, ObjectKind::Property { .. }))
            {
                return self.write_property(&existing, receiver, value, host, budget);
            }
        }
        self.set_member(receiver, key, value)?;
        self.objects[id].member_flags.insert(units, flags);
        Ok(())
    }
    pub(crate) fn read_property(
        &mut self,
        value: &Value,
        receiver: &Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = self.object_id(value)?;
        let ObjectKind::Property { getter, .. } = self.objects[id].kind.clone() else {
            bail!("TJS object is not a property")
        };
        let getter = getter.context("property has no getter")?;
        let Value::Object(reference) = value else {
            unreachable!()
        };
        let context = reference
            .context
            .map(Value::object)
            .unwrap_or_else(|| receiver.clone());
        self.invoke(&getter, &context, &[], host, budget)
    }
    pub(crate) fn write_property(
        &mut self,
        property: &Value,
        receiver: &Value,
        value: Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<()> {
        let id = self.object_id(property)?;
        let ObjectKind::Property { setter, .. } = self.objects[id].kind.clone() else {
            bail!("TJS object is not a property")
        };
        let setter = setter.context("property has no setter")?;
        let Value::Object(reference) = property else {
            unreachable!()
        };
        let context = reference
            .context
            .map(Value::object)
            .unwrap_or_else(|| receiver.clone());
        self.invoke(&setter, &context, &[value], host, budget)?;
        Ok(())
    }
}
