//! Shared storage names, limits, diagnostics, and deterministic host input.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Resolve the POSIX local-storage prefix used by Kirikiri's SDL ports.
/// This is a storage name, not a percent-encoded web URL.
pub fn local_storage_path(name: &str) -> &str {
    name.strip_prefix("file://.")
        .filter(|path| path.starts_with('/'))
        .unwrap_or(name)
}

/// Archive names use ASCII case folding, not Unicode case folding.
pub fn storage_name(name: &str) -> Result<String> {
    ensure!(!name.contains('\0'), "NUL in storage name");
    let name = name.replace('\\', "/").to_ascii_lowercase();
    ensure!(
        !name.starts_with('/') && !name.contains(':'),
        "absolute storage name: {name}"
    );
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                ensure!(parts.pop().is_some(), "storage escapes root: {name}");
            }
            _ => parts.push(part),
        }
    }
    ensure!(!parts.is_empty(), "empty storage name");
    Ok(parts.join("/"))
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub index_bytes: u64,
    pub file_bytes: u64,
    pub cache_bytes: usize,
    pub entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            index_bytes: 64 << 20,
            file_bytes: 512 << 20,
            cache_bytes: 64 << 20,
            entries: 1_000_000,
        }
    }
}

/// Default to the game's existing saves. Explicit overrides can select an
/// isolated directory; other installation resources are not save destinations.
pub fn save_directory(project: &Path, override_path: Option<&Path>) -> Result<PathBuf> {
    let project = project.canonicalize()?;
    let result = if let Some(path) = override_path {
        path.to_path_buf()
    } else {
        project.join("savedata")
    };
    // Resolve existing ancestors, including symlinks, before creating anything.
    let absolute = if result.is_absolute() {
        result
    } else {
        std::env::current_dir()?.join(result)
    };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("invalid save path"))?,
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid save path"))?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    ensure!(
        !resolved.starts_with(&project) || resolved.starts_with(project.join("savedata")),
        "save directory must be in the game's savedata directory or outside the installation"
    );
    Ok(resolved)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceLocation {
    pub storage: String,
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InputEvent {
    pub time_ms: u64,
    pub action: InputAction,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputAction {
    Advance,
    Pointer { x: i32, y: i32, pressed: bool },
    Key { code: String, pressed: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization() {
        assert_eq!(storage_name("A\\B//../日.TJS").unwrap(), "a/日.tjs");
        assert_eq!(storage_name("Ä.TJS").unwrap(), "Ä.tjs");
        for s in ["../../bad", "/tmp/x", "C:\\x", "", "a\0b"] {
            assert!(storage_name(s).is_err());
        }
    }
    #[test]
    fn default_saves_use_the_game_savedata_directory() {
        let p = tempfile::tempdir().unwrap();
        let expected = p.path().canonicalize().unwrap().join("savedata");
        assert_eq!(save_directory(p.path(), None).unwrap(), expected);
        assert!(!expected.exists());
        std::fs::create_dir(&expected).unwrap();
        assert_eq!(save_directory(p.path(), None).unwrap(), expected);
        assert_eq!(save_directory(p.path(), Some(&expected)).unwrap(), expected);
        assert!(save_directory(p.path(), Some(p.path())).is_err());
        assert!(save_directory(p.path(), Some(&p.path().join("data"))).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn save_path_checks_existing_symlink_ancestors() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(project.path(), outside.path().join("game")).unwrap();
        let path = outside.path().join("game/new/saves");
        assert!(save_directory(project.path(), Some(&path)).is_err());
        let safe = outside.path().join("new/saves");
        assert_eq!(save_directory(project.path(), Some(&safe)).unwrap(), safe);
        assert!(!safe.exists());
    }
}
