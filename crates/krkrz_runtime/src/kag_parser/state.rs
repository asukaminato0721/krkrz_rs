//! KAGParserEx's label-based save format and independent parser copies.
use super::*;

fn get(vm: &mut Vm, dict: &Value, key: &str) -> Result<Value> {
    vm.get_member(dict, &Value::string(key), true)
}
fn set(vm: &mut Vm, dict: &Value, key: &str, value: Value) -> Result<()> {
    vm.set_member(dict, &Value::string(key), value)
}
fn number(vm: &mut Vm, dict: &Value, key: &str, limit: usize) -> Result<usize> {
    let n = get(vm, dict, key)?.integer()?;
    ensure!(
        n >= 0 && n as u64 <= limit as u64,
        "invalid KAG saved {key}: {n}"
    );
    Ok(n as usize)
}
fn array(vm: &mut Vm, value: &Value, limit: usize, budget: &mut u64) -> Result<Vec<Value>> {
    let count = number(vm, value, "count", limit)?;
    (0..count)
        .map(|i| {
            charge(budget)?;
            vm.get_member(value, &Value::Integer(i as i64), false)
        })
        .collect()
}
fn save_conditions(
    vm: &mut Vm,
    dict: &Value,
    exclude: i32,
    conditions: &[(i32, bool)],
) -> Result<()> {
    set(vm, dict, "ExcludeLevel", Value::Integer(exclude.into()))?;
    set(vm, dict, "IfLevel", Value::Integer(conditions.len() as i64))?;
    set(
        vm,
        dict,
        "ExcludeLevelStack",
        Value::string(
            &conditions
                .iter()
                .map(|(n, _)| format!("{:08x}", *n as u32))
                .collect::<String>(),
        ),
    )?;
    set(
        vm,
        dict,
        "IfLevelExecutedStack",
        Value::string(
            &conditions
                .iter()
                .map(|(_, b)| if *b { '1' } else { '0' })
                .collect::<String>(),
        ),
    )
}
fn read_conditions(vm: &mut Vm, dict: &Value) -> Result<(i32, Vec<(i32, bool)>)> {
    let exclude = get(vm, dict, "ExcludeLevel")?.integer()? as i32;
    let count = number(vm, dict, "IfLevel", 128)?;
    let ints = get(vm, dict, "ExcludeLevelStack")?.text();
    let bools = get(vm, dict, "IfLevelExecutedStack")?.text();
    ensure!(
        ints.len() == count * 8
            && bools.len() == count
            && ints.bytes().all(|b| b.is_ascii_hexdigit())
            && bools.bytes().all(|b| b == b'0' || b == b'1'),
        "malformed KAG saved conditional stack"
    );
    let mut conditions = Vec::new();
    for (i, b) in bools.bytes().enumerate() {
        // The community implementation advances one character per entry, not
        // eight. Preserve this observable legacy save-file behavior.
        let n = u32::from_str_radix(&ints[i..i + 8], 16)? as i32;
        conditions.push((n, b == b'1'));
    }
    Ok((exclude, conditions))
}
fn save_position(vm: &mut Vm, dict: &Value, position: &Position) -> Result<()> {
    set(
        vm,
        dict,
        "lineBuffer",
        Value::String(position.buffer.clone().unwrap_or_default()),
    )?;
    set(
        vm,
        dict,
        "lineBufferUsing",
        Value::Integer(position.buffer.is_some().into()),
    )
}
impl Parser {
    pub(super) fn store(&self, vm: &mut Vm, budget: &mut u64) -> Result<Value> {
        let result = vm.new_dictionary()?;
        for (key, value) in [
            ("macros", &self.macros),
            ("paramMacros", &self.param_macros),
        ] {
            let copied = vm.new_dictionary()?;
            vm.dictionary_assign(&copied, std::slice::from_ref(value), false, budget)?;
            set(vm, &result, key, copied)?;
        }
        let mut macro_args = Vec::new();
        for args in &self.macro_args {
            let mut flat = Vec::new();
            for name in &args.names {
                charge(budget)?;
                flat.push(Value::string(name));
                flat.push(args.get(vm, name)?);
            }
            macro_args.push(vm.new_native_array(flat)?);
        }
        let value = vm.new_native_array(macro_args)?;
        set(vm, &result, "macroArgs", value)?;
        let mut calls = Vec::new();
        for call in &self.calls {
            charge(budget)?;
            let dict = vm.new_dictionary()?;
            for (key, value) in [
                ("storage", Value::string(&call.storage)),
                ("label", Value::string(&call.label)),
                ("offset", Value::Integer(call.position.line as i64)),
                ("orgLineStr", Value::String(call.original_line.clone())),
                ("pos", Value::Integer(call.position.pos as i64)),
                ("macroArgStackBase", Value::Integer(call.macro_base as i64)),
                (
                    "macroArgStackDepth",
                    Value::Integer(call.macro_depth as i64),
                ),
            ] {
                set(vm, &dict, key, value)?;
            }
            save_position(vm, &dict, &call.position)?;
            save_conditions(vm, &dict, call.exclude, &call.conditions)?;
            calls.push(dict);
        }
        let calls = vm.new_native_array(calls)?;
        set(vm, &result, "callStack", calls)?;
        for (key, value) in [
            ("storageName", Value::string(&self.storage)),
            (
                "storageShortName",
                Value::string(
                    self.storage
                        .rsplit(['/', '\\', '>'])
                        .next()
                        .unwrap_or_default(),
                ),
            ),
            ("curLabel", Value::string(&self.label)),
            ("curLine", Value::Integer(self.position.line as i64)),
            ("curPos", Value::Integer(self.position.pos as i64)),
            ("macroArgStackBase", Value::Integer(self.macro_base as i64)),
            (
                "macroArgStackDepth",
                Value::Integer(self.macro_args.len() as i64),
            ),
        ] {
            set(vm, &result, key, value)?;
        }
        save_position(vm, &result, &self.position)?;
        save_conditions(vm, &result, self.exclude, &self.conditions)?;
        Ok(result)
    }
}
impl Services {
    pub(super) fn kag_restore(
        &mut self,
        vm: &mut Vm,
        id: usize,
        state: &Value,
        budget: &mut u64,
    ) -> Result<()> {
        let handle = self.kag_parsers[&id].clone();
        // Validate sizes before allocating or changing the parser. Saved scripts
        // can supply arbitrary dictionaries, including malformed data.
        let depth = number(vm, state, "macroArgStackDepth", 128)?;
        let base = number(vm, state, "macroArgStackBase", depth)?;
        let saved_args = get(vm, state, "macroArgs")?;
        let mut macro_args = Vec::new();
        if !matches!(saved_args, Value::Void) {
            for args in array(vm, &saved_args, 128, budget)? {
                let flat = array(vm, &args, 100_000, budget)?;
                ensure!(flat.len() % 2 == 0, "malformed KAG saved macro arguments");
                let mut copied = Args::new(vm)?;
                for pair in flat.as_chunks::<2>().0 {
                    copied.add(vm, &pair[0].text(), pair[1].clone())?;
                }
                macro_args.push(copied);
            }
        }
        ensure!(macro_args.len() == depth, "malformed KAG saved macro depth");
        let saved_calls = get(vm, state, "callStack")?;
        let mut calls = Vec::new();
        if !matches!(saved_calls, Value::Void) {
            for dict in array(vm, &saved_calls, 128, budget)? {
                let (exclude, conditions) = read_conditions(vm, &dict)?;
                let macro_depth = number(vm, &dict, "macroArgStackDepth", depth)?;
                let macro_base = number(vm, &dict, "macroArgStackBase", macro_depth)?;
                let line = number(vm, &dict, "offset", 1_000_000)?;
                let pos = number(vm, &dict, "pos", 8 * 1024 * 1024)?;
                let buffer = get(vm, &dict, "lineBufferUsing")?
                    .truth()?
                    .then(|| get(vm, &dict, "lineBuffer").map(|v| utf16(&v)))
                    .transpose()?;
                ensure!(
                    buffer
                        .as_ref()
                        .is_none_or(|b| b.len() <= 8 * 1024 * 1024 && pos <= b.len()),
                    "invalid KAG saved line buffer"
                );
                calls.push(Call {
                    storage: get(vm, &dict, "storage")?.text(),
                    label: get(vm, &dict, "label")?.text(),
                    original_line: utf16(&get(vm, &dict, "orgLineStr")?),
                    position: Position { line, pos, buffer },
                    macro_base,
                    macro_depth,
                    exclude,
                    conditions,
                });
            }
        }
        let (exclude, conditions) = read_conditions(vm, state)?;
        let storage = get(vm, state, "storageName")?;
        let label = get(vm, state, "curLabel")?;
        let (storage, label) = {
            let mut p = handle.borrow_mut();
            for (key, target) in [("macros", &p.macros), ("paramMacros", &p.param_macros)] {
                let source = get(vm, state, key)?;
                if !matches!(source, Value::Void) {
                    vm.dictionary_assign(target, &[source], false, budget)?;
                }
            }
            p.macro_args = macro_args;
            p.macro_base = depth;
            p.calls = calls;
            if !matches!(storage, Value::Void) {
                p.storage = storage.text();
            }
            if !matches!(label, Value::Void) {
                p.label = label.text();
            }
            let pair = (p.storage.clone(), p.label.clone());
            p.clear_buffer();
            pair
        };
        self.kag_load(vm, id, &storage, budget)?;
        let mut p = handle.borrow_mut();
        p.jump(&label)?;
        p.exclude = exclude;
        p.conditions = conditions;
        p.macro_base = base;
        Ok(())
    }
}
