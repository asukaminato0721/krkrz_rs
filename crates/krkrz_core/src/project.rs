use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

/// Select project metadata without executing a Windows binary. An explicit
/// choice wins; otherwise prefer the directory name or a single non-helper EXE.
pub fn project_executable(project: &Path, explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    let project = project.canonicalize()?;
    if let Some(explicit) = explicit {
        let path = project.join(explicit).canonicalize()?;
        ensure!(
            path.starts_with(&project) && path.is_file(),
            "executable must be a file inside the project"
        );
        return Ok(Some(path));
    }
    let normalized = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let directory = normalized(&project.file_name().unwrap_or_default().to_string_lossy());
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&project)? {
        let path = entry?.path();
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            continue;
        }
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        if normalized(&stem) == directory {
            candidates.push((true, path));
        } else if ![
            "unins", "setup", "install", "updat", "patch", "config", "setting",
        ]
        .iter()
        .any(|prefix| stem.to_ascii_lowercase().starts_with(prefix))
        {
            candidates.push((false, path));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    // Ambiguous installations require an explicit choice rather than guessing.
    if candidates.len() == 1
        || candidates.first().is_some_and(|c| c.0) && !candidates.get(1).is_some_and(|c| c.0)
    {
        Ok(candidates.into_iter().next().map(|c| c.1))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_selection_is_generic_and_can_be_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Another_Game");
        std::fs::create_dir(&project).unwrap();
        for name in [
            "AnotherGame.EXE",
            "updater.exe",
            "unins000.exe",
            "launcher.exe",
        ] {
            std::fs::write(project.join(name), []).unwrap();
        }
        assert_eq!(
            project_executable(&project, None).unwrap(),
            Some(project.join("AnotherGame.EXE"))
        );
        assert_eq!(
            project_executable(&project, Some(Path::new("launcher.exe"))).unwrap(),
            Some(project.join("launcher.exe"))
        );
        assert!(project_executable(&project, Some(Path::new("missing.exe"))).is_err());
        std::fs::write(dir.path().join("outside.exe"), []).unwrap();
        assert!(project_executable(&project, Some(Path::new("../outside.exe"))).is_err());
        let generic = dir.path().join("generic");
        std::fs::create_dir(&generic).unwrap();
        for name in ["novel.exe", "unins000.exe"] {
            std::fs::write(generic.join(name), []).unwrap();
        }
        assert_eq!(
            project_executable(&generic, None).unwrap(),
            Some(generic.join("novel.exe"))
        );
        std::fs::write(generic.join("launcher.exe"), []).unwrap();
        assert_eq!(project_executable(&generic, None).unwrap(), None);
    }
}
