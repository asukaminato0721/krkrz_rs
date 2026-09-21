//! A separate egui process keeps modal script calls independent of the game event loop.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use krkrz_runtime::dialog::InputDialog;
use std::{
    cell::RefCell,
    io::{Read, Write},
    process::{Command, Stdio},
    rc::Rc,
};

const MAX_JSON_BYTES: usize = 16 << 20;

pub fn show(request: &InputDialog) -> Result<Option<Vec<String>>> {
    let bytes = serde_json::to_vec(request)?;
    ensure!(
        bytes.len() <= MAX_JSON_BYTES,
        "dialog request exceeds 16 MiB"
    );
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--native-dialog")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("start egui dialog process")?;
    let written = child.stdin.take().unwrap().write_all(&bytes);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("send egui dialog request");
    }
    let output = child.wait_with_output().context("wait for egui dialog")?;
    ensure!(
        output.status.success(),
        "egui dialog failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ensure!(
        output.stdout.len() <= MAX_JSON_BYTES,
        "dialog result exceeds 16 MiB"
    );
    serde_json::from_slice(&output.stdout).context("read egui dialog result")
}

pub fn run_child() -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_JSON_BYTES,
        "dialog request exceeds 16 MiB"
    );
    let request: InputDialog = serde_json::from_slice(&bytes).context("decode dialog request")?;
    ensure!(
        !request.fields.is_empty() && request.fields.len() <= 64,
        "dialog requires 1..64 text fields"
    );
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    let height = 105.0
        + request
            .fields
            .iter()
            .map(|f| if f.multiline { 180.0 } else { 66.0 })
            .sum::<f32>();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(&request.title)
        .with_inner_size([560.0, height.min(800.0)])
        .with_min_inner_size([360.0, 180.0]);
    if let Some([x, y]) = request.position {
        viewport = viewport.with_position([x as f32, y as f32]);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "krkrz-dialog",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(DialogApp::new(request, output)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("egui dialog: {error}"))?;
    let bytes = serde_json::to_vec(&*result.borrow())?;
    ensure!(
        bytes.len() <= MAX_JSON_BYTES,
        "dialog result exceeds 16 MiB"
    );
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}

fn install_fonts(ctx: &egui::Context) {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    let families = [
        "Noto Sans CJK JP",
        "Noto Sans JP",
        "Source Han Sans JP",
        "Noto Sans CJK SC",
        "WenQuanYi Micro Hei",
        "MS Gothic",
        "Yu Gothic",
    ];
    let names: Vec<_> = families
        .iter()
        .map(|name| fontdb::Family::Name(name))
        .collect();
    if let Some(id) = database.query(&fontdb::Query {
        families: &names,
        ..Default::default()
    }) {
        database.with_face_data(id, |bytes, index| {
            let mut data = egui::FontData::from_owned(bytes.to_vec());
            data.index = index;
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("dialog-cjk".into(), data.into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("dialog-cjk".into());
            }
            ctx.set_fonts(fonts);
        });
    }
}

struct DialogApp {
    request: InputDialog,
    values: Vec<String>,
    result: Rc<RefCell<Option<Vec<String>>>>,
    initialized: bool,
    composing: bool,
}
impl DialogApp {
    fn new(request: InputDialog, result: Rc<RefCell<Option<Vec<String>>>>) -> Self {
        let values = request
            .fields
            .iter()
            .map(|f| f.text.replace("\r\n", "\n"))
            .collect();
        Self {
            request,
            values,
            result,
            initialized: false,
            composing: false,
        }
    }
    fn accept(&self, ctx: &egui::Context) {
        *self.result.borrow_mut() = Some(self.values.clone());
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
impl eframe::App for DialogApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut committed = false;
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    match event {
                        egui::ImeEvent::Preedit { text, .. } => self.composing = !text.is_empty(),
                        egui::ImeEvent::Commit(_) => {
                            self.composing = false;
                            committed = true;
                        }
                        _ => {}
                    }
                }
            }
        });
        let mut submit = false;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 48.0).max(60.0))
                .show(ui, |ui| {
                    for (index, field) in self.request.fields.iter().enumerate() {
                        if !field.label.is_empty() {
                            ui.label(&field.label);
                        }
                        ui.horizontal(|ui| {
                            let width = (ui.available_width()
                                - if field.choices.is_some() { 42.0 } else { 0.0 })
                            .max(40.0);
                            let edit = if field.multiline {
                                egui::TextEdit::multiline(&mut self.values[index]).desired_rows(6)
                            } else {
                                egui::TextEdit::singleline(&mut self.values[index])
                            };
                            let mut output = edit
                                .id_salt(("field", field.id))
                                .desired_width(width)
                                .char_limit(field.max_length)
                                .show(ui);
                            if output.response.changed() {
                                truncate_utf16(&mut self.values[index], field.max_length);
                            }
                            if !self.initialized {
                                let focus = field.focused
                                    || (index == 0
                                        && !self.request.fields.iter().any(|f| f.focused));
                                if focus {
                                    output.response.request_focus();
                                }
                                if let Some([start, end]) = field.selection {
                                    let a = egui::text::CCursor::new(utf16_cursor(
                                        &self.values[index],
                                        start,
                                    ));
                                    let b = egui::text::CCursor::new(utf16_cursor(
                                        &self.values[index],
                                        end,
                                    ));
                                    output
                                        .state
                                        .cursor
                                        .set_char_range(Some(egui::text::CCursorRange::two(a, b)));
                                    output.state.store(&ctx, output.response.id);
                                }
                            }
                            if !field.multiline
                                && output.response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                submit = true;
                            }
                            if let Some(choices) = &field.choices {
                                egui::ComboBox::from_id_salt(("choices", field.id))
                                    .width(24.0)
                                    .selected_text("")
                                    .show_ui(ui, |ui| {
                                        for choice in choices {
                                            if ui
                                                .selectable_label(
                                                    self.values[index] == *choice,
                                                    choice,
                                                )
                                                .clicked()
                                            {
                                                self.values[index] = choice.clone();
                                                truncate_utf16(
                                                    &mut self.values[index],
                                                    field.max_length,
                                                );
                                            }
                                        }
                                    });
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(button_label(&self.request.accept_label))
                    .clicked()
                {
                    submit = true;
                }
                if ui
                    .button(button_label(&self.request.cancel_label))
                    .clicked()
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
        self.initialized = true;
        if !self.composing && !committed {
            if submit || ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                self.accept(&ctx);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
fn button_label(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut label = String::new();
    while let Some(c) = chars.next() {
        if c != '&' {
            label.push(c);
        } else if chars.peek() == Some(&'&') {
            label.push('&');
            chars.next();
        }
    }
    label
}
fn truncate_utf16(text: &mut String, limit: usize) {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > limit {
            text.truncate(index);
            break;
        }
    }
}
fn utf16_cursor(text: &str, offset: i32) -> usize {
    if offset < 0 {
        return text.chars().count();
    }
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= offset as usize
        })
        .count()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_selection_and_limits_count_utf16_without_splitting_surrogates() {
        let mut text = "あ😀b".to_string();
        assert_eq!(utf16_cursor(&text, 3), 2);
        assert_eq!(utf16_cursor(&text, -1), 3);
        truncate_utf16(&mut text, 2);
        assert_eq!(text, "あ");
        assert_eq!(button_label("&OK && 続行"), "OK & 続行");
    }
}
