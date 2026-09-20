use crate::{
    cx::CxEncryption,
    xp3::{Archive, Entry},
};
use anyhow::{Context, Result, ensure};
use krkrz_core::{Limits, storage_name};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Mounted archives are resolved in mount order; later mounts override earlier ones.
/// All reads are from the original archives; mounting never extracts files.
pub struct Storage {
    pub project: PathBuf,
    pub archives: Vec<Archive>,
    pub catalog: BTreeMap<String, usize>,
    cipher: Option<CxEncryption>,
    search_paths: Vec<String>,
    limits: Limits,
}
impl Storage {
    pub fn open(project: &Path, cipher: Option<CxEncryption>, limits: Limits) -> Result<Self> {
        let project = project.canonicalize()?;
        let mut files = Vec::new();
        for entry in fs::read_dir(&project)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("xp3"))
            {
                files.push(path);
            }
        }
        files.sort_by_key(|p| archive_priority(p));
        let mut storage = Self {
            project,
            archives: vec![],
            catalog: BTreeMap::new(),
            cipher,
            search_paths: vec![],
            limits,
        };
        for path in files {
            storage.mount(&path, limits)?;
        }
        Ok(storage)
    }
    pub fn mount(&mut self, path: &Path, limits: Limits) -> Result<()> {
        let archive = Archive::open(path, limits)?;
        let index = self.archives.len();
        for name in archive.entries.keys() {
            self.catalog.insert(name.clone(), index);
        }
        self.archives.push(archive);
        Ok(())
    }
    pub fn add_search_path(&mut self, path: &str) -> Result<()> {
        let path = storage_name(path)?;
        if !self.search_paths.contains(&path) {
            self.search_paths.push(path);
        }
        Ok(())
    }
    pub fn resolve(&self, name: &str) -> Result<String> {
        let name = storage_name(name)?;
        if self.loose_path(&name)?.is_some() || self.catalog.contains_key(&name) {
            return Ok(name);
        }
        for path in self.search_paths.iter().rev() {
            let candidate = format!("{path}/{name}");
            if self.loose_path(&candidate)?.is_some() || self.catalog.contains_key(&candidate) {
                return Ok(candidate);
            }
        }
        // Kirikiri auto-paths must be registered explicitly. Do not silently choose
        // one of several basename matches from unrelated archive directories.
        anyhow::bail!("storage not found: {name}")
    }
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let name = self.resolve(name)?;
        if let Some(path) = self.loose_path(&name)? {
            use std::io::Read;
            let file = fs::File::open(path)?;
            ensure!(
                file.metadata()?.len() <= self.limits.file_bytes,
                "loose storage exceeds limit"
            );
            let mut bytes = Vec::new();
            file.take(self.limits.file_bytes.saturating_add(1))
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= self.limits.file_bytes,
                "loose storage exceeds limit"
            );
            return Ok(bytes);
        }
        let index = self.catalog[&name];
        self.archives[index]
            .read(&name, self.cipher.as_ref())
            .with_context(|| format!("read {name}"))
    }
    fn loose_path(&self, name: &str) -> Result<Option<PathBuf>> {
        let mut path = self.project.clone();
        for component in name.split('/') {
            if !path.is_dir() {
                return Ok(None);
            }
            let mut found = None;
            for (count, entry) in fs::read_dir(&path)?.enumerate() {
                ensure!(
                    count < self.limits.entries,
                    "loose directory entry limit exceeded"
                );
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(component)
                {
                    ensure!(
                        found.is_none(),
                        "ambiguous case-insensitive storage name: {name}"
                    );
                    found = Some(entry.path());
                }
            }
            let Some(next) = found else {
                return Ok(None);
            };
            path = next.canonicalize()?;
            ensure!(
                path.starts_with(&self.project),
                "storage symlink escapes game directory: {name}"
            );
        }
        Ok(path.is_file().then_some(path))
    }
    pub fn placed_path(&self, name: &str) -> Result<String> {
        let name = self.resolve(name)?;
        if let Some(path) = self.loose_path(&name)? {
            return Ok(path.to_string_lossy().into_owned());
        }
        let (path, entry) = self.entry(&name)?;
        Ok(format!("{}>{}", path.display(), entry.name))
    }
    pub fn entry(&self, name: &str) -> Result<(&Path, &Entry)> {
        let name = self.resolve(name)?;
        let index = *self
            .catalog
            .get(&name)
            .context("storage is a loose file, not an archive entry")?;
        let archive = &self.archives[index];
        Ok((&archive.path, &archive.entries[&name]))
    }
    pub fn verify(&mut self, name: &str) -> Result<()> {
        let name = self.resolve(name)?;
        ensure!(
            self.loose_path(&name)?.is_none(),
            "loose file has no XP3 checksum: {name}"
        );
        let index = self.catalog[&name];
        self.archives[index].verify(&name, self.cipher.as_ref())
    }
    /// Extract a single explicitly named storage. Never overwrite an existing file.
    pub fn extract(&mut self, name: &str, destination: &Path) -> Result<()> {
        let bytes = self.read(name)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize()?;
        ensure!(
            !parent.starts_with(&self.project),
            "extraction destination must be outside the game installation"
        );
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        file.write_all(&bytes)?;
        Ok(())
    }
}
fn archive_priority(path: &Path) -> (bool, u64, String) {
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let patch = name.strip_prefix("patch").and_then(|n| {
        if n.is_empty() {
            Some(0)
        } else {
            n.parse().ok()
        }
    });
    (patch.is_some(), patch.unwrap_or(0), name)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_order_is_numeric() {
        let mut paths = [
            "patch10.xp3",
            "voice.xp3",
            "patch2.xp3",
            "data.xp3",
            "patch.xp3",
        ]
        .map(PathBuf::from);
        paths.sort_by_key(|p| archive_priority(p));
        assert_eq!(
            paths.map(|p| p.to_string_lossy().into_owned()),
            [
                "data.xp3",
                "voice.xp3",
                "patch.xp3",
                "patch2.xp3",
                "patch10.xp3"
            ]
        );
    }
    #[test]
    fn loose_files_resolve_case_and_registered_paths() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("Scripts")).unwrap();
        fs::write(project.path().join("Scripts/Hello.TJS"), b"return 1;").unwrap();
        let mut storage = Storage::open(project.path(), None, Limits::default()).unwrap();
        assert!(storage.read("hello.tjs").is_err());
        storage.add_search_path("scripts/").unwrap();
        assert_eq!(storage.read("HELLO.tjs").unwrap(), b"return 1;");
        assert!(storage.verify("hello.tjs").is_err());
        fs::write(project.path().join("Scripts/hello.tjs"), b"return 2;").unwrap();
        assert!(storage.read("hello.tjs").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn loose_symlinks_cannot_escape_the_installation() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.tjs"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("scripts")).unwrap();
        let mut storage = Storage::open(project.path(), None, Limits::default()).unwrap();
        assert!(storage.read("scripts/secret.tjs").is_err());
    }
}
