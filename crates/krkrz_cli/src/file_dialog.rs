use anyhow::{Result, ensure};
use eframe::egui;
use krkrz_runtime::file_dialog::{FileDialog, FileSelection};
use std::{
    cell::RefCell,
    io::{Read, Write},
    path::PathBuf,
    rc::Rc,
};

pub fn run_child() -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((1 << 20) + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1 << 20, "file dialog request exceeds limit");
    let request: FileDialog = serde_json::from_slice(&bytes)?;
    ensure!(
        request.filters.len() <= 128,
        "file dialog filter count exceeds limit"
    );
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title(&request.title)
            .with_inner_size([720.0, 520.0])
            .with_min_inner_size([440.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        "krkrz-file-dialog",
        options,
        Box::new(move |cc| {
            crate::input_dialog::install_fonts(&cc.egui_ctx);
            Ok(Box::new(FileApp::new(request, output)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("egui file dialog: {e}"))?;
    std::io::stdout().write_all(&serde_json::to_vec(&*result.borrow())?)?;
    Ok(())
}

struct FileApp {
    directory: PathBuf,
    location: String,
    name: String,
    filters: Vec<String>,
    filter: usize,
    entries: Vec<(String, bool)>,
    error: String,
    save: bool,
    default_ext: String,
    overwrite: Option<PathBuf>,
    result: Rc<RefCell<Option<FileSelection>>>,
}
impl FileApp {
    fn new(request: FileDialog, result: Rc<RefCell<Option<FileSelection>>>) -> Self {
        let mut directory = request.initial_dir;
        let initial = PathBuf::from(&request.name);
        let mut name = request.name;
        if initial.is_absolute()
            && let Some(parent) = initial.parent()
            && parent.is_dir()
        {
            directory = parent.to_path_buf();
            name = initial
                .file_name()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        }
        while !directory.is_dir() && directory.parent().is_some() {
            directory.pop();
        }
        if !directory.is_dir() {
            directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        }
        let filters = if request.filters.is_empty() {
            vec!["All files|*".into()]
        } else {
            request.filters
        };
        let filter = request
            .filter_index
            .saturating_sub(1)
            .min(filters.len() - 1);
        let mut app = Self {
            location: directory.to_string_lossy().into_owned(),
            directory,
            name,
            filters,
            filter,
            entries: Vec::new(),
            error: String::new(),
            result,
            save: request.save,
            default_ext: request.default_ext,
            overwrite: None,
        };
        app.refresh();
        app
    }
    fn refresh(&mut self) {
        self.entries.clear();
        self.error.clear();
        let result = (|| -> std::io::Result<()> {
            for entry in std::fs::read_dir(&self.directory)?.take(20_001) {
                let entry = entry?;
                if self.entries.len() >= 20_000 {
                    self.error =
                        "This directory has too many entries; enter a filename directly.".into();
                    break;
                }
                self.entries.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path().is_dir(),
                ));
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = e.to_string();
        }
        self.entries.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
    }
    fn navigate(&mut self, path: PathBuf) {
        match path.canonicalize() {
            Ok(path) if path.is_dir() => {
                self.directory = path;
                self.location = self.directory.to_string_lossy().into_owned();
                self.name.clear();
                self.refresh();
            }
            _ => self.error = "Cannot open this directory.".into(),
        }
    }
    fn open(&mut self, ctx: &egui::Context) {
        if self.name.is_empty() {
            self.error = "Enter a file name.".into();
            return;
        }
        let mut path = self.directory.join(&self.name);
        if path.is_dir() {
            self.navigate(path);
            return;
        }
        if self.save {
            path = with_default_extension(path, &self.filters[self.filter], &self.default_ext);
            let Some(parent) = path
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .filter(|p| p.is_dir())
            else {
                self.error = "The destination directory does not exist.".into();
                return;
            };
            let Some(name) = path.file_name() else {
                self.error = "Enter a file name.".into();
                return;
            };
            let path = parent.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_file() => self.overwrite = Some(path),
                Ok(_) => {
                    self.error =
                        "Select a regular file; directories and links cannot be replaced.".into()
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.accept(path, ctx),
                Err(e) => self.error = e.to_string(),
            }
            return;
        }
        match path.canonicalize() {
            Ok(path) if path.is_file() => self.accept(path, ctx),
            _ => self.error = "Select an existing file.".into(),
        }
    }
    fn accept(&mut self, path: PathBuf, ctx: &egui::Context) {
        *self.result.borrow_mut() = Some(FileSelection {
            path,
            filter_index: self.filter + 1,
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
impl eframe::App for FileApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Up").clicked()
                    && let Some(parent) = self.directory.parent()
                {
                    self.navigate(parent.to_path_buf());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.location)
                        .desired_width((ui.available_width() - 45.0).max(50.0)),
                );
                if ui.button("Go").clicked() {
                    self.navigate(PathBuf::from(&self.location));
                }
            });
            ui.separator();
            let patterns = self.filters[self.filter]
                .split_once('|')
                .map_or(self.filters[self.filter].as_str(), |(_, p)| p);
            let mut open = false;
            let mut enter = None;
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 115.0).max(60.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (name, directory) in &self.entries {
                        if !directory && !matches_filter(name, patterns) {
                            continue;
                        }
                        let label = if *directory {
                            format!("{name}/")
                        } else {
                            name.clone()
                        };
                        let response = ui.selectable_label(self.name == *name, label);
                        if response.clicked() {
                            self.name = name.clone();
                        }
                        if response.double_clicked() {
                            if *directory {
                                enter = Some(self.directory.join(name));
                            } else {
                                self.name = name.clone();
                                open = true;
                            }
                        }
                    }
                });
            if let Some(path) = enter {
                self.navigate(path);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("File name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
                );
            });
            egui::ComboBox::from_id_salt("filter")
                .selected_text(self.filters[self.filter].split('|').next().unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (i, filter) in self.filters.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.filter,
                            i,
                            filter.split('|').next().unwrap_or(""),
                        );
                    }
                });
            ui.horizontal(|ui| {
                if ui.button(if self.save { "Save" } else { "Open" }).clicked() {
                    open = true;
                }
                if ui.button("Cancel").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            if !self.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.error);
            }
            if open {
                self.open(&ctx);
            }
        });
        if let Some(path) = self.overwrite.clone() {
            egui::Modal::new(egui::Id::new("overwrite")).show(&ctx, |ui| {
                ui.heading("Replace existing file?");
                ui.label(path.to_string_lossy());
                ui.horizontal(|ui| {
                    if ui.button("Replace").clicked() {
                        self.accept(path, &ctx);
                    }
                    if ui.button("Cancel").clicked() {
                        self.overwrite = None;
                    }
                });
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.overwrite = None;
            }
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn with_default_extension(mut path: PathBuf, filter: &str, default: &str) -> PathBuf {
    if path.extension().is_none() && !default.is_empty() {
        // Win32 uses the selected filter's first concrete extension, falling
        // back to lpstrDefExt for wildcard filters. Explicit suffixes survive.
        let extension = filter
            .split_once('|')
            .and_then(|(_, patterns)| patterns.split(';').next())
            .and_then(|p| p.trim().strip_prefix("*."))
            .filter(|ext| !ext.is_empty() && !ext.contains(['*', '?', '/', '\\']))
            .unwrap_or(default.trim_start_matches('.'));
        if !extension.is_empty() && !extension.contains(['/', '\\']) {
            path.set_extension(extension);
        }
    }
    path
}

fn matches_filter(name: &str, patterns: &str) -> bool {
    patterns.split(';').any(|pattern| {
        let pattern = pattern.trim().to_lowercase();
        if pattern == "*.*" || pattern == "*" || pattern.is_empty() {
            return true;
        }
        let name: Vec<_> = name.to_lowercase().chars().collect();
        let mut matches = vec![false; name.len() + 1];
        matches[0] = true;
        for c in pattern.chars() {
            if c == '*' {
                for i in 1..matches.len() {
                    matches[i] |= matches[i - 1];
                }
            } else {
                for i in (1..matches.len()).rev() {
                    matches[i] = matches[i - 1] && (c == '?' || c == name[i - 1]);
                }
                matches[0] = false;
            }
        }
        matches[name.len()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_file_patterns_are_case_insensitive_and_allow_multiple_extensions() {
        assert!(matches_filter("画像.BMP", "*.bmp;*.png"));
        assert!(matches_filter("README", "*.*"));
        assert!(matches_filter("画像1.png", "画像?.png"));
        assert!(!matches_filter("image.txt", "*.bmp;*.png"));
    }
    #[test]
    fn save_extensions_follow_filters_without_replacing_explicit_suffixes() {
        for (name, filter, default, expected) in [
            ("画像", "BMP|*.bmp", ".bmp", "画像.bmp"),
            ("image", "PNG|*.png;*.apng", ".bmp", "image.png"),
            ("image", "All|*.*", ".bmp", "image.bmp"),
            ("image.png", "BMP|*.bmp", ".bmp", "image.png"),
            ("image", "BMP|*.bmp", "", "image"),
        ] {
            assert_eq!(
                with_default_extension(name.into(), filter, default),
                PathBuf::from(expected)
            );
        }
    }
}
