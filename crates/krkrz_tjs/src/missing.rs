//! Dynamic member dispatch (tTJSCustomObject::CallGetMissing/CallSetMissing).
use crate::{Host, Value, Vm, object::ObjectKind};
use anyhow::{Result, bail};

impl Vm {
    pub fn set_call_missing(&mut self, value: &Value) -> Result<()> {
        let Value::Object(reference) = value else {
            bail!("Scripts.setCallMissing requires an object");
        };
        if reference.object.is_some() {
            let id = self.object_id(value)?;
            self.objects[id].call_missing = true;
        }
        Ok(())
    }

    pub(crate) fn missing_candidate(
        &mut self,
        receiver: &Value,
        key: &Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<bool> {
        let Value::Object(reference) = receiver else {
            return Ok(false);
        };
        let Some(id) = reference.object else {
            return Ok(false);
        };
        let object = &self.objects[self.object_id(receiver)?];
        if !object.call_missing || object.processing_missing || self.has_member(receiver, key)? {
            return Ok(false);
        }
        // Array's numeric dispatch precedes its custom named-member dispatch.
        if matches!(self.objects[id].kind, ObjectKind::Array(_))
            && crate::dispatch::index(key).is_some()
        {
            return Ok(false);
        }
        Ok(matches!(
            self.resolve_member(receiver, key, true, host, budget)?,
            Value::Void
        ))
    }

    pub(crate) fn missing_get(
        &mut self,
        receiver: &Value,
        key: &Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        if self.missing_candidate(receiver, key, host, budget)? {
            self.call_missing(receiver, key, None, host, budget)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn call_missing(
        &mut self,
        receiver: &Value,
        key: &Value,
        value: Option<Value>,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        let id = self.object_id(receiver)?;
        self.objects[id].processing_missing = true;
        let result = (|| {
            let function = self.get_property(
                receiver,
                &Value::string("missing"),
                true,
                false,
                host,
                budget,
            )?;
            let callable = self.object_id(&function).ok().is_some_and(|id| {
                matches!(
                    self.objects[id].kind,
                    ObjectKind::Function(_) | ObjectKind::Native(_) | ObjectKind::Method { .. }
                )
            });
            if !callable {
                return Ok(None);
            }
            let setting = value.is_some();
            let cell = self.allocate(ObjectKind::VariantProperty(value.unwrap_or(Value::Void)))?;
            let handled = self.invoke(
                &function,
                &Value::object(id),
                &[
                    Value::Integer(setting.into()),
                    key.unary("string")?,
                    cell.clone(),
                ],
                host,
                budget,
            )?;
            if handled.integer()? as i32 == 0 {
                return Ok(None);
            }
            Ok(Some(self.read_property(&cell, receiver, host, budget)?))
        })();
        // Exceptions and execution limits must also release the reentrancy guard.
        self.objects[id].processing_missing = false;
        result
    }
}
