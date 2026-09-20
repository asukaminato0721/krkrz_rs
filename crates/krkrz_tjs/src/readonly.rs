//! Immutable native data views. Each read of a nested container creates a fresh
//! dispatch identity; copying through Scripts.clone yields ordinary TJS objects.
use crate::{Value, Vm, object::ObjectKind};
use anyhow::{Result, bail, ensure};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug)]
pub enum ReadOnlyData {
    Scalar(Value),
    Array(Vec<Arc<ReadOnlyData>>),
    Dictionary(BTreeMap<Vec<u16>, Arc<ReadOnlyData>>),
}
#[derive(Clone, Debug)]
pub(crate) struct ReadOnlyView {
    pub data: Arc<ReadOnlyData>,
    pub alive: Arc<AtomicBool>,
}
impl ReadOnlyView {
    pub fn check(&self) -> Result<()> {
        ensure!(
            self.alive.load(Ordering::Relaxed),
            "native data owner has been invalidated"
        );
        Ok(())
    }
    pub fn keys(&self) -> Result<Vec<Value>> {
        self.check()?;
        Ok(match self.data.as_ref() {
            ReadOnlyData::Array(items) => {
                (0..items.len()).map(|i| Value::Integer(i as i64)).collect()
            }
            ReadOnlyData::Dictionary(items) => items.keys().cloned().map(Value::String).collect(),
            ReadOnlyData::Scalar(_) => unreachable!(),
        })
    }
    fn lookup(&self, key: &Value) -> Result<Option<Arc<ReadOnlyData>>> {
        self.check()?;
        Ok(match self.data.as_ref() {
            ReadOnlyData::Array(items) => {
                let name = key.unary("string")?.text();
                if name == "count" {
                    Some(Arc::new(ReadOnlyData::Scalar(Value::Integer(
                        items.len() as i64
                    ))))
                } else {
                    let index = name.parse::<i64>().ok();
                    if index.is_some_and(|i| i < 0) {
                        bail!("negative native array index");
                    }
                    index.and_then(|i| items.get(i as usize)).cloned()
                }
            }
            ReadOnlyData::Dictionary(items) => {
                let Value::String(key) = key.unary("string")? else {
                    unreachable!()
                };
                items.get(&key).cloned()
            }
            ReadOnlyData::Scalar(_) => unreachable!(),
        })
    }
    pub fn has(&self, key: &Value) -> Result<bool> {
        Ok(self.lookup(key)?.is_some())
    }
}
impl Vm {
    /// Bind an immutable tree to a native owner's lifetime. Scalars are copied;
    /// containers share their backing data while retaining distinct identities.
    pub fn new_readonly_data(
        &mut self,
        data: Arc<ReadOnlyData>,
        alive: Arc<AtomicBool>,
    ) -> Result<Value> {
        ensure!(
            alive.load(Ordering::Relaxed),
            "native data owner has been invalidated"
        );
        match data.as_ref() {
            ReadOnlyData::Scalar(v) => Ok(v.clone()),
            _ => self.allocate(ObjectKind::ReadOnly(ReadOnlyView { data, alive })),
        }
    }
    pub(crate) fn readonly_member(
        &mut self,
        view: ReadOnlyView,
        key: &Value,
        optional: bool,
    ) -> Result<Value> {
        match view.lookup(key)? {
            Some(data) => self.new_readonly_data(data, view.alive),
            None if optional
                || matches!(view.data.as_ref(), ReadOnlyData::Dictionary(_))
                || key.text().parse::<u64>().is_ok() =>
            {
                Ok(Value::Void)
            }
            None => bail!("native data member not found: {}", key.text()),
        }
    }
}
