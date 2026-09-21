//! Portable file selection. Only files explicitly selected by the host become
//! readable outside the project/save roots. Save selection grants writes to
//! that exact file, without changing the session's save directory.
use crate::{Services, Value};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Vm, unsupported};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FileDialog {
    pub title: String,
    pub initial_dir: PathBuf,
    pub name: String,
    pub filters: Vec<String>,
    /// Win32 filter indices start at one; zero selects the first filter.
    pub filter_index: usize,
    #[serde(default)]
    pub save: bool,
    #[serde(default)]
    pub default_ext: String,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FileSelection {
    pub path: PathBuf,
    pub filter_index: usize,
}
type Handler = Box<dyn FnMut(&FileDialog) -> Result<Option<FileSelection>>>;
#[derive(Default)]
pub(crate) struct State {
    handler: Option<Handler>,
    selected: BTreeMap<String, (PathBuf, bool)>,
}

pub(crate) fn system_path(personal: bool, project: &Path) -> String {
    let path = if personal {
        dirs::document_dir().or_else(dirs::data_dir)
    } else {
        dirs::data_dir()
    }
    .unwrap_or_else(|| project.to_path_buf());
    format!("file://.{}/", path.to_string_lossy().trim_end_matches('/'))
}

impl Services {
    pub fn set_file_dialog_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&FileDialog) -> Result<Option<FileSelection>> + 'static,
    {
        self.file_dialogs.handler = Some(Box::new(handler));
    }
    pub(crate) fn selected_file(&self, name: &str) -> Option<&Path> {
        self.file_dialogs
            .selected
            .get(&krkrz_core::local_storage_path(name).to_ascii_lowercase())
            .map(|(path, _)| path.as_path())
    }
    pub(crate) fn write_storage_path(&self, name: &str) -> Result<PathBuf> {
        if let Some((path, writable)) = self
            .file_dialogs
            .selected
            .get(&krkrz_core::local_storage_path(name).to_ascii_lowercase())
        {
            ensure!(*writable, "file was selected for reading only");
            ensure!(
                save_selection_path(path)? == *path,
                "selected file path changed"
            );
            return Ok(path.clone());
        }
        crate::save_storage::path(&self.storage.project, &self.save_dir, name)
    }
    pub(crate) fn select_file(
        &mut self,
        vm: &mut Vm,
        options: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let mut get =
            |key: &str| vm.get_property(options, &Value::string(key), true, false, self, budget);
        let save = get("save")?.truth()?;
        let default_ext = get("defaultExt")?.text();
        let title = get("title")?.text();
        let initial = get("initialDir")?.text();
        let name = get("name")?.text();
        let index = get("filterIndex")?;
        let index = if index == Value::Void {
            0
        } else {
            index.integer()?
        };
        let filter = get("filter")?;
        let filters = if filter == Value::Void {
            vec![]
        } else if matches!(filter, Value::String(_)) {
            vec![filter.text()]
        } else {
            let count = vm
                .get_property(&filter, &Value::string("count"), false, false, self, budget)?
                .integer()?;
            ensure!(
                (0..=128).contains(&count),
                "file dialog filter count exceeds limit"
            );
            let mut values = Vec::new();
            for i in 0..count {
                values.push(
                    vm.get_property(&filter, &Value::Integer(i), false, false, self, budget)?
                        .text(),
                );
            }
            values
        };
        let request = FileDialog {
            title: if title.is_empty() {
                if save { "Save file" } else { "Open file" }.into()
            } else {
                title
            },
            initial_dir: if initial.is_empty() {
                self.storage.project.clone()
            } else {
                PathBuf::from(krkrz_core::local_storage_path(&initial))
            },
            name: krkrz_core::local_storage_path(&name).into(),
            filters,
            filter_index: index.max(0) as usize,
            save,
            default_ext,
        };
        let handler =
            self.file_dialogs.handler.as_mut().ok_or_else(|| {
                unsupported("Storages.selectFile requires a native file dialog host")
            })?;
        let Some(selection) = handler(&request)? else {
            return Ok(Value::Integer(0));
        };
        let path = if save {
            save_selection_path(&selection.path)?
        } else {
            let path = selection
                .path
                .canonicalize()
                .context("resolve selected file")?;
            ensure!(path.is_file(), "selected path is not a file");
            path
        };
        ensure!(
            selection.filter_index > 0 && selection.filter_index <= request.filters.len().max(1),
            "invalid selected filter index"
        );
        let name = format!("file://.{}", path.display());
        vm.set_property(
            options,
            &Value::string("filterIndex"),
            Value::Integer(selection.filter_index as i64),
            false,
            self,
            budget,
        )?;
        vm.set_property(
            options,
            &Value::string("name"),
            Value::string(&name),
            false,
            self,
            budget,
        )?;
        self.file_dialogs
            .selected
            .entry(path.to_string_lossy().to_ascii_lowercase())
            .and_modify(|(previous, writable)| {
                *writable = save || (*previous == path && *writable);
                *previous = path.clone();
            })
            .or_insert((path, save));
        Ok(Value::Integer(1))
    }
}

/// Resolve an existing parent for a new file; reject symlinks as save targets.
/// Rechecked before each write, including data appended after an image export.
fn save_selection_path(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "save selection must be an absolute path"
    );
    let parent = path
        .parent()
        .context("save selection has no parent")?
        .canonicalize()?;
    ensure!(parent.is_dir(), "save selection parent is not a directory");
    let name = path.file_name().context("save selection has no filename")?;
    let path = parent.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(
            metadata.file_type().is_file(),
            "save selection is not a regular file"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}
