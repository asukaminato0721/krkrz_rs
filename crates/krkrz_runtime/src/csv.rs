//! CSVParser plugin behavior from the historical Go Watanabe implementation.
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};

pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("CSVParser")?;
    for method in [
        "CSVParser",
        "init",
        "initStorage",
        "getNextLine",
        "parse",
        "parseStorage",
    ] {
        vm.register_native(&format!("CSVParser.{method}"))?;
    }
    vm.register_native_property(
        &class,
        "currentLineNumber",
        Some("CSVParser.currentLineNumber"),
        None,
    )?;
    Ok(())
}

pub(crate) struct Parser {
    pub target: Value,
    separator: u16,
    newline: Vec<u16>,
    input: Option<Input>,
    pos: usize,
    pub line_number: i64,
}
enum Input {
    Text(Vec<u16>),
    Bytes { data: Vec<u8>, utf8: bool },
}
impl Default for Parser {
    fn default() -> Self {
        Self {
            target: Value::NULL,
            separator: 44,
            newline: vec![13, 10],
            input: None,
            pos: 0,
            line_number: 0,
        }
    }
}
impl Parser {
    pub fn construct(&mut self, args: &[Value]) -> Result<()> {
        if let Some(target) = args.first() {
            ensure!(
                matches!(target, Value::Object(_)),
                "CSVParser target must be an object"
            );
            self.target = target.clone();
        }
        if let Some(separator) = args.get(1) {
            self.separator = separator.integer()? as u16;
        }
        if let Some(newline) = args.get(2) {
            self.newline = units(newline);
        }
        Ok(())
    }
    pub fn init(&mut self, input: Vec<u16>) {
        self.input = Some(Input::Text(input));
        self.pos = 0;
        self.line_number = 0;
    }
    fn init_bytes(&mut self, data: Vec<u8>, utf8: bool) {
        self.input = Some(Input::Bytes { data, utf8 });
        self.pos = 0;
        self.line_number = 0;
    }
    fn add_line(&mut self, line: &mut Vec<u16>, budget: &mut u64) -> Result<bool> {
        charge(budget)?;
        let Some(input) = &self.input else {
            return Ok(false);
        };
        if let Input::Bytes { data, utf8 } = input {
            let start = self.pos;
            while self.pos < data.len() && ![10, 13, 255].contains(&data[self.pos]) {
                charge(budget)?;
                self.pos += 1;
            }
            let end = self.pos;
            let terminator = data.get(self.pos).copied();
            if terminator.is_some() {
                self.pos += 1;
            }
            if terminator == Some(13)
                && self.pos < data.len()
                && (data[self.pos] == 10
                    || self.pos + 1 == data.len() && !data.len().is_multiple_of(8192))
            {
                self.pos += 1;
            }
            if end == start && (terminator.is_none() || terminator == Some(255)) {
                return Ok(false);
            }
            let text = if *utf8 {
                std::str::from_utf8(&data[start..end])?.into()
            } else {
                let (text, errors) =
                    encoding_rs::SHIFT_JIS.decode_without_bom_handling(&data[start..end]);
                if errors {
                    return Err(unsupported(
                        "CSVParser malformed CP932 replacement rules are not implemented",
                    ));
                }
                text
            };
            // MultiByteToWideChar output is appended as a NUL-terminated string.
            line.extend(text.split('\0').next().unwrap_or("").encode_utf16());
            return Ok(true);
        }
        let Input::Text(input) = input else {
            unreachable!()
        };
        if self.pos == input.len() {
            return Ok(false);
        }
        while let Some(&unit) = input.get(self.pos) {
            charge(budget)?;
            self.pos += 1;
            if unit == 13 {
                // The original string reader drops a final non-LF unit after CR.
                if self.pos < input.len() && (input[self.pos] == 10 || self.pos + 1 == input.len())
                {
                    self.pos += 1;
                }
                break;
            }
            if unit == 10 {
                break;
            }
            line.push(unit);
        }
        Ok(true)
    }
    pub fn next(&mut self, budget: &mut u64) -> Result<Option<Vec<Value>>> {
        charge(budget)?;
        let mut line = vec![];
        if !self.add_line(&mut line, budget)? {
            self.input = None;
            return Ok(None);
        }
        self.line_number += 1;
        let mut fields = vec![];
        if line.is_empty() {
            return Ok(Some(fields));
        }
        let mut i = 0;
        loop {
            let mut field = vec![];
            if line.get(i) == Some(&34) {
                i += 1;
                'quoted: loop {
                    while i < line.len() {
                        charge(budget)?;
                        let unit = line[i];
                        i += 1;
                        if unit == 34 {
                            if line.get(i) != Some(&34) {
                                while i < line.len() && line[i] != self.separator {
                                    charge(budget)?;
                                    field.push(line[i]);
                                    i += 1;
                                }
                                break 'quoted;
                            }
                            i += 1;
                        }
                        field.push(unit);
                    }
                    if field.len().saturating_add(self.newline.len()) > 32 << 20 {
                        return Err(unsupported("CSVParser field exceeds 64 MiB limit"));
                    }
                    field.extend_from_slice(&self.newline);
                    if !self.add_line(&mut line, budget)? {
                        break;
                    }
                }
            } else {
                while i < line.len() && line[i] != self.separator {
                    charge(budget)?;
                    field.push(line[i]);
                    i += 1;
                }
            }
            fields.push(Value::String(field));
            if i >= line.len() {
                break;
            }
            i += 1;
        }
        Ok(Some(fields))
    }
}
pub(crate) fn units(value: &Value) -> Vec<u16> {
    if let Value::String(value) = value {
        value.clone()
    } else {
        value.text().encode_utf16().collect()
    }
}
fn charge(budget: &mut u64) -> Result<()> {
    if *budget == 0 {
        return Err(unsupported("CSVParser execution budget exhausted"));
    }
    *budget -= 1;
    Ok(())
}

