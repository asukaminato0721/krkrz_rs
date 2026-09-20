//! Shared deterministic session services. Presentation and full Kirikiri objects remain unimplemented.
pub mod audio;
pub mod compositor;
pub mod scheduler;
use anyhow::{Context, Result, bail, ensure};
use krkrz_assets::{cx::CxEncryption, storage::Storage, text};
use krkrz_core::{Limits, save_directory, storage_name};
use krkrz_tjs::{Host, Instruction, Value, Vm, compile, compile_expression};
use serde::Serialize;
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize)]
pub struct TraceEvent {
    pub time_ms: u64,
    pub storage: String,
    pub line: usize,
    pub column: usize,
    pub operation: String,
}
pub struct Services {
    pub storage: Storage,
    pub save_dir: PathBuf,
    pub time_ms: u64,
    pub trace: Vec<TraceEvent>,
    pub messages: Vec<String>,
    pub trace_enabled: bool,
    depth: usize,
}
impl Services {
    fn script(&mut self, vm: &mut Vm, name: &str, budget: &mut u64) -> Result<Value> {
        ensure!(self.depth < 128, "script call stack depth exceeded");
        let bytes = self.storage.read(name)?;
        ensure!(
            !bytes.starts_with(b"TJS2"),
            "{name}: compiled TJS2 bytecode execution is not implemented"
        );
        let source = text::decode(&bytes).with_context(|| format!("decode {name}"))?;
        let program = compile(name, &source)?;
        self.depth += 1;
        let result = vm.execute(&program, self, budget);
        self.depth -= 1;
        result
    }
}
impl Host for Services {
    fn call(&mut self, vm: &mut Vm, name: &str, args: &[Value], budget: &mut u64) -> Result<Value> {
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("{name}: missing argument {i}"))
        };
        match name {
            "Debug.message" | "Debug.notice" => {
                self.messages
                    .push(args.iter().map(Value::text).collect::<Vec<_>>().join(" "));
                Ok(Value::Void)
            }
            "System.getTickCount" => Ok(Value::Integer(self.time_ms as i64)),
            "Scripts.execStorage" => self.script(vm, &arg(0)?.text(), budget),
            "Scripts.exec" | "Scripts.eval" => {
                let source = arg(0)?.text();
                let program = if name == "Scripts.eval" {
                    compile_expression("<eval>", &source)?
                } else {
                    compile("<exec>", &source)?
                };
                ensure!(self.depth < 128, "dynamic script call depth exceeded");
                self.depth += 1;
                let result = vm.execute(&program, self, budget);
                self.depth -= 1;
                result
            }
            "Storages.addAutoPath" => {
                self.storage.add_search_path(&arg(0)?.text())?;
                Ok(Value::Void)
            }
            "Storages.isExistentStorage" => {
                let name = storage_name(&arg(0)?.text())?;
                Ok(Value::Integer(i64::from(
                    self.storage.resolve(&name).is_ok(),
                )))
            }
            "Storages.getPlacedPath" => {
                let name = arg(0)?.text();
                match self.storage.placed_path(&name) {
                    Ok(path) => Ok(Value::string(&path)),
                    Err(_) => Ok(Value::string("")),
                }
            }
            _ => bail!("unsupported Kirikiri native operation: {name}"),
        }
    }
    fn trace(&mut self, instruction: &Instruction) -> Result<()> {
        if self.trace_enabled {
            ensure!(self.trace.len() < 1_000_000, "trace event limit exceeded");
            self.trace.push(TraceEvent {
                time_ms: self.time_ms,
                storage: instruction.location.storage.clone(),
                line: instruction.location.line,
                column: instruction.location.column,
                operation: format!("{:?}", instruction.op),
            });
        }
        Ok(())
    }
}
pub struct Session {
    pub vm: Vm,
    pub services: Services,
    pub budget: u64,
}
impl Session {
    pub fn open(
        project: &Path,
        save_dir: Option<&Path>,
        otome_profile: bool,
        budget: u64,
    ) -> Result<Self> {
        let save_dir = save_directory(project, save_dir)?;
        let cipher = if otome_profile {
            Some(CxEncryption::otome_domain()?)
        } else {
            None
        };
        let storage = Storage::open(project, cipher, Limits::default())?;
        Ok(Self {
            vm: Vm::default(),
            services: Services {
                storage,
                save_dir,
                time_ms: 0,
                trace: vec![],
                messages: vec![],
                trace_enabled: false,
                depth: 0,
            },
            budget,
        })
    }
    pub fn startup(&mut self) -> Result<Value> {
        self.execute_storage("startup.tjs")
    }
    pub fn execute_storage(&mut self, name: &str) -> Result<Value> {
        self.services.script(&mut self.vm, name, &mut self.budget)
    }
    pub fn evaluate(&mut self, source: &str) -> Result<Value> {
        let p = compile_expression("<tool>", source)?;
        self.vm.execute(&p, &mut self.services, &mut self.budget)
    }
    pub fn advance_clock(&mut self, time_ms: u64) -> Result<()> {
        ensure!(
            time_ms >= self.services.time_ms,
            "session clock cannot move backwards"
        );
        self.services.time_ms = time_ms;
        Ok(())
    }
}
