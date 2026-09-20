use crate::{Value, Vm, object::ObjectKind};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Literal {
    Value(Value),
    Array(Vec<Literal>),
    Dictionary(Vec<(Literal, Literal)>),
}
impl Literal {
    pub(crate) fn validate(&self, depth: usize) -> Result<()> {
        ensure!(depth < 128, "constant literal nesting exceeds limit");
        match self {
            Self::Value(Value::Object(reference)) => ensure!(
                reference.object.is_none() && reference.context.is_none(),
                "object handle in constant literal"
            ),
            Self::Array(items) => {
                ensure!(items.len() <= 100_000, "constant array exceeds limit");
                for item in items {
                    item.validate(depth + 1)?;
                }
            }
            Self::Dictionary(items) => {
                ensure!(items.len() <= 100_000, "constant dictionary exceeds limit");
                for (key, value) in items {
                    key.validate(depth + 1)?;
                    value.validate(depth + 1)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}
impl Vm {
    pub(crate) fn materialize_literal(
        &mut self,
        literal: &Literal,
        budget: &mut u64,
    ) -> Result<Value> {
        if *budget == 0 {
            return Err(crate::unsupported(
                "constant literal execution budget exhausted",
            ));
        }
        *budget -= 1;
        match literal {
            Literal::Value(value) => Ok(value.clone()),
            Literal::Array(items) => {
                let items = items
                    .iter()
                    .map(|item| self.materialize_literal(item, budget))
                    .collect::<Result<Vec<_>>>()?;
                self.allocate(ObjectKind::Array(items))
            }
            Literal::Dictionary(items) => {
                let result = self.new_dictionary()?;
                for (key, value) in items {
                    let key = self.materialize_literal(key, budget)?;
                    let value = self.materialize_literal(value, budget)?;
                    self.set_member(&result, &key, value)?;
                }
                Ok(result)
            }
        }
    }
}
