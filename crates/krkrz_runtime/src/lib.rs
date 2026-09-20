//! Shared deterministic session services. Presentation and full Kirikiri objects remain unimplemented.
pub mod audio;
pub mod compositor;
pub mod scheduler;
pub mod window;
use anyhow::{Context, Result, ensure};
use krkrz_assets::{cx::CxEncryption, storage::Storage, text};
use krkrz_core::{Limits, save_directory};
use krkrz_tjs::{Host, Instruction, Value, Vm, compile_with_preprocessor, unsupported};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
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
    pub windows: BTreeMap<usize, window::WindowState>,
    pub arguments: BTreeMap<String, String>,
    loaded_plugins: BTreeSet<String>,
    depth: usize,
}
impl Services {
    fn script(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        expression: bool,
        budget: &mut u64,
    ) -> Result<Value> {
        if self.depth >= 128 {
            return Err(unsupported("script call stack depth exceeded"));
        }
        let bytes = self.storage.read(name)?;
        if bytes.starts_with(b"TJS2") {
            return Err(unsupported(format!(
                "{name}: compiled TJS2 bytecode execution is not implemented"
            )));
        }
        let source = text::decode(&bytes).with_context(|| format!("decode {name}"))?;
        let program = compile_with_preprocessor(name, &source, expression, &mut vm.preprocessor)?;
        self.depth += 1;
        let result = vm.execute_in_context(&program, context, self, budget);
        self.depth -= 1;
        result
    }
}
impl Host for Services {
    fn call_with_context(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        if let Some(operation) = name.strip_prefix("Window.") {
            let Value::Object(reference) = context else {
                anyhow::bail!("Window requires an object context")
            };
            let id = reference
                .object
                .context("Window requires a non-null context")?;
            if operation == "@initialize" {
                self.windows.entry(id).or_default();
                return Ok(Value::Void);
            }
            let window = self
                .windows
                .get_mut(&id)
                .context("context has no Window native instance")?;
            if operation == "Window" {
                if args
                    .first()
                    .is_some_and(|v| !matches!(v, Value::Void) && *v != Value::NULL)
                {
                    return Err(unsupported("parented windows are not implemented"));
                }
                ensure!(!window.constructed, "Window constructor has already run");
                let system = vm
                    .globals
                    .get("System")
                    .cloned()
                    .context("System namespace is missing")?;
                window.caption = vm
                    .get_member(&system, &Value::string("title"), false)?
                    .text();
                window.constructed = true;
                return Ok(Value::Void);
            }
            if operation == "onResize" {
                ensure!(window.constructed, "Window constructor has not run");
                // WindowIntf's default event methods forward an event Dictionary
                // to `action`. An absent action is ignored by the native engine.
                let action = vm.get_member(context, &Value::string("action"), true)?;
                if !matches!(action, Value::Void) {
                    let event = vm.new_dictionary()?;
                    vm.set_member(&event, &Value::string("type"), Value::string("onResize"))?;
                    vm.set_member(&event, &Value::string("target"), context.clone())?;
                    return vm.call_function(&action, context, &[event], self, budget);
                }
                return Ok(Value::Void);
            }
            return window.call(operation, args);
        }
        self.call(vm, name, args, budget)
    }
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
            "Plugins.link" => {
                let path = arg(0)?.text().replace('\\', "/");
                match path
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "scriptsex.dll" => {
                        if !self.loaded_plugins.contains("scriptsex.dll") {
                            vm.register_scripts_ex()?;
                            self.loaded_plugins.insert("scriptsex.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    _ => Err(unsupported(format!(
                        "unsupported Kirikiri native operation: Plugins.link({path})"
                    ))),
                }
            }
            "System.getTickCount" => Ok(Value::Integer(self.time_ms as i64)),
            "System.getArgument" => Ok(self
                .arguments
                .get(&arg(0)?.text())
                .map(|v| Value::string(v))
                .unwrap_or(Value::Void)),
            "Scripts.execStorage" | "Scripts.evalStorage" => {
                if args
                    .get(1)
                    .is_some_and(|v| !matches!(v, Value::Void) && !v.text().is_empty())
                {
                    return Err(unsupported("script storage read modes are not implemented"));
                }
                let context = script_context(args.get(2));
                self.script(
                    vm,
                    &arg(0)?.text(),
                    &context,
                    name == "Scripts.evalStorage",
                    budget,
                )
            }
            "Scripts.exec" | "Scripts.eval" => {
                let offset = args.get(2).map(Value::integer).transpose()?.unwrap_or(0);
                if !(0..=1_000_000).contains(&offset) {
                    return Err(unsupported("script line offset outside supported range"));
                }
                let source = "\n".repeat(offset as usize) + &arg(0)?.text();
                let storage = args
                    .get(1)
                    .filter(|v| !matches!(v, Value::Void))
                    .map(Value::text)
                    .unwrap_or_else(|| {
                        if name == "Scripts.eval" {
                            "<eval>".into()
                        } else {
                            "<exec>".into()
                        }
                    });
                let program = compile_with_preprocessor(
                    &storage,
                    &source,
                    name == "Scripts.eval",
                    &mut vm.preprocessor,
                )?;
                if self.depth >= 128 {
                    return Err(unsupported("dynamic script call depth exceeded"));
                }
                self.depth += 1;
                let result =
                    vm.execute_in_context(&program, &script_context(args.get(3)), self, budget);
                self.depth -= 1;
                result
            }
            "Storages.addAutoPath" => {
                self.storage.add_search_path(&arg(0)?.text())?;
                Ok(Value::Void)
            }
            "Storages.isExistentStorage" => Ok(Value::Integer(i64::from(
                self.storage.resolve(&arg(0)?.text()).is_ok(),
            ))),
            "Storages.extractStorageName"
            | "Storages.extractStoragePath"
            | "Storages.extractStorageExt"
            | "Storages.chopStorageExt" => {
                let path = arg(0)?.text().replace('\\', "/");
                let begin = path.rfind(['/', '>']).map_or(0, |i| i + 1);
                let dot = path[begin..].rfind('.').map(|i| i + begin);
                let result = match name {
                    "Storages.extractStorageName" => &path[begin..],
                    "Storages.extractStoragePath" => &path[..begin],
                    "Storages.extractStorageExt" => dot.map_or("", |i| &path[i..]),
                    _ => dot.map_or(path.as_str(), |i| &path[..i]),
                };
                Ok(Value::string(result))
            }
            "Storages.getPlacedPath" => {
                let name = arg(0)?.text();
                match self.storage.placed_path(&name) {
                    Ok(path) => Ok(Value::string(&path)),
                    Err(_) => Ok(Value::string("")),
                }
            }
            _ => Err(unsupported(format!(
                "unsupported Kirikiri native operation: {name}"
            ))),
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
        let mut vm = Vm::default();
        vm.preprocessor.set("kirikiriz", 1);
        window::register(&mut vm)?;
        let system = vm.register_namespace("System")?;
        for (key, value) in [
            ("exePath", format!("{}/", storage.project.display())),
            ("title", "krkrz_rs".into()),
            ("osName", std::env::consts::OS.into()),
            ("platformName", std::env::consts::OS.into()),
            ("versionString", "1.2.0.3".into()),
        ] {
            vm.set_member(&system, &Value::string(key), Value::string(&value))?;
        }
        for name in [
            "Debug.message",
            "Debug.notice",
            "System.getTickCount",
            "System.getArgument",
            "Scripts.execStorage",
            "Scripts.evalStorage",
            "Scripts.exec",
            "Scripts.eval",
            "Storages.addAutoPath",
            "Storages.isExistentStorage",
            "Storages.getPlacedPath",
            "Storages.extractStorageName",
            "Storages.extractStoragePath",
            "Storages.extractStorageExt",
            "Storages.chopStorageExt",
            "Plugins.link",
        ] {
            vm.register_native(name)?;
        }
        Ok(Self {
            vm,
            services: Services {
                storage,
                save_dir,
                time_ms: 0,
                trace: vec![],
                messages: vec![],
                trace_enabled: false,
                windows: BTreeMap::new(),
                arguments: BTreeMap::from([("-debugwin".into(), "no".into())]),
                loaded_plugins: BTreeSet::new(),
                depth: 0,
            },
            budget,
        })
    }
    pub fn startup(&mut self) -> Result<Value> {
        self.execute_storage("startup.tjs")
    }
    pub fn execute_storage(&mut self, name: &str) -> Result<Value> {
        self.services.script(
            &mut self.vm,
            name,
            &Value::object(0),
            false,
            &mut self.budget,
        )
    }
    pub fn evaluate(&mut self, source: &str) -> Result<Value> {
        let p = compile_with_preprocessor("<tool>", source, true, &mut self.vm.preprocessor)?;
        self.vm.execute(&p, &mut self.services, &mut self.budget)
    }
    pub fn advance_clock(&mut self, time_ms: u64) -> Result<()> {
        ensure!(
            time_ms >= self.services.time_ms,
            "session clock cannot move backwards"
        );
        self.services.time_ms = time_ms;
        self.dispatch_events()?;
        Ok(())
    }
    /// Deliver one pending batch. Events posted by callbacks wait for the next
    /// batch, preventing native recursion and preserving the shared VM budget.
    pub fn dispatch_events(&mut self) -> Result<()> {
        let pending = self
            .services
            .windows
            .iter_mut()
            .filter_map(|(id, state)| std::mem::take(&mut state.resize_pending).then_some(*id))
            .collect::<Vec<_>>();
        for id in pending {
            let window = Value::object(id);
            let callback = self
                .vm
                .get_member(&window, &Value::string("onResize"), false)?;
            self.vm.call_function(
                &callback,
                &window,
                &[],
                &mut self.services,
                &mut self.budget,
            )?;
        }
        Ok(())
    }
}

fn script_context(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Void) => Value::object(0),
        Some(Value::Object(reference)) if reference.object.is_none() => Value::object(0),
        Some(value) => value.clone(),
    }
}
