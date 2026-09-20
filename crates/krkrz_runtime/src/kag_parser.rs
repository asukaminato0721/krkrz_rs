//! KAGParserEx native execution, following the community KAGParser.cpp implementation.
use crate::{Services, layer::object};
use anyhow::{Context, Result, ensure};
use krkrz_kag::native::{Attribute, Position, Source, Tag, scan_tag};
use krkrz_tjs::{Value, Vm, compile_expression, unsupported};
use std::{cell::RefCell, rc::Rc, sync::Arc};
#[derive(Clone)]
struct Args {
    dict: Value,
    list: Value,
    names: Vec<String>,
}
impl Args {
    fn new(vm: &mut Vm) -> Result<Self> {
        Ok(Self {
            dict: vm.new_dictionary()?,
            list: vm.new_native_array(Vec::new())?,
            names: Vec::new(),
        })
    }
    fn clear(&mut self, vm: &mut Vm, budget: &mut u64) -> Result<()> {
        vm.dictionary_assign(&self.dict, &[], true, budget)?;
        vm.set_member(&self.list, &Value::string("count"), Value::Integer(0))?;
        self.names.clear();
        Ok(())
    }
    fn add(&mut self, vm: &mut Vm, name: &str, value: Value) -> Result<()> {
        vm.set_member(&self.dict, &Value::string(name), value)?;
        vm.set_member(
            &self.list,
            &Value::Integer(self.names.len() as i64),
            Value::string(name),
        )?;
        self.names.push(name.into());
        Ok(())
    }
    fn get(&self, vm: &mut Vm, name: &str) -> Result<Value> {
        vm.get_member(&self.dict, &Value::string(name), true)
    }
    fn result(&self, vm: &mut Vm) -> Result<Value> {
        vm.set_member(&self.dict, &Value::string("taglist"), self.list.clone())?;
        Ok(self.dict.clone())
    }
    fn duplicate(&self, vm: &mut Vm, budget: &mut u64) -> Result<Self> {
        let mut result = Self::new(vm)?;
        for name in &self.names {
            charge(budget)?;
            let value = self.get(vm, name)?;
            result.add(vm, name, value)?;
        }
        Ok(result)
    }
}
#[derive(Clone)]
struct Call {
    storage: String,
    position: Position,
    label: String,
    macro_base: usize,
    macro_depth: usize,
    exclude: i32,
    conditions: Vec<(i32, bool)>,
    original_line: Vec<u16>,
}
#[derive(Clone)]
pub(crate) struct Parser {
    alive: bool,
    source: Option<Arc<Source>>,
    storage: String,
    position: Position,
    current_line: Vec<u16>,
    label: String,
    ignore_cr: bool,
    special: bool,
    multiline: bool,
    debug: i32,
    interrupted: bool,
    macros: Value,
    param_macros: Value,
    args: Args,
    macro_args: Vec<Args>,
    macro_base: usize,
    recording: Option<(String, Vec<u16>)>,
    conditions: Vec<(i32, bool)>,
    exclude: i32,
    calls: Vec<Call>,
}
impl Parser {
    fn new(vm: &mut Vm) -> Result<Self> {
        Ok(Self {
            alive: true,
            source: None,
            storage: String::new(),
            position: Position::default(),
            current_line: Vec::new(),
            label: String::new(),
            ignore_cr: false,
            special: true,
            multiline: false,
            debug: 1,
            interrupted: false,
            macros: vm.new_dictionary()?,
            param_macros: vm.new_dictionary()?,
            args: Args::new(vm)?,
            macro_args: Vec::new(),
            macro_base: 0,
            recording: None,
            conditions: Vec::new(),
            exclude: -1,
            calls: Vec::new(),
        })
    }
    fn break_condition(&mut self) {
        self.conditions.clear();
        self.exclude = -1;
        self.recording = None;
        self.macro_args.truncate(self.macro_base);
    }
    fn clear_buffer(&mut self) {
        self.source = None;
        self.current_line.clear();
        self.storage.clear();
        self.break_condition();
    }
    fn jump(&mut self, label: &str) -> Result<()> {
        if label.is_empty() {
            return Ok(());
        }
        let source = self.source.as_ref().context("KAG parser has no scenario")?;
        let line = *source
            .labels
            .get(label)
            .with_context(|| format!("KAG label not found in {}: {label}", self.storage))?;
        self.position = Position {
            line,
            ..Position::default()
        };
        self.label = source.aliases[line].clone();
        self.break_condition();
        Ok(())
    }
    fn push_call(&mut self) -> Result<()> {
        if self.calls.len() >= 128 {
            return Err(unsupported("KAG call depth exceeded"));
        }
        let source = self.source.clone().context("KAG parser has no scenario")?;
        let previous = (0..self.position.line.min(source.lines.len()))
            .rev()
            .find(|&line| !source.aliases[line].is_empty());
        let mut position = self.position.clone();
        position.line -= previous.unwrap_or(0);
        self.calls.push(Call {
            storage: self.storage.clone(),
            position,
            label: previous
                .map(|line| source.aliases[line].clone())
                .unwrap_or_default(),
            macro_base: self.macro_base,
            macro_depth: self.macro_args.len(),
            exclude: self.exclude,
            conditions: self.conditions.clone(),
            original_line: source
                .lines
                .get(self.position.line)
                .cloned()
                .unwrap_or_default(),
        });
        self.macro_base = self.macro_args.len();
        Ok(())
    }
}
fn charge(budget: &mut u64) -> Result<()> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| unsupported("KAGParser execution budget exhausted"))?;
    Ok(())
}
fn utf16(value: &Value) -> Vec<u16> {
    match value {
        Value::String(v) => v.clone(),
        _ => value.text().encode_utf16().collect(),
    }
}
fn bound(id: usize) -> Value {
    Value::Object(krkrz_tjs::ObjectRef {
        object: Some(id),
        context: Some(id),
    })
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    crate::plugins::declare_class(vm, "KAGParser")?;
    let class = vm.globals["KAGParser"].clone();
    for key in [
        "curLine",
        "curPos",
        "curLineStr",
        "curLabel",
        "macros",
        "paramMacros",
        "macroParams",
        "mp",
        "callStackDepth",
    ] {
        vm.register_native_property(&class, key, Some(&format!("KAGParser.get:{key}")), None)?;
    }
    Ok(())
}
impl Services {
    fn kag_event(
        &mut self,
        vm: &mut Vm,
        id: usize,
        name: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        let context = bound(id);
        let function =
            vm.get_property(&context, &Value::string(name), true, false, self, budget)?;
        if matches!(function, Value::Void) {
            return Ok(None);
        }
        let result = vm.call_function(&function, &context, args, self, budget)?;
        ensure!(
            self.kag_parsers.get(&id).is_some_and(|p| p.borrow().alive),
            "KAGParser invalidated during {name}"
        );
        Ok(Some(result))
    }
    fn kag_eval(
        &mut self,
        vm: &mut Vm,
        id: usize,
        expression: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let storage = self.kag_parsers[&id].borrow().storage.clone();
        let program = compile_expression(&storage, &expression.text())?;
        let result = vm.execute_in_context(&program, &bound(id), self, budget)?;
        ensure!(
            self.kag_parsers.get(&id).is_some_and(|p| p.borrow().alive),
            "KAGParser invalidated during expression"
        );
        Ok(result)
    }
    fn kag_load(&mut self, vm: &mut Vm, id: usize, name: &str, budget: &mut u64) -> Result<()> {
        let handle = self.kag_parsers[&id].clone();
        let reload = {
            let mut p = handle.borrow_mut();
            p.break_condition();
            p.source.is_none() || p.storage != name
        };
        if reload {
            handle.borrow_mut().clear_buffer();
            let result =
                self.kag_event(vm, id, "onScenarioLoad", &[Value::string(name)], budget)?;
            let units = if let Some(Value::String(s)) = result {
                s
            } else {
                krkrz_assets::text::decode(&self.read_storage(name)?)?
                    .encode_utf16()
                    .collect()
            };
            let mut source = Source::parse(&units)?;
            source.index_labels()?;
            let mut p = handle.borrow_mut();
            p.storage = name.into();
            p.source = Some(Arc::new(source));
        }
        {
            let mut p = handle.borrow_mut();
            p.position = Position::default();
            p.current_line = p
                .source
                .as_ref()
                .and_then(|s| s.lines.first())
                .cloned()
                .unwrap_or_default();
            p.break_condition();
        }
        self.kag_event(vm, id, "onScenarioLoaded", &[Value::string(name)], budget)?;
        Ok(())
    }
    fn kag_goto(
        &mut self,
        vm: &mut Vm,
        id: usize,
        storage: &str,
        label: &str,
        budget: &mut u64,
    ) -> Result<()> {
        if !storage.is_empty() {
            self.kag_load(vm, id, storage, budget)?;
        }
        if !label.is_empty() {
            self.kag_parsers[&id].borrow_mut().jump(label)?;
        }
        Ok(())
    }
    fn kag_return(
        &mut self,
        vm: &mut Vm,
        id: usize,
        storage: &str,
        label: &str,
        budget: &mut u64,
    ) -> Result<()> {
        let handle = self.kag_parsers[&id].clone();
        let call = handle
            .borrow()
            .calls
            .last()
            .context("KAG return without call")?
            .clone();
        {
            let mut p = handle.borrow_mut();
            p.macro_base = call.macro_depth;
            p.macro_args.truncate(call.macro_depth);
        }
        if !storage.is_empty() || !label.is_empty() {
            self.kag_goto(vm, id, storage, label, budget)?;
        } else {
            self.kag_load(vm, id, &call.storage, budget)?;
            let mut p = handle.borrow_mut();
            p.jump(&call.label)?;
            let mut position = call.position;
            position.line = position
                .line
                .checked_add(p.position.line)
                .context("KAG return line overflow")?;
            let source = p
                .source
                .as_ref()
                .context("KAG scenario cleared during return")?;
            ensure!(
                position.line <= source.lines.len()
                    && source
                        .lines
                        .get(position.line)
                        .is_none_or(|line| *line == call.original_line),
                "KAG return lost scenario synchronization"
            );
            p.position = position;
            p.exclude = call.exclude;
            p.conditions = call.conditions;
            p.current_line = p.position.line(p.source.as_ref().unwrap()).to_vec();
        }
        {
            let mut p = handle.borrow_mut();
            p.macro_base = call.macro_base;
            p.calls.pop();
        }
        self.kag_event(vm, id, "onAfterReturn", &[], budget)?;
        Ok(())
    }
    pub(crate) fn kag_call(
        &mut self,
        vm: &mut Vm,
        op: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let id = object(context)?.context("KAGParser requires non-null context")?;
        if op == "@initialize" {
            self.kag_parsers
                .insert(id, Rc::new(RefCell::new(Parser::new(vm)?)));
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            if let Some(p) = self.kag_parsers.remove(&id) {
                p.borrow_mut().alive = false;
            }
            return Ok(Value::Void);
        }
        let handle = self
            .kag_parsers
            .get(&id)
            .context("context has no KAGParser native instance")?
            .clone();
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("KAGParser.{op}: missing argument {i}"))
        };
        if let Some(key) = op.strip_prefix("get:") {
            let p = handle.borrow();
            return Ok(match key {
                "curLine" => Value::Integer(p.position.line as i64),
                "curPos" => Value::Integer(p.position.pos as i64),
                "curLineStr" => Value::String(p.current_line.clone()),
                "curStorage" => Value::string(&p.storage),
                "curLabel" => Value::string(&p.label),
                "ignoreCR" => Value::Integer(p.ignore_cr.into()),
                "processSpecialTags" => Value::Integer(p.special.into()),
                "multiLineTagEnabled" => Value::Integer(p.multiline.into()),
                "debugLevel" => Value::Integer(p.debug.into()),
                "macros" => p.macros.clone(),
                "paramMacros" => p.param_macros.clone(),
                "mp" | "macroParams" => p.macro_args.last().map_or(Value::NULL, |a| a.dict.clone()),
                "callStackDepth" => Value::Integer(p.calls.len() as i64),
                _ => {
                    return Err(unsupported(format!(
                        "unsupported KAGParser property: {key}"
                    )));
                }
            });
        }
        match op {
            "KAGParser" | "finalize" => {}
            "set:ignoreCR" => handle.borrow_mut().ignore_cr = arg(0)?.truth()?,
            "set:processSpecialTags" => handle.borrow_mut().special = arg(0)?.truth()?,
            "set:multiLineTagEnabled" => handle.borrow_mut().multiline = arg(0)?.truth()?,
            "set:debugLevel" => handle.borrow_mut().debug = arg(0)?.integer()? as i32,
            "interrupt" => handle.borrow_mut().interrupted = true,
            "resetInterrupt" => handle.borrow_mut().interrupted = false,
            "clear" => {
                let mut p = handle.borrow_mut();
                p.clear_buffer();
                p.macro_args.clear();
                p.macro_base = 0;
                p.calls.clear();
            }
            "clearCallStack" => {
                let mut p = handle.borrow_mut();
                p.calls.clear();
                p.macro_args.clear();
                p.macro_base = 0;
            }
            "popMacroArgs" => {
                ensure!(
                    handle.borrow_mut().macro_args.pop().is_some(),
                    "KAG macro argument stack underflow"
                );
            }
            "loadScenario" | "set:curStorage" => self.kag_load(vm, id, &arg(0)?.text(), budget)?,
            "goToLabel" => handle.borrow_mut().jump(&arg(0)?.text())?,
            "callLabel" => {
                let mut p = handle.borrow_mut();
                p.push_call()?;
                p.jump(&arg(0)?.text())?;
            }
            "assign" => {
                let source_id =
                    object(arg(0)?)?.context("KAGParser.assign requires non-null source")?;
                if source_id != id {
                    let source = self
                        .kag_parsers
                        .get(&source_id)
                        .context("KAGParser.assign requires a KAGParser")?
                        .borrow()
                        .clone();
                    let mut copied = source.clone();
                    copied.macro_args = source
                        .macro_args
                        .iter()
                        .map(|a| a.duplicate(vm, budget))
                        .collect::<Result<_>>()?;
                    let mut target = handle.borrow_mut();
                    vm.dictionary_assign(&target.macros, &[source.macros], false, budget)?;
                    vm.dictionary_assign(
                        &target.param_macros,
                        &[source.param_macros],
                        false,
                        budget,
                    )?;
                    copied.macros = target.macros.clone();
                    copied.param_macros = target.param_macros.clone();
                    copied.args = target.args.clone();
                    copied.special = target.special;
                    copied.multiline = target.multiline;
                    copied.interrupted = target.interrupted;
                    *target = copied;
                } else {
                    let p = handle.borrow();
                    vm.dictionary_assign(&p.macros, &[], true, budget)?;
                    vm.dictionary_assign(&p.param_macros, &[], true, budget)?;
                }
            }
            "store" => return handle.borrow().store(vm, budget),
            "restore" => self.kag_restore(vm, id, arg(0)?, budget)?,
            "getNextTag" => return self.kag_next(vm, id, budget),
            _ => {
                return Err(unsupported(format!(
                    "unsupported KAGParser operation: {op}"
                )));
            }
        }
        Ok(Value::Void)
    }
    fn kag_attribute(
        &mut self,
        vm: &mut Vm,
        id: usize,
        attr: &Attribute,
        condition: &mut bool,
        budget: &mut u64,
        depth: usize,
    ) -> Result<()> {
        charge(budget)?;
        if depth > 64 {
            return Err(unsupported("KAG parameter macro recursion exceeded"));
        }
        let handle = self.kag_parsers[&id].clone();
        if attr.all {
            let top = handle.borrow().macro_args.last().cloned();
            if let Some(top) = top {
                for name in &top.names {
                    if name != "tagname" {
                        let value = top.get(vm, name)?;
                        handle.borrow_mut().args.add(vm, name, value)?;
                    }
                }
            }
            return Ok(());
        }
        let params = handle.borrow().param_macros.clone();
        let macro_value = vm.get_member(&params, &Value::string(&attr.name), true)?;
        if matches!(macro_value, Value::Object(_)) {
            let count = vm
                .get_property(
                    &macro_value,
                    &Value::string("count"),
                    false,
                    false,
                    self,
                    budget,
                )?
                .integer()?;
            ensure!(
                (0..=65536).contains(&count),
                "invalid KAG parameter macro size"
            );
            for i in (0..count).step_by(2) {
                let name = vm
                    .get_property(&macro_value, &Value::Integer(i), false, false, self, budget)?
                    .text();
                let mut value = utf16(&vm.get_property(
                    &macro_value,
                    &Value::Integer(i + 1),
                    false,
                    false,
                    self,
                    budget,
                )?);
                let entity = value.first() == Some(&38);
                let macro_arg = value.first() == Some(&37);
                if entity || macro_arg {
                    value.remove(0);
                }
                self.kag_attribute(
                    vm,
                    id,
                    &Attribute {
                        name,
                        value,
                        entity,
                        macro_arg,
                        all: false,
                        end: attr.end,
                    },
                    condition,
                    budget,
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        let value = if attr.entity {
            let value = self.kag_eval(vm, id, &Value::String(attr.value.clone()), budget)?;
            if matches!(value, Value::Void) {
                value
            } else {
                value.unary("string")?
            }
        } else if attr.macro_arg {
            let top = handle.borrow().macro_args.last().cloned();
            if let Some(top) = top {
                let split = attr.value.iter().position(|c| *c == 124);
                let end = split.unwrap_or(attr.value.len());
                let name = String::from_utf16_lossy(&attr.value[..end]);
                let value = top.get(vm, &name)?;
                if matches!(value, Value::Void) && split.is_some() {
                    Value::String(attr.value[end + 1..].to_vec())
                } else {
                    value
                }
            } else {
                Value::String(attr.value.clone())
            }
        } else {
            Value::String(attr.value.clone())
        };
        if attr.name == "cond" {
            *condition = self.kag_eval(vm, id, &value, budget)?.truth()?;
        } else {
            handle.borrow_mut().args.add(vm, &attr.name, value)?;
        }
        Ok(())
    }
    fn kag_next(&mut self, vm: &mut Vm, id: usize, budget: &mut u64) -> Result<Value> {
        let handle = self.kag_parsers[&id].clone();
        loop {
            charge(budget)?;
            {
                let mut p = handle.borrow_mut();
                ensure!(p.alive, "KAGParser is invalid");
                if p.source
                    .as_ref()
                    .is_none_or(|s| p.position.line >= s.lines.len())
                {
                    return Ok(Value::Void);
                }
                p.args.clear(vm, budget)?;
                if p.interrupted {
                    p.interrupted = false;
                    p.args.add(vm, "tagname", Value::string("interrupt"))?;
                    return p.args.result(vm);
                }
            }
            // Labels and inline scripts are handled only at physical line starts.
            let special_line = {
                let p = handle.borrow();
                p.position.pos == 0 && p.position.buffer.is_none()
            };
            if special_line {
                let line = {
                    let p = handle.borrow();
                    p.source.as_ref().unwrap().lines[p.position.line].clone()
                };
                if line.first() == Some(&59) {
                    handle.borrow_mut().position.next_line();
                    continue;
                }
                if line.first() == Some(&42) {
                    let (label, caption) = {
                        let mut p = handle.borrow_mut();
                        ensure!(p.recording.is_none(), "label inside KAG macro");
                        p.label = p.source.as_ref().unwrap().aliases[p.position.line].clone();
                        let caption = line
                            .iter()
                            .position(|c| *c == 124)
                            .map_or(Value::Void, |i| Value::String(line[i + 1..].to_vec()));
                        (Value::string(&p.label), caption)
                    };
                    self.kag_event(vm, id, "onLabel", &[label, caption], budget)?;
                    handle.borrow_mut().position.next_line();
                    continue;
                }
                let line_text = String::from_utf16_lossy(&line);
                if matches!(line_text.as_str(), "[iscript]" | "[iscript]\\" | "@iscript") {
                    let (script, start, storage, active) = {
                        let mut p = handle.borrow_mut();
                        ensure!(p.recording.is_none(), "inline script inside KAG macro");
                        p.position.line += 1;
                        let start = p.position.line;
                        let mut script = Vec::new();
                        let source = p.source.clone().unwrap();
                        let active = p.exclude == -1;
                        while p.position.line < source.lines.len() {
                            let line = &source.lines[p.position.line];
                            if matches!(
                                String::from_utf16_lossy(line).as_ref(),
                                "[endscript]" | "[endscript]\\" | "@endscript"
                            ) {
                                break;
                            }
                            if active {
                                script.extend_from_slice(line);
                                script.extend([13, 10]);
                            }
                            p.position.line += 1;
                            charge(budget)?;
                        }
                        ensure!(
                            p.position.line < source.lines.len(),
                            "unterminated KAG inline script"
                        );
                        (
                            script,
                            start,
                            p.storage
                                .rsplit(['/', '\\'])
                                .next()
                                .unwrap_or("")
                                .to_owned(),
                            active,
                        )
                    };
                    if active {
                        self.kag_event(
                            vm,
                            id,
                            "onScript",
                            &[
                                Value::String(script),
                                Value::string(&storage),
                                Value::Integer(start as i64),
                            ],
                            budget,
                        )?;
                    }
                    handle.borrow_mut().position.next_line();
                    continue;
                }
            }
            let tag = {
                let mut p = handle.borrow_mut();
                let source = p.source.clone().unwrap();
                let line = p.position.line(&source).to_vec();
                p.current_line = line.clone();
                let pos = p.position.pos;
                let ch = line.get(pos).copied().unwrap_or(0);
                if !p.ignore_cr
                    && ((ch == 92 && pos + 1 == line.len())
                        || (ch == 0 && pos >= 3 && line.get(pos - 3..pos) == Some(&[91, 112, 93])))
                {
                    p.position.next_line();
                    continue;
                }
                if ch == 0 {
                    p.position.next_line();
                    if p.ignore_cr {
                        continue;
                    }
                    if let Some((_, record)) = &mut p.recording {
                        record.extend("[r eol=true]".encode_utf16());
                        continue;
                    }
                    if p.exclude != -1 {
                        continue;
                    }
                    p.args.add(vm, "tagname", Value::string("r"))?;
                    p.args.add(vm, "eol", Value::string("true"))?;
                    return p.args.result(vm);
                }
                let command = p.position.buffer.is_none() && pos == 0 && ch == 64;
                if !command && (ch != 91 || line.get(pos + 1) == Some(&91)) {
                    p.position.pos += if ch == 91 { 2 } else { 1 };
                    if ch == 9 {
                        continue;
                    }
                    if let Some((_, record)) = &mut p.recording {
                        if ch == 91 {
                            record.push(91);
                        }
                        if ch == 10 {
                            record.extend("[r]".encode_utf16());
                        } else {
                            record.push(ch);
                        }
                        continue;
                    }
                    if p.exclude != -1 {
                        continue;
                    }
                    if ch == 93 { eprintln!("KAG BRACKET: {}:{}:{} buffer={:?}", p.storage, p.position.line, pos, p.position.buffer.as_ref().map(|v| String::from_utf16_lossy(v).to_string())); }
                    p.args.add(
                        vm,
                        "tagname",
                        Value::string(if ch == 10 { "r" } else { "ch" }),
                    )?;
                    if ch != 10 {
                        p.args.add(vm, "text", Value::String(vec![ch]))?;
                    }
                    return p.args.result(vm);
                }
                let multiline = p.multiline;
                let tag = scan_tag(&source, &mut p.position, multiline)?;
                p.current_line = p.position.line(&source).to_vec();
                tag
            };
            let mut condition = true;
            handle
                .borrow_mut()
                .args
                .add(vm, "tagname", Value::string(&tag.name))?;
            for attribute in &tag.attributes {
                handle.borrow_mut().position.pos = attribute.end;
                let evaluate = {
                    let p = handle.borrow();
                    p.recording.is_none() && (p.exclude == -1 || tag.name == "elsif")
                };
                if evaluate {
                    self.kag_attribute(vm, id, attribute, &mut condition, budget, 0)?;
                }
            }
            handle.borrow_mut().position.pos = tag.delimiter;
            if self.kag_control(vm, id, &tag, condition, budget)? {
                return handle.borrow().args.result(vm);
            }
        }
    }
    fn kag_control(
        &mut self,
        vm: &mut Vm,
        id: usize,
        tag: &Tag,
        condition: bool,
        budget: &mut u64,
    ) -> Result<bool> {
        let handle = self.kag_parsers[&id].clone();
        let kind = if handle.borrow().special {
            tag.name.as_str()
        } else {
            ""
        };
        {
            let mut p = handle.borrow_mut();
            if p.recording.is_some() && condition && p.exclude == -1 {
                if kind == "endmacro" {
                    let (name, mut text) = p.recording.take().unwrap();
                    text.extend("[macropop]".encode_utf16());
                    vm.set_member(&p.macros, &Value::string(&name), Value::String(text))?;
                } else {
                    p.recording.as_mut().unwrap().1.extend_from_slice(&tag.raw);
                }
                tag.advance(&mut p.position);
                return Ok(false);
            }
        }
        match kind {
            "if" | "ignore" => {
                let exclude = handle.borrow().exclude;
                let active = if exclude == -1 {
                    let exp = handle.borrow().args.get(vm, "exp")?;
                    ensure!(!exp.text().is_empty(), "missing KAG conditional expression");
                    let value = self.kag_eval(vm, id, &exp, budget)?.truth()?;
                    if kind == "ignore" { !value } else { value }
                } else {
                    false
                };
                let mut p = handle.borrow_mut();
                if p.conditions.len() >= 128 {
                    return Err(unsupported("KAG conditional depth exceeded"));
                }
                p.conditions.push((exclude, active));
                if exclude == -1 && !active {
                    p.exclude = p.conditions.len() as i32;
                }
                tag.advance(&mut p.position);
                return Ok(false);
            }
            "elsif" | "else" => {
                let state = {
                    let p = handle.borrow();
                    p.conditions
                        .last()
                        .map(|(_, v)| (*v, p.exclude == p.conditions.len() as i32))
                };
                if let Some((executed, level)) = state {
                    if executed {
                        let mut p = handle.borrow_mut();
                        p.exclude = p.conditions.len() as i32;
                    } else if level {
                        let active = if kind == "else" {
                            true
                        } else {
                            let exp = handle.borrow().args.get(vm, "exp")?;
                            ensure!(!exp.text().is_empty(), "missing KAG conditional expression");
                            self.kag_eval(vm, id, &exp, budget)?.truth()?
                        };
                        if active {
                            let mut p = handle.borrow_mut();
                            p.conditions.last_mut().unwrap().1 = true;
                            p.exclude = -1;
                        }
                    }
                }
                tag.advance(&mut handle.borrow_mut().position);
                return Ok(false);
            }
            "endif" | "endignore" => {
                let mut p = handle.borrow_mut();
                if let Some((level, _)) = p.conditions.pop() {
                    p.exclude = level;
                }
                tag.advance(&mut p.position);
                return Ok(false);
            }
            _ => {}
        }
        if !condition || handle.borrow().exclude != -1 {
            tag.advance(&mut handle.borrow_mut().position);
            return Ok(false);
        }
        let macro_value = {
            let p = handle.borrow();
            vm.get_member(&p.macros, &Value::string(&tag.name), true)?
        };
        let special = matches!(
            kind,
            "macro"
                | "endmacro"
                | "macropop"
                | "erasemacro"
                | "pmacro"
                | "erasepmacro"
                | "emb"
                | "jump"
                | "call"
                | "return"
        );
        if kind == "emb" || (!special && !matches!(macro_value, Value::Void)) {
            let is_macro = !matches!(macro_value, Value::Void);
            let text = if is_macro {
                utf16(&macro_value)
            } else {
                let exp = handle.borrow().args.get(vm, "exp")?;
                ensure!(!exp.text().is_empty(), "missing KAG embedded expression");
                let text = utf16(&self.kag_eval(vm, id, &exp, budget)?);
                let escape = handle.borrow().args.get(vm, "escape")?;
                if matches!(escape, Value::Void) || escape.truth()? {
                    text.into_iter()
                        .flat_map(|c| if c == 91 { vec![91, 91] } else { vec![c] })
                        .collect()
                } else {
                    text
                }
            };
            let mut p = handle.borrow_mut();
            let source = p
                .source
                .as_ref()
                .context("KAG scenario cleared during expansion")?;
            let line = p.position.line(source);
            ensure!(
                tag.start <= line.len(),
                "KAG position changed during expansion"
            );
            let end = tag.delimiter + usize::from(!tag.line_command);
            let mut buffer = line[..tag.start].to_vec();
            buffer.extend(text);
            buffer.extend_from_slice(&line[end.min(line.len())..]);
            if tag.line_command && !p.ignore_cr {
                buffer.push(92);
            }
            if buffer.len() > 8 * 1024 * 1024 {
                return Err(unsupported("KAG expansion exceeds text limit"));
            }
            p.position.buffer = Some(buffer);
            p.position.pos = tag.start;
            if is_macro {
                if p.macro_args.len() >= 128 {
                    return Err(unsupported("KAG macro depth exceeded"));
                }
                let copy = p.args.duplicate(vm, budget)?;
                p.macro_args.push(copy);
            }
            return Ok(false);
        }
        match kind {
            "jump" | "call" | "return" => {
                let (storage, target, args) = {
                    let p = handle.borrow();
                    (
                        p.args.get(vm, "storage")?.text(),
                        p.args.get(vm, "target")?.text(),
                        p.args.dict.clone(),
                    )
                };
                let event = match kind {
                    "jump" => "onJump",
                    "call" => "onCall",
                    _ => "onReturn",
                };
                let result = self.kag_event(vm, id, event, &[args], budget)?;
                if result.map(|v| v.truth()).transpose()?.unwrap_or(true) {
                    if kind == "call" {
                        let mut p = handle.borrow_mut();
                        tag.advance(&mut p.position);
                        p.push_call()?;
                    }
                    if kind == "return" {
                        self.kag_return(vm, id, &storage, &target, budget)?;
                    } else {
                        self.kag_goto(vm, id, &storage, &target, budget)?;
                    }
                    return Ok(false);
                }
            }
            "macro" => {
                let mut p = handle.borrow_mut();
                let name = p.args.get(vm, "name")?.text().to_lowercase();
                ensure!(!name.is_empty(), "missing KAG macro name");
                p.recording = Some((name, Vec::new()));
            }
            "endmacro" => anyhow::bail!("KAG endmacro without macro"),
            "macropop" => {
                ensure!(
                    handle.borrow_mut().macro_args.pop().is_some(),
                    "KAG macro argument stack underflow"
                );
            }
            "erasemacro" | "erasepmacro" => {
                let p = handle.borrow();
                let name = p.args.get(vm, "name")?;
                let dict = if kind == "erasemacro" {
                    &p.macros
                } else {
                    &p.param_macros
                };
                ensure!(
                    vm.delete_member(dict, &name)?.truth()?,
                    "unknown KAG macro name: {}",
                    name.text()
                );
            }
            "pmacro" => {
                let p = handle.borrow();
                let name = p.args.get(vm, "name")?;
                let mut entries = Vec::new();
                for key in &p.args.names {
                    if key != "name" && key != "tagname" {
                        entries.push(Value::string(key));
                        entries.push(p.args.get(vm, key)?);
                    }
                }
                let array = vm.new_native_array(entries)?;
                vm.set_member(&p.param_macros, &name, array)?;
            }
            _ => {
                tag.advance(&mut handle.borrow_mut().position);
                return Ok(true);
            }
        }
        tag.advance(&mut handle.borrow_mut().position);
        Ok(false)
    }
}

mod state;

impl Parser {
    pub(crate) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.extend([
            self.macros.clone(),
            self.param_macros.clone(),
            self.args.dict.clone(),
            self.args.list.clone(),
        ]);
        for args in &self.macro_args {
            out.extend([args.dict.clone(), args.list.clone()]);
        }
    }
}
