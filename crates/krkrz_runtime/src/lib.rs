//! Shared deterministic session services. Presentation and full Kirikiri objects remain unimplemented.
mod alpha_movie;
mod app_lock;
mod async_trigger;
pub mod audio;
pub mod compositor;
mod continuous;
mod csv;
mod dialog;
pub mod display;
mod draw_device;
mod font;
pub mod fonts;
mod get_sample;
pub mod graphics;
mod kag_parser;
mod layer;
mod layer_draw;
mod menu;
mod phase_vocoder;
mod plugins;
mod psb_file;
mod save_storage;
pub mod scheduler;
mod sound;
mod sound_flags;
mod sound_stream;
mod text_render;
mod timer;
mod video_overlay;
pub mod window;
mod window_ex;
use anyhow::{Context, Result, ensure};
use krkrz_assets::{cx::CxEncryption, storage::Storage, text};
use krkrz_core::{Limits, save_directory};
use krkrz_tjs::{Host, Instruction, ObjectRef, Value, Vm, compile_with_preprocessor, unsupported};
pub use layer::input::InputEvent;
pub use menu::{MenuAppearance, MenuBitmap};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub use window_ex::SystemMenuItem;
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
    /// Unix wall-clock origin; replay hosts can set a recorded value before startup.
    pub epoch_ms: i64,
    pub trace: Vec<TraceEvent>,
    pub messages: Vec<String>,
    pub trace_enabled: bool,
    pub windows: BTreeMap<usize, window::WindowState>,
    main_window: Option<usize>,
    draw_devices: draw_device::Devices,
    /// The headless display size. A native host replaces it before startup.
    pub screen_size: (u32, u32),
    displays: Vec<display::Monitor>,
    pub image_cache: graphics::ImageCache,
    pub fonts: fonts::FontBook,
    app_locks: app_lock::AppLocks,
    async_triggers: async_trigger::State,
    continuous_handlers: Vec<Option<Value>>,
    events: scheduler::EventQueue,
    timers: BTreeMap<usize, timer::Timer>,
    layers: BTreeMap<usize, layer::Layer>,
    font_objects: BTreeMap<usize, font::Font>,
    kag_parsers: BTreeMap<usize, std::rc::Rc<std::cell::RefCell<kag_parser::Parser>>>,
    csv_parsers: BTreeMap<usize, csv::Parser>,
    sounds: BTreeMap<usize, sound::Sound>,
    wave_flags_class: Value,
    wave_flags: BTreeMap<usize, Option<usize>>,
    vocoders: BTreeMap<usize, phase_vocoder::Settings>,
    videos: BTreeMap<usize, video_overlay::Video>,
    psb_files: BTreeMap<usize, Option<psb_file::File>>,
    text_renderers: BTreeMap<usize, text_render::Renderer>,
    layer_draw: layer_draw::State,
    menus: menu::State,
    dialogs: dialog::State,
    window_ex: window_ex::State,
    alpha_movies: BTreeMap<usize, alpha_movie::Player>,
    alpha_movie_links: BTreeSet<String>,
    sound_global_volume: i32,
    sample_plugin: get_sample::State,
    pub arguments: BTreeMap<String, String>,
    loaded_plugins: BTreeSet<String>,
    plugin_names: Vec<String>,
    depth: usize,
}
impl Services {
    fn read_storage(&mut self, name: &str) -> Result<Vec<u8>> {
        if let Ok(path) = save_storage::path(&self.storage.project, &self.save_dir, name)
            && path.is_file()
        {
            ensure!(
                path.metadata()?.len() <= 512 << 20,
                "save resource exceeds size limit"
            );
            return Ok(std::fs::read(path)?);
        }
        Ok(self.storage.read(name)?.to_vec())
    }
    fn script(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        expression: bool,
        mode: &str,
        budget: &mut u64,
    ) -> Result<Value> {
        if self.depth >= 128 {
            return Err(unsupported("script call stack depth exceeded"));
        }
        let bytes = self.read_storage(name)?;
        let (mode, offset) = save_storage::offset_mode(mode)?;
        if !mode.is_empty() {
            return Err(unsupported("unsupported script storage read mode"));
        }
        let bytes = bytes
            .get(offset.unwrap_or(0)..)
            .context("script read offset exceeds file size")?;
        if bytes.starts_with(b"TJS2") {
            return Err(unsupported(format!(
                "{name}: compiled TJS2 bytecode execution is not implemented"
            )));
        }
        let source = text::decode(bytes).with_context(|| format!("decode {name}"))?;
        let program = compile_with_preprocessor(name, &source, expression, &mut vm.preprocessor)?;
        self.depth += 1;
        let result = vm.execute_in_context(&program, context, self, budget);
        self.depth -= 1;
        result
    }
}
impl Host for Services {
    fn unix_time_ms(&self) -> i64 {
        self.epoch_ms
            .saturating_add(self.time_ms.min(i64::MAX as u64) as i64)
    }
    fn call_with_result(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
        result_needed: bool,
    ) -> Result<Value> {
        if name == "WaveSoundBuffer.getSample" {
            return self.sample_call(vm, "getSample", context, args, budget, result_needed);
        }
        // TJS compiles eval as an expression with a return only when the caller
        // requested a result. Discarded eval permits result-less swap operations.
        let name = match (name, result_needed) {
            ("Scripts.eval", false) => "Scripts.evalDiscard",
            ("Scripts.evalStorage", false) => "Scripts.evalStorageDiscard",
            _ => name,
        };
        self.call_with_context(vm, name, context, args, budget)
    }
    fn call_with_context(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        if let Some(operation) = name.strip_prefix("WaveFlags.") {
            return self.wave_flags_call(operation, context, args);
        }
        if let Some(operation) = name.strip_prefix("KAGParser.") {
            return self.kag_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("BasicDrawDevice.") {
            return self.draw_device_call(operation, context);
        }
        if let Some(operation) = name.strip_prefix("WindowEx.") {
            return self.window_ex_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("Layer.") {
            return self.layer_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("Font.") {
            return self.font_call(operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("Timer.") {
            return self.timer_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("AsyncTrigger.") {
            return self.async_call(vm, operation, context, args, budget);
        }
        if name == "Window.get:menu" {
            return self.window_menu(vm, context, budget);
        }
        if let Some(operation) = name.strip_prefix("MenuItem.") {
            return self.menu_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("AlphaMovie.") {
            return self.alpha_movie_call(operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("GdiPlus.") {
            return self.layer_draw_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("VideoOverlay.") {
            return self.video_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("PhaseVocoder.") {
            return self.vocoder_call(operation, context, args);
        }
        if let Some(operation) = name.strip_prefix("WaveSoundBuffer.") {
            return self.sound_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("TextRenderBase.") {
            return self.text_render_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("WIN32Dialog.") {
            return self.dialog_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("PSBFile.") {
            return self.psb_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("CSVParser.") {
            return self.csv_call(vm, operation, context, args, budget);
        }
        if let Some(operation) = name.strip_prefix("Window.") {
            if operation == "get:mainWindow" {
                return Ok(self.main_window.map_or(Value::NULL, |id| {
                    Value::Object(ObjectRef {
                        object: Some(id),
                        context: Some(id),
                    })
                }));
            }
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
            if operation == "@invalidate" {
                if self.main_window == Some(id) {
                    self.main_window = None;
                }
                let registered = if let Some(window) = self.windows.get_mut(&id) {
                    window.invalidating = true;
                    std::mem::take(&mut window.registered_objects)
                } else {
                    Vec::new()
                };
                for object in registered {
                    if let Err(error) = vm.invalidate(&object, self, budget) {
                        if error.downcast_ref::<krkrz_tjs::VmAbort>().is_some() {
                            return Err(error);
                        }
                        self.messages.push(format!("{error:#}"));
                    }
                }
                if let Some(window) = self.windows.get(&id) {
                    let device = window.draw_device.clone();
                    if matches!(device, Value::Object(_)) && device != Value::NULL {
                        vm.invalidate(&device, self, budget)?;
                    }
                }
                self.windows.remove(&id);
                self.window_ex.windows.remove(&id);
                return Ok(Value::Void);
            }
            if matches!(operation, "get:focusedLayer" | "set:focusedLayer") {
                return self.window_focus(vm, id, operation == "set:focusedLayer", args, budget);
            }
            if operation == "set:fullScreen" {
                let enabled = args
                    .first()
                    .context("Window.fullScreen: missing value")?
                    .truth()?;
                self.window_full_screen(id, enabled)?;
                return Ok(Value::Void);
            }
            if operation == "set:drawDevice" {
                let value = args
                    .first()
                    .context("Window.drawDevice: missing value")?
                    .clone();
                self.set_draw_device(vm, id, value, budget)?;
                return Ok(Value::Void);
            }
            let first_window = !self
                .windows
                .values()
                .any(|window| window.constructed && !window.invalidating);
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
                if first_window {
                    self.main_window = Some(id);
                }
                let class = self
                    .draw_devices
                    .class
                    .clone()
                    .context("BasicDrawDevice class is missing")?;
                let device = vm.construct(&class, &[], self, budget)?;
                self.set_draw_device(vm, id, device, budget)?;
                return Ok(Value::Void);
            }
            if operation == "close" {
                let callback = vm.get_member(context, &Value::string("onCloseQuery"), false)?;
                vm.call_function(&callback, context, &[Value::Integer(1)], self, budget)?;
                return Ok(Value::Void);
            }
            if operation == "onCloseQuery" {
                if args.first().is_some_and(|v| v.truth().unwrap_or(false)) {
                    vm.invalidate(context, self, budget)?;
                }
                return Ok(Value::Void);
            }
            if let Some(keys) = layer::input::window_event_keys(operation) {
                ensure!(window.constructed, "Window constructor has not run");
                // WindowIntf's default event methods forward an event Dictionary
                // to `action`. An absent action is ignored by the native engine.
                let action = vm.get_member(context, &Value::string("action"), true)?;
                if !matches!(action, Value::Void) {
                    let event = vm.new_dictionary()?;
                    vm.set_member(&event, &Value::string("type"), Value::string(operation))?;
                    vm.set_member(
                        &event,
                        &Value::string("target"),
                        Value::Object(ObjectRef {
                            object: Some(id),
                            context: Some(id),
                        }),
                    )?;
                    for (key, value) in keys.iter().zip(args) {
                        vm.set_member(&event, &Value::string(key), value.clone())?;
                    }
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
            "TextStream.read" => {
                let bytes = self.read_storage(&arg(0)?.text())?;
                let (mode, offset) = save_storage::offset_mode(&arg(1)?.text())?;
                if !mode.is_empty() {
                    return Err(unsupported("unsupported text read mode"));
                }
                Ok(Value::String(text::decode_units(
                    bytes
                        .get(offset.unwrap_or(0)..)
                        .context("text read offset exceeds file size")?,
                )?))
            }
            "TextStream.write" | "TextStream.writePlugin" => {
                let Value::String(units) = arg(1)? else {
                    anyhow::bail!("text writer requires a string");
                };
                let path =
                    save_storage::path(&self.storage.project, &self.save_dir, &arg(0)?.text())?;
                let (bytes, offset) = if name == "TextStream.write" {
                    let (mode, offset) = save_storage::offset_mode(&arg(2)?.text())?;
                    (text::encode_units(units, &mode)?, offset)
                } else {
                    let string = String::from_utf16(units)?;
                    let bytes = if arg(2)?.text() == "utf8" {
                        string.into_bytes()
                    } else {
                        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(&string);
                        if errors {
                            return Err(unsupported(
                                "saveStruct CP932 fallback escaping is not implemented",
                            ));
                        }
                        bytes.into_owned()
                    };
                    (bytes, None)
                };
                save_storage::write(&path, &bytes, offset)?;
                Ok(Value::Void)
            }
            "Debug.message" | "Debug.notice" => {
                self.messages
                    .push(args.iter().map(Value::text).collect::<Vec<_>>().join(" "));
                Ok(Value::Void)
            }
            "Plugins.link" => {
                let spelling = arg(0)?.unary("string")?.text();
                if self.plugin_names.contains(&spelling) {
                    return Ok(Value::Void);
                }
                let path = spelling.replace('\\', "/");
                let result = match path
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "packinone.dll" => {
                        if !self.loaded_plugins.contains("packinone.dll") {
                            if !self.loaded_plugins.contains("scriptsex.dll") {
                                vm.register_scripts_ex()?;
                            }
                            if !self.loaded_plugins.contains("savestruct.dll") {
                                vm.register_save_struct()?;
                            }
                            if !self.loaded_plugins.contains("csvparser.dll") {
                                csv::register(vm)?;
                            }
                            self.loaded_plugins.extend(
                                ["scriptsex.dll", "savestruct.dll", "csvparser.dll"]
                                    .map(String::from),
                            );
                            plugins::packinone(vm)?;
                            self.loaded_plugins.insert("packinone.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    // wuvorbis installs the Ogg decoder; WaveSoundBuffer already
                    // uses the Rust Vorbis decoder for these streams.
                    "wuvorbis.dll" => {
                        self.loaded_plugins.insert("wuvorbis.dll".into());
                        Ok(Value::Void)
                    }
                    // extrans installs transition providers, with no TJS globals.
                    // Layer.beginTransition remains responsible for validating
                    // the selected provider and rejecting unimplemented effects.
                    "extrans.dll" | "extnagano.dll" => {
                        self.loaded_plugins
                            .insert(path.rsplit('/').next().unwrap().to_ascii_lowercase());
                        Ok(Value::Void)
                    }
                    // PackinOne already registers the image extension. The
                    // original accepts this component alias without replacing
                    // its Layer methods. Their invocation is still explicit
                    // unsupported until the native Layer implementation exists.
                    "layereximage.dll" if self.loaded_plugins.contains("packinone.dll") => {
                        Ok(Value::Void)
                    }
                    "kagparserex.dll" => {
                        if !self.loaded_plugins.contains("kagparserex.dll") {
                            kag_parser::register(vm)?;
                            self.loaded_plugins.insert("kagparserex.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "scriptsex.dll" => {
                        if !self.loaded_plugins.contains("scriptsex.dll") {
                            vm.register_scripts_ex()?;
                            self.loaded_plugins.insert("scriptsex.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "csvparser.dll" => {
                        if !self.loaded_plugins.contains("csvparser.dll") {
                            csv::register(vm)?;
                            self.loaded_plugins.insert("csvparser.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "savestruct.dll" => {
                        if !self.loaded_plugins.contains("savestruct.dll") {
                            vm.register_save_struct()?;
                            self.loaded_plugins.insert("savestruct.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "textrender.dll" => {
                        if !self.loaded_plugins.contains("textrender.dll") {
                            text_render::register(vm)?;
                            self.loaded_plugins.insert("textrender.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "win32dialog.dll" => {
                        self.dialogs
                            .link(vm, path.rsplit('/').next().unwrap_or(""))?;
                        Ok(Value::Void)
                    }
                    "windowex.dll" => {
                        self.window_ex
                            .link(vm, path.rsplit('/').next().unwrap_or(""))?;
                        Ok(Value::Void)
                    }
                    "menu.dll" => {
                        self.menus
                            .relink(vm, path.rsplit('/').next().unwrap_or(""))?;
                        Ok(Value::Void)
                    }
                    "alphamovie.dll" => {
                        let spelling = path.rsplit('/').next().unwrap_or("");
                        if !self.alpha_movie_links.contains(spelling) {
                            alpha_movie::register(vm)?;
                            self.alpha_movie_links.insert(spelling.into());
                        }
                        Ok(Value::Void)
                    }
                    "layerexdraw.dll" => {
                        let link_name = path.rsplit('/').next().unwrap_or("");
                        if !self.loaded_plugins.contains("layerexdraw.dll") {
                            self.layer_draw = layer_draw::register(vm, link_name)?;
                            self.loaded_plugins.insert("layerexdraw.dll".into());
                        } else {
                            ensure!(
                                self.layer_draw.link_name == link_name,
                                "GdiPlus class is already registered under another plugin-name spelling"
                            );
                        }
                        Ok(Value::Void)
                    }
                    "psbfile.dll" => {
                        if !self.loaded_plugins.contains("psbfile.dll") {
                            psb_file::register(vm)?;
                            self.loaded_plugins.insert("psbfile.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    "getsample.dll" => {
                        if !self.loaded_plugins.contains("getsample.dll") {
                            get_sample::register(vm)?;
                            self.loaded_plugins.insert("getsample.dll".into());
                        }
                        Ok(Value::Void)
                    }
                    _ => Err(unsupported(format!(
                        "unsupported Kirikiri native operation: Plugins.link({path})"
                    ))),
                };
                if result.is_ok() {
                    self.plugin_names.push(spelling);
                }
                result
            }
            "Plugins.getList" => {
                vm.new_native_array(self.plugin_names.iter().map(|s| Value::string(s)).collect())
            }
            "System.getKeyState" => {
                let key = args
                    .first()
                    .context("getKeyState requires a key")?
                    .integer()? as u32;
                let current = args.get(1).map(Value::truth).transpose()?.unwrap_or(true);
                let mut pressed = false;
                for window in self.windows.values_mut() {
                    let recent = window.input.pressed.remove(&key);
                    pressed |= if current {
                        window.input.keys.contains(&key)
                    } else {
                        recent
                    };
                }
                Ok(Value::Integer(pressed.into()))
            }
            "System.getTickCount" => Ok(Value::Integer(self.time_ms as i64)),
            "System.addContinuousHandler" | "System.removeContinuousHandler" => {
                let handler = arg(0)?;
                ensure!(
                    matches!(handler, Value::Object(_)),
                    "continuous handler must be an Object"
                );
                let index = self
                    .continuous_handlers
                    .iter()
                    .position(|h| h.as_ref() == Some(handler));
                if name == "System.addContinuousHandler" {
                    if index.is_none() && handler != &Value::NULL {
                        ensure!(
                            self.continuous_handlers.len() < 100_000,
                            "continuous handler limit exceeded"
                        );
                        self.continuous_handlers.push(Some(handler.clone()));
                    }
                } else if let Some(index) = index {
                    self.continuous_handlers[index] = None;
                }
                Ok(Value::Void)
            }
            "System.readRegValue" => {
                arg(0)?.unary("string")?;
                // The Linux host has no Windows registry. Kirikiri SDL2 likewise
                // reports a missing value, allowing scripts to use their defaults.
                Ok(Value::Void)
            }
            "System.get:screenWidth" => {
                Ok(Value::Integer(self.application_display().bounds[2].into()))
            }
            "System.get:screenHeight" => {
                Ok(Value::Integer(self.application_display().bounds[3].into()))
            }
            "System.get:desktopLeft" => Ok(Value::Integer(self.desktop_bounds()[0].into())),
            "System.get:desktopTop" => Ok(Value::Integer(self.desktop_bounds()[1].into())),
            "System.get:desktopWidth" => Ok(Value::Integer(self.desktop_bounds()[2].into())),
            "System.get:desktopHeight" => Ok(Value::Integer(self.desktop_bounds()[3].into())),
            "System.addFont" => {
                let name = arg(0)?.text();
                if self.storage.resolve(&name).is_err() {
                    return Ok(Value::Void);
                }
                let data = self.read_storage(&name)?;
                self.fonts.add(data)?;
                // PackinOne's installed addFont wrapper discards the count.
                Ok(Value::Void)
            }
            "System.createAppLock" => Ok(Value::Integer(
                self.app_locks.acquire(&arg(0)?.text())?.into(),
            )),
            "System.get:graphicCacheLimit" => Ok(Value::Integer(self.image_cache.limit() as i64)),
            "System.set:graphicCacheLimit" => {
                self.image_cache.set_limit(arg(0)?.integer()?);
                Ok(Value::Void)
            }
            "System.getArgument" => Ok(self
                .arguments
                .get(&arg(0)?.text())
                .map(|v| Value::string(v))
                .unwrap_or(Value::Void)),
            "System.setArgument" => {
                let name = arg(0)?.unary("string")?.text();
                let value = arg(1)?.unary("string")?.text();
                self.arguments.insert(name, value);
                Ok(Value::Void)
            }
            "System.doCompact" => {
                let level = match args.first() {
                    None | Some(Value::Void) => 100,
                    Some(value) => value.integer()? as i32,
                };
                // Upstream compacts its string/variant allocation pools at 5.
                // Rust allocations have no equivalent retained free pools.
                if level >= 10 {
                    for archive in &mut self.storage.archives {
                        archive.clear_cache();
                    }
                }
                if level >= 15 {
                    self.image_cache.clear();
                }
                Ok(Value::Void)
            }
            "Scripts.setCallMissing" => {
                vm.set_call_missing(arg(0)?)?;
                Ok(Value::Void)
            }
            "Scripts.getClassNames" => vm.class_names(arg(0)?),
            "Scripts.execStorage" | "Scripts.evalStorage" | "Scripts.evalStorageDiscard" => {
                let context = script_context(args.get(2));
                self.script(
                    vm,
                    &arg(0)?.text(),
                    &context,
                    name == "Scripts.evalStorage",
                    &args.get(1).map(Value::text).unwrap_or_default(),
                    budget,
                )
            }
            "Scripts.exec" | "Scripts.eval" | "Scripts.evalDiscard" => {
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
                        if name.starts_with("Scripts.eval") {
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
                save_storage::path(&self.storage.project, &self.save_dir, &arg(0)?.text())
                    .is_ok_and(|p| p.is_file())
                    || self.storage.resolve(&arg(0)?.text()).is_ok(),
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
                if let Ok(path) = save_storage::path(&self.storage.project, &self.save_dir, &name)
                    && path.is_file()
                {
                    return Ok(Value::string(&path.to_string_lossy()));
                }
                match self.storage.placed_path(&name) {
                    Ok(path) => Ok(Value::string(&path)),
                    Err(_) => Ok(Value::string("")),
                }
            }
            "Storages.getFullPath" => Ok(Value::string(&self.storage.full_path(&arg(0)?.text())?)),
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
        let constants = krkrz_tjs::compile(
            "<core constants>",
            include_str!("../data/core_constants.tjs"),
        )?;
        // Trusted, fixed bootstrap data has its own bounded initialization
        // budget. User scripts retain the full caller-supplied budget.
        vm.execute(&constants, &mut (), &mut 100_000)?;
        window::register(&mut vm)?;
        let draw_device_class = draw_device::register(&mut vm)?;
        async_trigger::register(&mut vm)?;
        timer::register(&mut vm)?;
        layer::register(&mut vm)?;
        font::register(&mut vm)?;
        sound::register(&mut vm)?;
        let wave_flags_class = sound_flags::register(&mut vm)?;
        phase_vocoder::register(&mut vm)?;
        video_overlay::register(&mut vm)?;
        let system = vm.register_namespace("System")?;
        for name in [
            "screenWidth",
            "screenHeight",
            "desktopLeft",
            "desktopTop",
            "desktopWidth",
            "desktopHeight",
        ] {
            vm.register_native_property(&system, name, Some(&format!("System.get:{name}")), None)?;
        }
        vm.register_native_property(
            &system,
            "graphicCacheLimit",
            Some("System.get:graphicCacheLimit"),
            Some("System.set:graphicCacheLimit"),
        )?;
        for (key, value) in [
            ("exePath", format!("{}/", storage.project.display())),
            (
                "exeName",
                storage
                    .project
                    .join(if otome_profile {
                        "otomedomain.exe"
                    } else {
                        "krkrz_engine"
                    })
                    .display()
                    .to_string(),
            ),
            ("title", "krkrz_rs".into()),
            ("osName", std::env::consts::OS.into()),
            ("platformName", std::env::consts::OS.into()),
            ("versionString", "1.2.0.3".into()),
            ("dataPath", format!("file://.{}/", save_dir.display())),
        ] {
            vm.set_member(&system, &Value::string(key), Value::string(&value))?;
        }
        for name in [
            "Debug.message",
            "Debug.notice",
            "System.getTickCount",
            "System.getKeyState",
            "System.addContinuousHandler",
            "System.removeContinuousHandler",
            "System.readRegValue",
            "System.createAppLock",
            "System.getArgument",
            "System.setArgument",
            "System.doCompact",
            "Scripts.execStorage",
            "Scripts.evalStorage",
            "Scripts.exec",
            "Scripts.eval",
            "Scripts.getClassNames",
            "Scripts.setCallMissing",
            "Storages.addAutoPath",
            "Storages.isExistentStorage",
            "Storages.getPlacedPath",
            "Storages.getFullPath",
            "Storages.extractStorageName",
            "Storages.extractStoragePath",
            "Storages.extractStorageExt",
            "Storages.chopStorageExt",
            "Plugins.link",
            "Plugins.getList",
        ] {
            vm.register_native(name)?;
        }
        Ok(Self {
            vm,
            services: Services {
                storage,
                save_dir,
                time_ms: 0,
                epoch_ms: <() as Host>::unix_time_ms(&()),
                trace: vec![],
                messages: vec![],
                trace_enabled: false,
                windows: BTreeMap::new(),
                main_window: None,
                draw_devices: draw_device::Devices::new(draw_device_class),
                screen_size: (1280, 720),
                displays: Vec::new(),
                image_cache: graphics::ImageCache::new(graphics::automatic_limit()),
                app_locks: app_lock::AppLocks::default(),
                async_triggers: async_trigger::State::default(),
                continuous_handlers: Vec::new(),
                events: scheduler::EventQueue::default(),
                timers: BTreeMap::new(),
                layers: BTreeMap::new(),
                font_objects: BTreeMap::new(),
                kag_parsers: BTreeMap::new(),
                fonts: fonts::FontBook::default(),
                csv_parsers: BTreeMap::new(),
                sounds: BTreeMap::new(),
                wave_flags_class,
                wave_flags: BTreeMap::new(),
                vocoders: BTreeMap::new(),
                videos: BTreeMap::new(),
                psb_files: BTreeMap::new(),
                text_renderers: BTreeMap::new(),
                layer_draw: layer_draw::State::default(),
                menus: menu::State::default(),
                dialogs: dialog::State::default(),
                window_ex: window_ex::State::default(),
                alpha_movies: BTreeMap::new(),
                alpha_movie_links: BTreeSet::new(),
                sound_global_volume: 100_000,
                sample_plugin: get_sample::State::default(),
                arguments: BTreeMap::from([("-debugwin".into(), "no".into())]),
                loaded_plugins: BTreeSet::new(),
                plugin_names: Vec::new(),
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
            "",
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
        self.services.advance_timers(time_ms)?;
        self.services.sound_advance(time_ms);
        self.services.time_ms = time_ms;
        self.dispatch_window_commands()?;
        self.dispatch_events()?;
        Ok(())
    }
    /// Advance one host tick, deliver queued callbacks, and prepare live window
    /// surfaces. Native presentation and headless inspection use the same path.
    pub fn tick(&mut self, time_ms: u64) -> Result<()> {
        self.budget = self
            .budget
            .checked_sub(1)
            .ok_or_else(|| unsupported("host tick execution budget exceeded"))?;
        self.advance_clock(time_ms)?;
        self.dispatch_continuous()?;
        self.dispatch_layer_transitions()?;
        if self.services.events.exclusive_posted() {
            return Ok(());
        }
        let windows: Vec<_> = self.services.windows.keys().copied().collect();
        for id in windows {
            if self
                .services
                .windows
                .get(&id)
                .is_some_and(|w| w.constructed && w.visible && !w.minimized)
            {
                self.prepare_window_paint(&Value::object(id))?;
                self.complete_window_transitions(id)?;
            }
        }
        Ok(())
    }
    /// Deliver one pending batch. Events posted by callbacks wait for the next
    /// batch, preventing native recursion and preserving the shared VM budget.
    pub fn dispatch_events(&mut self) -> Result<()> {
        let through = self.services.events.begin_batch();
        self.services
            .dispatch_async(&mut self.vm, through, 1, &mut self.budget)?;
        if self.services.events.exclusive_posted() {
            return Ok(());
        }
        let pending_menus = std::mem::take(&mut self.services.menus.pending);
        let pending_audio = self
            .services
            .sounds
            .iter_mut()
            .map(|(id, sound)| (*id, std::mem::take(&mut sound.events)))
            .collect::<Vec<_>>();
        let pending_sounds = self
            .services
            .sounds
            .iter_mut()
            .filter_map(|(id, sound)| std::mem::take(&mut sound.fade_pending).then_some(*id))
            .collect::<Vec<_>>();
        let pending = self
            .services
            .windows
            .iter_mut()
            .filter_map(|(id, state)| std::mem::take(&mut state.resize_pending).then_some(*id))
            .collect::<Vec<_>>();
        for id in pending {
            // An earlier callback in this batch can invalidate another window.
            if !self.services.windows.contains_key(&id) {
                continue;
            }
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
        for id in pending_menus {
            if self.services.menus.is_live(id) {
                let item = Value::object(id);
                let callback = self
                    .vm
                    .get_member(&item, &Value::string("onClick"), false)?;
                self.vm.call_function(
                    &callback,
                    &item,
                    &[],
                    &mut self.services,
                    &mut self.budget,
                )?;
            }
        }
        for (id, events) in pending_audio {
            for (generation, name, argument) in events {
                if !self
                    .services
                    .sounds
                    .get(&id)
                    .is_some_and(|s| s.generation == generation)
                {
                    break;
                }
                if name == "onStatusChanged" {
                    self.services.sound_status(
                        &mut self.vm,
                        &Value::object(id),
                        id,
                        "stop",
                        &mut self.budget,
                    )?;
                } else {
                    self.services.sound_event(
                        &mut self.vm,
                        &Value::object(id),
                        name,
                        &[argument],
                        &mut self.budget,
                    )?;
                }
            }
        }
        for id in pending_sounds {
            if self.services.sounds.contains_key(&id) {
                self.services.sound_event(
                    &mut self.vm,
                    &Value::object(id),
                    "onFadeCompleted",
                    &[],
                    &mut self.budget,
                )?;
            }
        }
        self.services
            .dispatch_async(&mut self.vm, through, 0, &mut self.budget)?;
        if !self.services.events.exclusive_posted() {
            self.services
                .dispatch_async(&mut self.vm, through, 2, &mut self.budget)?;
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
