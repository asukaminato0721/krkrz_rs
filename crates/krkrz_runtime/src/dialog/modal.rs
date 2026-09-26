use super::*;

/// A portable modal text form. None from the host means Cancel.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputDialog {
    pub title: String,
    pub fields: Vec<InputField>,
    pub accept_label: String,
    pub cancel_label: String,
    pub position: Option<[i32; 2]>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputField {
    pub id: i64,
    pub label: String,
    pub text: String,
    pub multiline: bool,
    /// Some marks an editable combo box, including an empty suggestion list.
    pub choices: Option<Vec<String>>,
    pub max_length: usize,
    pub selection: Option<[i32; 2]>,
    pub focused: bool,
    #[serde(default)]
    pub range: Option<[i32; 2]>,
    #[serde(default)]
    pub checkbox: bool,
}
pub(super) type InputHandler = Box<dyn FnMut(&InputDialog) -> Result<Option<Vec<String>>>>;
#[derive(Clone, Default)]
struct ControlOptions {
    choices: Vec<String>,
    max_length: Option<usize>,
    selection: Option<[i32; 2]>,
}

#[derive(Clone)]
pub(super) struct Dialog {
    pub modeless: bool,
    pub template: Vec<u8>,
    pub header: Template,
    pub items: Vec<Template>,
    pub owner: Value,
    pub(super) active: bool,
    pub(super) result: Option<i64>,
    pub(super) controls: Vec<Template>,
    options: BTreeMap<i64, ControlOptions>,
    focused: Option<i64>,
    position: Option<[i32; 2]>,
}
impl Default for Dialog {
    fn default() -> Self {
        Self {
            modeless: false,
            template: Vec::new(),
            header: Template::default(),
            items: Vec::new(),
            owner: Value::NULL,
            active: false,
            result: None,
            controls: Vec::new(),
            options: BTreeMap::new(),
            focused: None,
            position: None,
        }
    }
}
impl Dialog {
    pub(super) fn new(owner: Value) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }

    fn input(&self) -> Result<(Vec<usize>, InputDialog)> {
        let mut indices = Vec::new();
        let mut fields = Vec::new();
        let mut labels = Vec::new();
        let mut accept = None;
        let mut cancel = None;
        for (index, item) in self.controls.iter().enumerate() {
            let style = item.number("style") as u32;
            ensure!(
                style & 0x18000000 == 0x10000000,
                unsupported("WIN32Dialog input controls must be visible and enabled")
            );
            let class = control_class(item);
            match class {
                130 if style & 0x1f <= 2 => labels.push(text(item, "title")),
                129 | 133 => {
                    ensure!(
                        (class == 129 && style & 0xffff & !0x10c4 == 0)
                            || (class == 133 && style & 0xffff == 2),
                        unsupported(format!("WIN32Dialog text control style {style:#x}"))
                    );
                    let control_id = item.number("id");
                    let options = self.options.get(&control_id).cloned().unwrap_or_default();
                    fields.push(InputField {
                        id: control_id,
                        label: labels.join("\n"),
                        text: text(item, "title"),
                        multiline: class == 129 && style & 4 != 0,
                        choices: (class == 133).then_some(options.choices),
                        max_length: options.max_length.unwrap_or(1_000_000),
                        selection: options.selection,
                        focused: self.focused == Some(control_id),
                        range: None,
                        checkbox: false,
                    });
                    labels.clear();
                    indices.push(index);
                }
                128 if style & 0xf <= 1 => match item.number("id") {
                    1 if accept.is_none() => accept = Some(text(item, "title")),
                    2 if cancel.is_none() => cancel = Some(text(item, "title")),
                    _ => {
                        return Err(unsupported(
                            "WIN32Dialog input requires OK and Cancel buttons",
                        ));
                    }
                },
                _ => {
                    return Err(unsupported(format!(
                        "WIN32Dialog control class {class}: supported input controls are labels, edits, editable combos and OK/Cancel buttons"
                    )));
                }
            }
        }
        ensure!(
            !fields.is_empty() && labels.is_empty(),
            unsupported("WIN32Dialog input requires labelled edit controls")
        );
        Ok((
            indices,
            InputDialog {
                title: text(&self.header, "title"),
                fields,
                accept_label: accept
                    .ok_or_else(|| unsupported("WIN32Dialog input requires an OK button"))?,
                cancel_label: cancel
                    .ok_or_else(|| unsupported("WIN32Dialog input requires a Cancel button"))?,
                position: self.position,
            },
        ))
    }
}
fn control_class(item: &Template) -> i64 {
    match item.fields.get("windowClass") {
        Some(Value::Integer(v)) => *v,
        Some(Value::String(v)) => match String::from_utf16_lossy(v).to_lowercase().as_str() {
            "button" => 128,
            "edit" => 129,
            "static" => 130,
            "combobox" => 133,
            _ => -1,
        },
        _ => -1,
    }
}

