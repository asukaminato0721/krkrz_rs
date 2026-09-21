//! Native file pickers, isolated from the game's event loop in a child process.
use crate::file_dialog_egui::{matches_filter, with_default_extension};
use anyhow::{Result, ensure};
use krkrz_runtime::file_dialog::{FileDialog, FileSelection};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
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
    ensure!(
        !request.filters.iter().any(|s| s.contains('\0')),
        "NUL in file dialog filter"
    );
    let result = match native_filters(&request.filters) {
        Some(filters) => show(&request, &filters),
        // rfd accepts extension lists, whereas TJS also allows patterns such
        // as `stand_??.bmp`. Preserve those filters with the existing UI.
        None => crate::file_dialog_egui::show(request)?,
    };
    std::io::stdout().write_all(&serde_json::to_vec(&result)?)?;
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Filter {
    label: String,
    extensions: Vec<String>,
}

fn native_filters(filters: &[String]) -> Option<Vec<Filter>> {
    filters
        .iter()
        .map(|filter| {
            let (label, patterns) = filter.split_once('|').unwrap_or((filter, filter));
            let extensions: Option<Vec<_>> = patterns
                .split(';')
                .map(|pattern| {
                    let pattern = pattern.trim();
                    if matches!(pattern, "" | "*" | "*.*") {
                        return Some("*".into());
                    }
                    pattern
                        .strip_prefix("*.")
                        .filter(|ext| !ext.is_empty() && !ext.contains(['*', '?', '/', '\\']))
                        .map(str::to_owned)
                })
                .collect();
            Some(Filter {
                label: label.into(),
                extensions: extensions?,
            })
        })
        .collect()
}

fn initial_index(request: &FileDialog) -> usize {
    request
        .filter_index
        .saturating_sub(1)
        .min(request.filters.len().saturating_sub(1))
}

fn initial_path(request: &FileDialog) -> (PathBuf, String) {
    let path = if request.name.is_empty() {
        None
    } else {
        Some(request.initial_dir.join(&request.name))
    };
    let directory = path
        .as_ref()
        .and_then(|p| p.parent())
        .filter(|p| p.is_dir())
        .unwrap_or(&request.initial_dir)
        .to_path_buf();
    let name = path
        .as_ref()
        .filter(|p| p.parent().is_some_and(|p| p.is_dir()))
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| request.name.clone());
    (directory, name)
}

fn show(request: &FileDialog, filters: &[Filter]) -> Option<FileSelection> {
    let index = initial_index(request);
    let (directory, name) = initial_path(request);
    let name = if request.save {
        extend(PathBuf::from(name), request, index)
            .to_string_lossy()
            .into_owned()
    } else {
        name
    };
    let mut dialog = rfd::FileDialog::new()
        .set_title(&request.title)
        .set_directory(directory)
        .set_file_name(name);
    // rfd has no initial-filter setter; present the requested filter first.
    for i in std::iter::once(index).chain((0..filters.len()).filter(|&i| i != index)) {
        if let Some(filter) = filters.get(i) {
            dialog = dialog.add_filter(&filter.label, &filter.extensions);
        }
    }
    loop {
        let path = if request.save {
            dialog.clone().save_file()
        } else {
            dialog.clone().pick_file()
        }?;
        let filter_index = result_index(request, &path);
        let final_path = if request.save {
            extend(path.clone(), request, filter_index)
        } else {
            path.clone()
        };
        // Native pickers confirm the path they return. If our default suffix
        // changes that path to an existing file, reopen with the final name so
        // overwrite confirmation covers the actual destination.
        if final_path != path && final_path.exists() {
            dialog = dialog
                .set_directory(final_path.parent().unwrap_or(Path::new("/")))
                .set_file_name(final_path.file_name()?.to_string_lossy());
            continue;
        }
        return Some(FileSelection {
            path: final_path,
            filter_index: filter_index + 1,
        });
    }
}

fn extend(path: PathBuf, request: &FileDialog, index: usize) -> PathBuf {
    if path.as_os_str().is_empty() {
        return path;
    }
    with_default_extension(
        path,
        request
            .filters
            .get(index)
            .map(String::as_str)
            .unwrap_or("*"),
        &request.default_ext,
    )
}

fn result_index(request: &FileDialog, path: &Path) -> usize {
    // The public rfd API returns only a path. Infer a unique concrete match;
    // preserve the requested index for overlapping filters or no match.
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut matching = request
        .filters
        .iter()
        .enumerate()
        .filter_map(|(i, filter)| {
            let patterns = filter.split_once('|').map_or(filter.as_str(), |(_, p)| p);
            let concrete = patterns
                .split(';')
                .filter(|p| !matches!(p.trim(), "" | "*" | "*.*"))
                .any(|pattern| matches_filter(&name, pattern));
            concrete.then_some(i)
        });
    match (matching.next(), matching.next()) {
        (Some(i), None) => i,
        _ => initial_index(request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> FileDialog {
        FileDialog {
            title: "Select".into(),
            initial_dir: "/tmp".into(),
            name: "image".into(),
            filters: vec![
                "BMP|*.bmp".into(),
                "PNG|*.png;*.apng".into(),
                "All|*.*".into(),
            ],
            filter_index: 2,
            save: true,
            default_ext: ".bmp".into(),
        }
    }
    #[test]
    fn native_filters_preserve_labels_extensions_and_wildcards() {
        let filters = native_filters(&request().filters).unwrap();
        assert_eq!(
            filters[1],
            Filter {
                label: "PNG".into(),
                extensions: vec!["png".into(), "apng".into()]
            }
        );
        assert_eq!(filters[2].extensions, ["*"]);
        assert!(native_filters(&["Images|image?.bmp".into()]).is_none());
        assert!(native_filters(&["Images|*.bmp;stand_*.png".into()]).is_none());
    }
    #[test]
    fn results_use_original_filter_indices_and_keep_explicit_extensions() {
        let mut r = request();
        assert_eq!(initial_index(&r), 1);
        assert_eq!(result_index(&r, Path::new("画像.BMP")), 0);
        assert_eq!(result_index(&r, Path::new("image.apng")), 1);
        assert_eq!(extend("image".into(), &r, 1), Path::new("image.png"));
        assert_eq!(extend("image.bmp".into(), &r, 1), Path::new("image.bmp"));
        r.filters.push("More PNG|*.png".into());
        assert_eq!(result_index(&r, Path::new("image.png")), 1);
        r.filter_index = 100;
        assert_eq!(initial_index(&r), 3);
        r.filters.clear();
        assert_eq!(result_index(&r, Path::new("image")), 0);
        assert_eq!(extend("image".into(), &r, 0), Path::new("image.bmp"));
    }
    #[test]
    fn initial_names_preserve_subdirectories_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut r = request();
        r.initial_dir = dir.path().into();
        r.name = "sub/画像".into();
        assert_eq!(initial_path(&r), (dir.path().join("sub"), "画像".into()));
        r.name = "missing/画像".into();
        assert_eq!(initial_path(&r), (dir.path().into(), r.name.clone()));
    }
}
