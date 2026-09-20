//! Explicit TJS invalidation, following tTJSCustomObject::_Finalize.
//! The arena still owns objects: this does not implement reference-counted GC.
use crate::{Host, Value, Vm, object::ObjectKind};
use anyhow::Result;

impl Vm {
    pub(crate) fn is_valid(&self, value: &Value) -> Result<Value> {
        let valid = if matches!(value, Value::Object(_)) {
            self.objects[self.object_handle(value)?].valid
        } else {
            true
        };
        Ok(Value::Integer(i64::from(valid)))
    }

    pub fn invalidate(
        &mut self,
        value: &Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        if !matches!(value, Value::Object(_)) {
            return Ok(Value::Integer(0));
        }
        let id = self.object_handle(value)?;
        if matches!(self.objects[id].kind, ObjectKind::ReadOnly(_)) {
            self.objects[id].valid = false;
            return Ok(Value::Integer(0));
        }
        let object = &self.objects[id];
        // Native method dispatches inherit the default E_NOTIMPL. Native class
        // objects (including the four builtin constructors) are custom objects.
        let native_method = match &object.kind {
            ObjectKind::Native(name) => {
                !["Array", "Dictionary", "Exception", "RegExp"].contains(&name.as_str())
            }
            ObjectKind::Method { .. } => true,
            _ => false,
        };
        if !object.valid || native_method {
            return Ok(Value::Integer(0));
        }
        if object.finalizing {
            return Ok(Value::Integer(1));
        }
        self.objects[id].finalizing = true;
        let result = self.finalize_object(id, host, budget);
        self.objects[id].finalizing = false;
        result?;
        self.objects[id].valid = false;
        Ok(Value::Integer(1))
    }

    fn finalize_object(&mut self, id: usize, host: &mut impl Host, budget: &mut u64) -> Result<()> {
        if matches!(
            self.objects[id].kind,
            ObjectKind::Instance | ObjectKind::Global
        ) {
            let receiver = Value::object(id);
            let mut finalize = self.get_member(&receiver, &Value::string("finalize"), true)?;
            // TJSDefaultFuncCall retries a property through its getter. A
            // write-only or invalid property yields an ignored error code.
            if let Value::Object(reference) = &finalize
                && let Some(property) = reference.object
                && self.objects.get(property).is_some_and(|object| {
                    object.valid
                        && matches!(
                            object.kind,
                            ObjectKind::Property {
                                getter: Some(_),
                                ..
                            }
                        )
                })
            {
                finalize = self.read_property(&finalize, &receiver, host, budget)?;
            }
            // FuncCall's missing/non-callable return code is ignored upstream;
            // exceptions actually raised by a script finalizer propagate.
            if let Value::Object(reference) = &finalize
                && let Some(function) = reference.object
                && self.objects.get(function).is_some_and(|object| {
                    object.valid
                        && matches!(
                            object.kind,
                            ObjectKind::Function(_)
                                | ObjectKind::Native(_)
                                | ObjectKind::Method { .. }
                        )
                })
            {
                self.invoke(&finalize, &receiver, &[], host, budget)?;
            }
        }
        for finalizer in self.objects[id].native_finalizers.clone().iter().rev() {
            // Date's native Invalidate is empty. Cached native closures can
            // still read its timestamp after the script object is invalidated.
            if !matches!(finalizer.as_str(), "Date.@invalidate" | "RandomGenerator.@invalidate") {
                host.call_with_context(self, finalizer, &Value::object(id), &[], budget)?;
            }
        }
        let object = &mut self.objects[id];
        object.native_finalizers.clear();
        object.members.clear();
        object.member_flags.clear();
        object.member_layout = Default::default();
        if let ObjectKind::Array(items) = &mut object.kind {
            items.clear();
        }
        if id == 0 {
            self.globals.clear();
        }
        Ok(())
    }
}
