//! PSBFile binding established with the installed plugin and synthetic PSB v2
//! documents. Format decoding lives in krkrz_assets and derives from GARbro.
use crate::Services;
use anyhow::{Context, Result};
use krkrz_assets::psb::{Document, Value as Psb};
use krkrz_tjs::{ReadOnlyData, Value, Vm, unsupported};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct File {
    root: Arc<ReadOnlyData>,
    alive: Arc<AtomicBool>,
}
impl Drop for File {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("PSBFile")?;
    for method in ["PSBFile", "finalize"] {
        vm.register_native(&format!("PSBFile.{method}"))?;
    }
    vm.register_native_property(&class, "root", Some("PSBFile.get:root"), None)?;
    Ok(())
}
fn convert(value: Psb, bytes: &[u8], budget: &mut u64) -> Result<Arc<ReadOnlyData>> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| unsupported("PSB conversion execution budget exceeded"))?;
    let result = match value {
        Psb::Null => ReadOnlyData::Scalar(Value::Void),
        Psb::Bool(v) => ReadOnlyData::Scalar(Value::Integer(v.into())),
        Psb::Integer(v) => ReadOnlyData::Scalar(Value::Integer(v)),
        Psb::Real(v) => ReadOnlyData::Scalar(Value::Real(v)),
        Psb::String(v) => ReadOnlyData::Scalar(Value::string(&v)),
        Psb::List(v) => ReadOnlyData::Array(
            v.into_iter()
                .map(|v| convert(v, bytes, budget))
                .collect::<Result<_>>()?,
        ),
        Psb::Object(v) => ReadOnlyData::Dictionary(
            v.into_iter()
                .map(|(k, v)| Ok((k.encode_utf16().collect(), convert(v, bytes, budget)?)))
                .collect::<Result<_>>()?,
        ),
        Psb::Resource { offset, length, .. } => {
            let end = offset
                .checked_add(length)
                .context("PSB resource range overflow")?;
            ReadOnlyData::Scalar(Value::Octet(
                bytes
                    .get(offset..end)
                    .context("PSB resource outside storage")?
                    .to_vec(),
            ))
        }
    };
    Ok(Arc::new(result))
}
impl Services {
    pub(crate) fn psb_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("PSBFile requires an object context");
        };
        let id = reference
            .object
            .context("PSBFile requires a non-null context")?;
        match operation {
            "@initialize" => {
                self.psb_files.insert(id, None);
            }
            "@invalidate" => {
                self.psb_files.remove(&id);
            }
            "PSBFile" => {
                anyhow::ensure!(
                    self.psb_files.contains_key(&id),
                    "context has no PSBFile native instance"
                );
                let name = args
                    .first()
                    .context("PSBFile requires a storage name")?
                    .text();
                let bytes = self.read_storage(&name)?;
                let root = convert(Document::parse(&bytes)?.root, &bytes, budget)?;
                self.psb_files.insert(
                    id,
                    Some(File {
                        root,
                        alive: Arc::new(AtomicBool::new(true)),
                    }),
                );
            }
            "finalize" => {}
            "get:root" => {
                let file = self
                    .psb_files
                    .get(&id)
                    .and_then(Option::as_ref)
                    .context("PSBFile is not initialized")?;
                return vm.new_readonly_data(file.root.clone(), file.alive.clone());
            }
            _ => {
                return Err(unsupported(format!(
                    "PSBFile.{operation} is not implemented"
                )));
            }
        }
        Ok(Value::Void)
    }
}