impl crate::Services {
    pub(crate) fn csv_call(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("CSVParser requires an object context");
        };
        let id = reference
            .object
            .context("CSVParser requires a non-null context")?;
        if name == "@initialize" {
            self.csv_parsers.entry(id).or_default();
            return Ok(Value::Void);
        }
        if name == "@invalidate" {
            self.csv_parsers.remove(&id);
            return Ok(Value::Void);
        }
        let parser = self
            .csv_parsers
            .get_mut(&id)
            .context("context has no CSVParser native instance")?;
        match name {
            "CSVParser" => {
                parser.construct(args)?;
                return Ok(Value::Void);
            }
            "currentLineNumber" => return Ok(Value::Integer(parser.line_number)),
            "getNextLine" => {
                return parser
                    .next(budget)?
                    .map(|items| vm.new_array(items))
                    .transpose()
                    .map(|v| v.unwrap_or(Value::Void));
            }
            _ => (),
        }
        let storage = matches!(name, "initStorage" | "parseStorage");
        let parse = matches!(name, "parse" | "parseStorage");
        ensure!(
            parse || matches!(name, "init" | "initStorage"),
            "unknown registered CSVParser operation: {name}"
        );
        ensure!(parse || !args.is_empty(), "CSVParser.{name}: missing input");
        if let Some(input) = args.first() {
            if storage {
                self.csv_parsers.get_mut(&id).unwrap().input = None;
                let bytes = self.read_storage(&input.text())?;
                ensure!(bytes.len() <= 64 << 20, "CSV input exceeds 64 MiB limit");
                // Installed PackinOne predates the string text-stream mode.
                let utf8 = args.get(1).map(Value::integer).transpose()?.unwrap_or(0) != 0;
                self.csv_parsers
                    .get_mut(&id)
                    .unwrap()
                    .init_bytes(bytes, utf8);
            } else {
                self.csv_parsers.get_mut(&id).unwrap().init(units(input));
            }
        }
        if parse {
            let target = self.csv_parsers[&id].target.clone();
            let target = if target == Value::NULL {
                context.clone()
            } else {
                target
            };
            let mut method = vm.get_member(&target, &Value::string("doLine"), true)?;
            // The plugin keeps AsObject(), then calls it with the target. An
            // existing bound context on the stored closure is discarded.
            if let Value::Object(reference) = &mut method {
                reference.context = None;
            }
            if !matches!(method, Value::Void) {
                while let Some(fields) = self.csv_parsers.get_mut(&id).unwrap().next(budget)? {
                    let line = Value::Integer(self.csv_parsers[&id].line_number);
                    let fields = vm.new_array(fields)?;
                    vm.call_function(&method, &target, &[fields, line], self, budget)?;
                }
            }
        }
        Ok(Value::Void)
    }
}