fn text(template: &Template, field: &str) -> String {
    String::from_utf16_lossy(&template.text(field))
}

impl Services {
    /// Install the native host's modal input UI, or a deterministic replay handler.
    pub fn set_input_dialog_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&InputDialog) -> Result<Option<Vec<String>>> + 'static,
    {
        self.dialogs.input_handler = Some(Box::new(handler));
    }

    pub(crate) fn show_input_dialog(
        &mut self,
        request: &InputDialog,
    ) -> Result<Option<Vec<String>>> {
        let handler = self.dialogs.input_handler.as_mut().ok_or_else(|| {
            unsupported("native input dialog requires a host UI or replay handler")
        })?;
        handler(request)
    }

    fn modal(&self, object_id: usize) -> Result<&Dialog> {
        match self.dialogs.objects.get(&(object_id, "WIN32Dialog".into())) {
            Some(Object::Dialog(dialog)) => Ok(dialog),
            _ => bail!("WIN32Dialog invalidated during callback"),
        }
    }
    fn modal_mut(&mut self, object_id: usize) -> Result<&mut Dialog> {
        match self
            .dialogs
            .objects
            .get_mut(&(object_id, "WIN32Dialog".into()))
        {
            Some(Object::Dialog(dialog)) => Ok(dialog),
            _ => bail!("WIN32Dialog invalidated during callback"),
        }
    }
    fn dialog_event(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        name: &str,
        msg: i64,
        wp: i64,
        budget: &mut u64,
    ) -> Result<bool> {
        let owner = self.modal(id(context)?)?.owner.clone();
        let target = if owner == Value::NULL {
            context
        } else {
            &owner
        };
        let function = vm.get_property(target, &Value::string(name), true, false, self, budget)?;
        if matches!(function, Value::Void) {
            return Ok(false);
        }
        vm.call_function(
            &function,
            target,
            &[Value::Integer(msg), Value::Integer(wp), Value::Integer(0)],
            self,
            budget,
        )?
        .truth()
    }
    fn open_input_dialog(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let object_id = id(context)?;
        ensure!(
            self.dialogs.input_handler.is_some(),
            unsupported("WIN32Dialog.open requires a native input dialog host")
        );
        let dialog = self.modal_mut(object_id)?;
        ensure!(
            !dialog.modeless,
            unsupported("WIN32Dialog modeless presentation")
        );
        ensure!(!dialog.active, "WIN32Dialog is already open");
        dialog.controls.clone_from(&dialog.items);
        dialog.options.clear();
        dialog.focused = None;
        dialog.position = None;
        dialog.input()?; // Reject unsupported templates before running callbacks.
        dialog.active = true;
        dialog.result = None;
        let result = (|| {
            self.dialog_event(vm, context, "onInit", 0x110, 0, budget)?;
            loop {
                charge(budget, 1)?;
                if let Some(result) = self.modal(object_id)?.result {
                    return Ok(Value::Integer(result));
                }
                let (indices, request) = self.modal(object_id)?.input()?;
                let answer = self.dialogs.input_handler.as_mut().unwrap()(&request)?;
                let command = if let Some(answer) = answer {
                    ensure!(
                        answer.len() == indices.len(),
                        "dialog host returned the wrong number of fields"
                    );
                    let mut values = Vec::new();
                    for (answer, field) in answer.into_iter().zip(&request.fields) {
                        ensure!(
                            answer.encode_utf16().count() <= 1_000_000,
                            "dialog input exceeds one million UTF-16 units"
                        );
                        let text = if field.multiline {
                            answer
                                .replace("\r\n", "\n")
                                .replace('\r', "\n")
                                .replace('\n', "\r\n")
                        } else {
                            answer
                        };
                        charge(budget, text.encode_utf16().count() as u64)?;
                        values.push(Value::string(&text));
                    }
                    for (index, value) in indices.into_iter().zip(values) {
                        self.modal_mut(object_id)?.controls[index]
                            .fields
                            .insert("title".into(), value);
                    }
                    1
                } else {
                    2
                };
                let handled =
                    self.dialog_event(vm, context, "onCommand", 0x111, command, budget)?;
                let dialog = self.modal_mut(object_id)?;
                if !handled && dialog.result.is_none() {
                    dialog.result = Some(command);
                }
            }
        })();
        // Both script errors and host failures leave the dialog closed.
        if let Ok(dialog) = self.modal_mut(object_id) {
            dialog.active = false;
            dialog.controls.clear();
            dialog.options.clear();
        }
        result
    }

    pub(super) fn modal_dialog_call(
        &mut self,
        vm: &mut Vm,
        method: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        if method == "open" {
            return self.open_input_dialog(vm, context, budget).map(Some);
        }
        let object_id = id(context)?;
        if !self.modal(object_id)?.active {
            return Ok(None);
        }
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("WIN32Dialog.{method}: missing argument {i}"))
        };
        let result = match method {
            "get:isValid" => Value::Integer(self.modal(object_id)?.result.is_none().into()),
            "set:modeless" => bail!("Dialog is opened."),
            "makeTemplate" => bail!("Dialog is opened."),
            // Geometry is a portable estimate using 8x16 base font units; the
            // native UI lays out actual glyphs and controls using its own font.
            "get:width" => Value::Integer(self.modal(object_id)?.header.number("cx") * 2),
            "get:height" => Value::Integer(self.modal(object_id)?.header.number("cy") * 2),
            "get:left" | "get:top" => Value::Integer(i64::from(
                self.modal(object_id)?.position.unwrap_or([0, 0])[usize::from(method == "get:top")],
            )),
            "setPos" => {
                self.modal_mut(object_id)?.position =
                    Some([arg(0)?.integer()? as i32, arg(1)?.integer()? as i32]);
                Value::Void
            }
            "close" => {
                self.modal_mut(object_id)?.result = Some(arg(0)?.integer()?);
                Value::Void
            }
            "getItemText" | "setItemText" | "sendItemMessage" | "setItemFocus" => {
                let control_id = arg(0)?.integer()?;
                let index = self
                    .modal(object_id)?
                    .controls
                    .iter()
                    .position(|c| c.number("id") == control_id)
                    .context("dialog control not found")?;
                match method {
                    "setItemFocus" => {
                        self.modal_mut(object_id)?.focused = Some(control_id);
                        Value::Void
                    }
                    "getItemText" => {
                        Value::string(&text(&self.modal(object_id)?.controls[index], "title"))
                    }
                    "setItemText" => {
                        let value = arg(1)?.unary("string")?;
                        let Value::String(units) = &value else {
                            unreachable!()
                        };
                        ensure!(
                            units.len() <= 1_000_000,
                            "dialog input exceeds one million UTF-16 units"
                        );
                        charge(budget, units.len() as u64)?;
                        self.modal_mut(object_id)?.controls[index]
                            .fields
                            .insert("title".into(), value);
                        Value::Void
                    }
                    _ => {
                        let message = arg(1)?.integer()?;
                        let class = control_class(&self.modal(object_id)?.controls[index]);
                        match (class, message) {
                            (128, 0xf0 | 0xf2) => Value::Integer(0),
                            (133, 0x143) => {
                                // CB_ADDSTRING
                                let value = arg(3)?.text();
                                charge(budget, value.encode_utf16().count() as u64 + 1)?;
                                let options = self
                                    .modal_mut(object_id)?
                                    .options
                                    .entry(control_id)
                                    .or_default();
                                ensure!(
                                    options.choices.len() < 100_000,
                                    "dialog combo item limit exceeded"
                                );
                                options.choices.push(value);
                                Value::Integer(options.choices.len() as i64 - 1)
                            }
                            (133, 0x141) | (129, 0xc5) => {
                                // CB_LIMITTEXT / EM_LIMITTEXT
                                let limit = arg(2)?.integer()? as u32 as usize;
                                self.modal_mut(object_id)?
                                    .options
                                    .entry(control_id)
                                    .or_default()
                                    .max_length = Some(if limit == 0 {
                                    1_000_000
                                } else {
                                    limit.min(1_000_000)
                                });
                                Value::Integer(0)
                            }
                            (129, 0xb1) => {
                                // EM_SETSEL
                                self.modal_mut(object_id)?
                                    .options
                                    .entry(control_id)
                                    .or_default()
                                    .selection =
                                    Some([arg(2)?.integer()? as i32, arg(3)?.integer()? as i32]);
                                Value::Integer(0)
                            }
                            _ => {
                                return Err(unsupported(format!(
                                    "WIN32Dialog.sendItemMessage({message:#x}) for class {class}"
                                )));
                            }
                        }
                    }
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }
}
