//! Native saveStruct and the older saveStruct plugin shipped in PackinOne.
use crate::{Host, ObjectRef, Value, Vm, object::ObjectKind, unsupported};
use anyhow::{Context, Result, ensure};

impl Vm {
    pub(crate) fn register_serialization(&mut self, plugin: bool) -> Result<()> {
        for class in ["Array", "Dictionary"] {
            let receiver = self.globals[class].clone();
            let methods: &[&str] = if plugin {
                if class == "Array" {
                    &["save2", "saveStruct2", "toStructString"]
                } else {
                    &["saveStruct2", "toStructString"]
                }
            } else if class == "Array" {
                &["save", "load", "saveStruct"]
            } else {
                &["saveStruct"]
            };
            for method in methods {
                let value = self.allocate(ObjectKind::Native(format!(
                    "Serialization.{class}.{method}"
                )))?;
                let id = self.object_id(&value)?;
                self.objects[id].native_static = class == "Dictionary";
                self.set_member(&receiver, &Value::string(method), value)?;
                let id = self.object_id(&receiver)?;
                self.objects[id].member_flags.insert(
                    method.encode_utf16().collect(),
                    0x1000
                        | if class == "Dictionary" {
                            crate::scripts_ex::STATIC
                        } else {
                            0
                        },
                );
            }
        }
        Ok(())
    }
    pub fn register_save_struct(&mut self) -> Result<()> {
        self.register_serialization(true)
    }

    pub(crate) fn serialization_call(
        &mut self,
        name: &str,
        receiver: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
        needed: bool,
    ) -> Result<Value> {
        let (class, method) = name
            .split_once('.')
            .context("invalid serialization method")?;
        if method == "toStructString" && !needed {
            return Ok(Value::Void);
        }
        let id = self.object_id(receiver)?;
        let array = matches!(self.objects[id].kind, ObjectKind::Array(_));
        let core = matches!(method, "save" | "load" | "saveStruct");
        if class == "Array" {
            ensure!(array, "Array native instance required");
        } else if core {
            ensure!(
                matches!(self.objects[id].kind, ObjectKind::Dictionary),
                "Dictionary native instance required"
            );
        }
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("{name}: missing argument {i}"))
        };
        let result = Value::Object(ObjectRef {
            object: Some(id),
            context: Some(id),
        });
        if method == "load" {
            let mode = args.get(1).cloned().unwrap_or(Value::Void);
            let content = host.call(self, "TextStream.read", &[arg(0)?.clone(), mode], budget)?;
            let Value::String(content) = content else {
                anyhow::bail!("text reader returned a non-string");
            };
            let end = content
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(content.len());
            let mut items = vec![];
            let (mut start, mut i) = (0, 0);
            while i < end {
                if content[i] == 13 || content[i] == 10 {
                    items.push(Value::String(content[start..i].to_vec()));
                    let cr = content[i] == 13;
                    i += 1;
                    if cr && content.get(i) == Some(&10) {
                        i += 1;
                    }
                    start = i;
                } else {
                    i += 1;
                }
            }
            if start < end {
                items.push(Value::String(content[start..end].to_vec()));
            }
            self.objects[id].kind = ObjectKind::Array(items);
            return Ok(result);
        }
        let newline_arg = if method == "toStructString" { 0 } else { 2 };
        let lf = !core
            && args
                .get(newline_arg)
                .map(Value::integer)
                .transpose()?
                .unwrap_or(0)
                != 0;
        let newline = if lf { "\n" } else { "\r\n" };
        let mut writer = Writer {
            units: vec![],
            stack: vec![],
            core,
            newline,
            budget,
        };
        if method == "save" || method == "save2" {
            let ObjectKind::Array(items) = &self.objects[id].kind else {
                unreachable!()
            };
            for item in items {
                writer.charge()?;
                if let Value::String(units) = item {
                    writer.units.extend_from_slice(units);
                } else if core {
                    if matches!(item, Value::Integer(_) | Value::Real(_)) {
                        writer.text(&item.text());
                    }
                } else {
                    ensure!(
                        matches!(item, Value::Void),
                        "save2 requires string elements"
                    );
                }
                writer.text(newline);
            }
        } else {
            writer.object(self, id, 0, class == "Array")?;
        }
        writer.check_limit()?;
        let units = writer.units;
        if method == "toStructString" {
            return Ok(Value::String(units));
        }
        let mode = if core {
            args.get(1).cloned().unwrap_or(Value::Void)
        } else {
            Value::string(
                if args.get(1).map(Value::integer).transpose()?.unwrap_or(0) != 0 {
                    "utf8"
                } else {
                    "cp932"
                },
            )
        };
        host.call(
            self,
            if core {
                "TextStream.write"
            } else {
                "TextStream.writePlugin"
            },
            &[arg(0)?.clone(), Value::String(units), mode],
            budget,
        )?;
        Ok(if core { result } else { Value::Void })
    }
}

