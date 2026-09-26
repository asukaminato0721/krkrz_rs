//! ktsndopt volume dialog API observed in Route of AYANO's Override.tjs.
use crate::{
    Services,
    dialog::{InputDialog, InputField},
};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::Value;
impl Services {
    pub(crate) fn sound_options_call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        match name {
            "ktSndOptDlg_Final" => {
                self.sound_options = None;
                Ok(Value::Void)
            }
            "ktSndOptDlg_GetState" => Ok(Value::Octet(
                self.sound_options
                    .as_ref()
                    .context("sound options dialog is not initialized")?
                    .clone(),
            )),
            _ => {
                ensure!(args.len() == 15, "ktSndOptDlg_Init requires 15 arguments");
                self.sound_options = None;
                let mut state = Vec::with_capacity(9);
                let mut fields = Vec::new();
                for (group, label) in ["Music", "Sound effects", "Voice"].iter().enumerate() {
                    let a = &args[group * 5..group * 5 + 5];
                    let volume = a[1].integer()?;
                    let silence = a[4].integer()?;
                    ensure!(
                        (0..=100).contains(&volume) && (0..=100).contains(&silence),
                        "sound option volumes must be 0..100"
                    );
                    state.extend([volume as u8, u8::from(a[3].truth()?), silence as u8]);
                    for (offset, visible, checkbox, text) in [
                        (0, a[0].truth()?, false, format!("{label} volume")),
                        (
                            1,
                            a[2].truth()?,
                            true,
                            format!("Reduce {label} during voice playback"),
                        ),
                        (
                            2,
                            a[2].truth()?,
                            false,
                            format!("{label} volume during voice playback"),
                        ),
                    ] {
                        if visible {
                            let id = group * 3 + offset;
                            fields.push(InputField {
                                id: id as i64,
                                label: text,
                                text: state[id].to_string(),
                                multiline: false,
                                choices: None,
                                max_length: 3,
                                selection: None,
                                focused: false,
                                range: (!checkbox).then_some([0, 100]),
                                checkbox,
                            });
                        }
                    }
                }
                if !fields.is_empty() {
                    let request = InputDialog {
                        title: "Sound settings".into(),
                        fields,
                        accept_label: "OK".into(),
                        cancel_label: "Cancel".into(),
                        position: None,
                    };
                    let Some(answer) = self.show_input_dialog(&request)? else {
                        return Ok(Value::Integer(0));
                    };
                    ensure!(
                        answer.len() == request.fields.len(),
                        "sound options dialog returned the wrong number of values"
                    );
                    for (field, value) in request.fields.iter().zip(answer) {
                        let value = value.parse::<u8>().context("invalid sound option value")?;
                        ensure!(
                            value <= if field.checkbox { 1 } else { 100 },
                            "sound option value is out of range"
                        );
                        state[field.id as usize] = value;
                    }
                }
                self.sound_options = Some(state);
                Ok(Value::Integer(1))
            }
        }
    }
}