struct Writer<'a> {
    units: Vec<u16>,
    stack: Vec<usize>,
    core: bool,
    newline: &'static str,
    budget: &'a mut u64,
}
impl Writer<'_> {
    fn check_limit(&self) -> Result<()> {
        if self.units.len() > 32 << 20 {
            return Err(unsupported("serialization output exceeds 64 MiB"));
        }
        Ok(())
    }
    fn charge(&mut self) -> Result<()> {
        if *self.budget == 0 || self.stack.len() >= 128 || self.units.len() > 32 << 20 {
            return Err(unsupported(
                "serialization budget, nesting, or output limit exceeded",
            ));
        }
        *self.budget -= 1;
        Ok(())
    }
    fn text(&mut self, text: &str) {
        self.units.extend(text.encode_utf16());
    }
    fn quote(&mut self, units: &[u16], escaped: bool) {
        self.text("\"");
        let mut hex = false;
        for &unit in units.iter().take_while(|unit| **unit != 0) {
            let escape = match unit {
                34 => Some("\\\""),
                92 => Some("\\\\"),
                7 if escaped => Some("\\a"),
                8 if escaped => Some("\\b"),
                12 if escaped => Some("\\f"),
                10 if escaped => Some("\\n"),
                13 if escaped => Some("\\r"),
                9 if escaped => Some("\\t"),
                11 if escaped => Some("\\v"),
                39 if escaped => Some("\\'"),
                _ => None,
            };
            if let Some(escape) = escape {
                self.text(escape);
                hex = false;
            } else if escaped
                && (unit < 32
                    || hex && char::from_u32(unit.into()).is_some_and(|v| v.is_ascii_hexdigit()))
            {
                self.text(&format!("\\x{unit:02x}"));
                hex = true;
            } else {
                self.units.push(unit);
                hex = false;
            }
        }
        self.text("\"");
    }
    fn value(&mut self, vm: &Vm, value: &Value, indent: usize) -> Result<()> {
        self.charge()?;
        match value {
            Value::Void => self.text("void"),
            Value::Integer(value) => {
                if !self.core {
                    self.text("int ");
                }
                self.text(&value.to_string());
            }
            Value::Real(value) => {
                if !self.core {
                    self.text("real ");
                }
                let text = crate::value::real_text(*value);
                let hex = if !value.is_finite() || *value == 0.0 {
                    text.clone()
                } else {
                    format!(
                        "{}0x1.{:013X}p{}",
                        if value.is_sign_negative() { "-" } else { "" },
                        value.to_bits() & ((1u64 << 52) - 1),
                        ((value.to_bits() >> 52) & 0x7ff) as i32 - 1023
                    )
                };
                self.text(&format!("{hex} /* {text} */"));
            }
            Value::String(units) => self.quote(units, self.core),
            Value::Octet(bytes) => {
                self.text("<% ");
                for (i, byte) in bytes.iter().enumerate() {
                    self.charge()?;
                    if self.core && i > 0 {
                        self.text(" ");
                    }
                    self.text(&if self.core {
                        format!("{byte:02X}")
                    } else {
                        format!("{byte:02x} ")
                    });
                }
                self.text(if self.core { " %>" } else { "%>" });
            }
            Value::Object(reference) => {
                let id = if self.core {
                    reference.context.or(reference.object)
                } else {
                    reference.object
                };
                if let Some(id) = id {
                    if self.core
                        && !matches!(
                            vm.objects[id].kind,
                            ObjectKind::Array(_) | ObjectKind::Dictionary
                        )
                    {
                        // Native saves discard non-container object identities. Pointer
                        // comments are diagnostic and cannot be portable across engines.
                        self.text("null /* non-container object */");
                    } else {
                        self.object(
                            vm,
                            id,
                            indent,
                            matches!(vm.objects[id].kind, ObjectKind::Array(_)),
                        )?;
                    }
                } else {
                    self.text("null");
                }
            }
        }
        Ok(())
    }
    fn object(&mut self, vm: &Vm, id: usize, indent: usize, array: bool) -> Result<()> {
        self.charge()?;
        if self.stack.contains(&id) {
            if self.core {
                self.text("null /* object recursion detected */");
                return Ok(());
            }
            return Err(unsupported(
                "cyclic object in saveStruct plugin serialization",
            ));
        }
        self.stack.push(id);
        if self.core {
            self.text("(const) ");
        }
        self.text(if array { "[" } else { "%[" });
        if self.core {
            self.text(self.newline);
        }
        let entries = if array {
            let ObjectKind::Array(items) = &vm.objects[id].kind else {
                unreachable!()
            };
            items
                .iter()
                .map(|item| (None, item.clone()))
                .collect::<Vec<_>>()
        } else {
            ensure!(
                id != 0,
                "global-object serialization requires complete global enumeration"
            );
            vm.objects[id]
                .member_layout
                .keys()
                .into_iter()
                .filter(|key| {
                    vm.objects[id].member_flags.get(key).copied().unwrap_or(0) & 0x1000 == 0
                })
                .map(|key| (Some(key.clone()), vm.objects[id].members[&key].clone()))
                .collect()
        };
        for (i, (key, value)) in entries.iter().enumerate() {
            if i > 0 {
                self.text(",");
                if self.core || !array {
                    self.text(self.newline);
                }
            }
            if self.core {
                self.text(&" ".repeat(indent + 1));
            }
            if let Some(key) = key {
                self.quote(key, self.core);
                self.text(if self.core { " => " } else { "=>" });
            }
            self.value(vm, value, indent + 1)?;
        }
        if self.core {
            if !entries.is_empty() {
                self.text(self.newline);
            }
            self.text(&" ".repeat(indent));
        }
        self.text("]");
        self.stack.pop();
        Ok(())
    }
}
