use crate::{
    cx::CxEncryption,
    xp3::{Archive, Entry},
};
use anyhow::{Context, Result, ensure};
use krkrz_core::{Limits, storage_name};
use std::{
    cell::RefCell,
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
    loose_directories: RefCell<BTreeMap<PathBuf, DirectoryIndex>>,
}

struct DirectoryIndex {
    metadata: fs::Metadata,
    // None records a case-insensitive collision, which must remain an error.
    entries: BTreeMap<String, Option<PathBuf>>,
}

impl DirectoryIndex {
    fn unchanged(&self, metadata: &fs::Metadata) -> bool {
        let modified = self.metadata.modified().ok();
        if modified.is_none() || modified != metadata.modified().ok() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let old = &self.metadata;
            if (old.dev(), old.ino(), old.ctime(), old.ctime_nsec())
                != (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            {
                return false;
            }
        }
        true
    }
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
            loose_directories: RefCell::new(BTreeMap::new()),
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
        let path = self.normalize(path, true)?;
        self.search_paths.retain(|existing| existing != &path);
        self.search_paths.push(path);
        Ok(())
    }
    /// Normalize a storage name without requiring the file or directory to
    /// exist. Auto paths and patch selection belong to `resolve`, not this API.
    pub fn full_path(&self, name: &str) -> Result<String> {
        if name.is_empty() {
            return Ok(String::new());
        }
        ensure!(!name.contains('\0'), "NUL in storage name");
        let name = name.replace('\\', "/").to_ascii_lowercase();
        let name = krkrz_core::local_storage_path(&name);
        ensure!(
            !name.contains(':'),
            "unsupported storage medium or drive path"
        );
        let (outer, member) = name
            .split_once('>')
            .map_or((name, None), |(a, b)| (a, Some(b)));
        ensure!(
            !outer.is_empty(),
            "archive storage requires an archive name"
        );
        let absolute = if outer.starts_with('/') {
            outer.to_owned()
        } else {
            format!("{}/{outer}", self.project.display())
        };
        let outer = normalize_full_component(&absolute, true)?;
        let mut result = format!("file://.{outer}");
        if let Some(member) = member {
            ensure!(
                !member.contains('>'),
                "nested archive storage is not supported"
            );
            result.push('>');
            result.push_str(&normalize_full_component(member, false)?);
        }
        Ok(result)
    }
    fn normalize(&self, name: &str, directory: bool) -> Result<String> {
        let name = name.replace('\\', "/");
        let name = krkrz_core::local_storage_path(&name);
        let prefix = format!("{}/", self.project.display());
        let name = if name
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
        {
            &name[prefix.len()..]
        } else {
            name
        };
        if let Some((archive, member)) = name.split_once('>') {
            ensure!(
                !member.contains('>'),
                "nested archive storage is not supported"
            );
            let archive = storage_name(archive)?;
            let member = if directory && member.is_empty() {
                String::new()
            } else {
                storage_name(member)?
            };
            Ok(format!("{archive}>{member}"))
        } else {
            if directory && (name.is_empty() || name == "." || name == "./") {
                Ok(String::new())
            } else {
                storage_name(name)
            }
        }
    }
    fn archive_location<'a>(&self, name: &'a str) -> Option<(usize, &'a str)> {
        if let Some((archive, member)) = name.split_once('>') {
            self.archives
                .iter()
                .position(|a| {
                    a.path
                        .strip_prefix(&self.project)
                        .is_ok_and(|p| p.to_string_lossy().eq_ignore_ascii_case(archive))
                })
                .filter(|i| self.archives[*i].entries.contains_key(member))
                .map(|i| (i, member))
        } else {
            self.catalog.get(name).map(|i| (*i, name))
        }
    }
    fn contains(
        &self,
        name: &str,
        directories: &mut BTreeMap<PathBuf, Option<fs::Metadata>>,
    ) -> Result<bool> {
        Ok(self.loose_path_with_metadata(name, directories)?.is_some()
            || self.archive_location(name).is_some())
    }
    pub fn resolve(&self, name: &str) -> Result<String> {
        let name = self.normalize(name, false)?;
        // Auto paths often share the same missing loose directory. Validate
        // each directory once during this lookup, not once per candidate.
        let mut directories = BTreeMap::new();
        if self.contains(&name, &mut directories)? {
            return Ok(name);
        }
        if !name.contains('>') {
            for path in self.search_paths.iter().rev() {
                let separator = if path.is_empty() || path.ends_with('>') {
                    ""
                } else {
                    "/"
                };
                let candidate = format!("{path}{separator}{name}");
                if self.contains(&candidate, &mut directories)? {
                    return Ok(candidate);
                }
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
        let (index, member) = self
            .archive_location(&name)
            .context("archive entry disappeared")?;
        self.archives[index]
            .read(member, self.cipher.as_ref())
            .with_context(|| format!("read {name}"))
    }
    fn loose_path(&self, name: &str) -> Result<Option<PathBuf>> {
        self.loose_path_with_metadata(name, &mut BTreeMap::new())
    }

    fn loose_path_with_metadata(
        &self,
        name: &str,
        directories: &mut BTreeMap<PathBuf, Option<fs::Metadata>>,
    ) -> Result<Option<PathBuf>> {
        if name.contains('>') {
            return Ok(None);
        }
        let mut path = self.project.clone();
        for component in name.split('/') {
            let Some(metadata) = directories
                .entry(path.clone())
                .or_insert_with(|| fs::metadata(&path).ok())
                .as_ref()
            else {
                return Ok(None);
            };
            if !metadata.is_dir() {
                return Ok(None);
            }
            let mut cache = self.loose_directories.borrow_mut();
            if !cache
                .get(&path)
                .is_some_and(|index| index.unchanged(metadata))
            {
                let mut entries = BTreeMap::new();
                for (count, entry) in fs::read_dir(&path)?.enumerate() {
                    ensure!(
                        count < self.limits.entries,
                        "loose directory entry limit exceeded"
                    );
                    let entry = entry?;
                    entries
                        .entry(entry.file_name().to_string_lossy().to_ascii_lowercase())
                        .and_modify(|value| *value = None)
                        .or_insert_with(|| Some(entry.path()));
                }
                // Bound both directory count and total indexed names. Eviction
                // affects only lookup speed; metadata validates every cache hit.
                cache.remove(&path);
                if cache.len() >= 64
                    || cache
                        .values()
                        .map(|index| index.entries.len())
                        .sum::<usize>()
                        + entries.len()
                        > self.limits.entries
                {
                    cache.clear();
                }
                cache.insert(
                    path.clone(),
                    DirectoryIndex {
                        metadata: metadata.clone(),
                        entries,
                    },
                );
            }
            let Some(next) = cache[&path].entries.get(&component.to_ascii_lowercase()) else {
                return Ok(None);
            };
            let next = next
                .as_ref()
                .with_context(|| format!("ambiguous case-insensitive storage name: {name}"))?;
            // Revalidate symlinks even when the containing directory is cached.
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
        let (index, member) = self
            .archive_location(&name)
            .context("storage is a loose file, not an archive entry")?;
        let archive = &self.archives[index];
        Ok((&archive.path, &archive.entries[member]))
    }
    pub fn verify(&mut self, name: &str) -> Result<()> {
        let name = self.resolve(name)?;
        ensure!(
            self.loose_path(&name)?.is_none(),
            "loose file has no XP3 checksum: {name}"
        );
        let (index, member) = self
            .archive_location(&name)
            .context("archive entry disappeared")?;
        self.archives[index].verify(member, self.cipher.as_ref())
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

fn normalize_full_component(path: &str, absolute: bool) -> Result<String> {
    let mut parts = Vec::new();
    let mut directory = path.ends_with('/');
    let mut components = path.split('/').peekable();
    while let Some(part) = components.next() {
        if part.is_empty() {
            continue;
        }
        // The installed engine collapses dot segments only when another path
        // separator follows. A final dot segment remains literal.
        if components.peek().is_some() && part.bytes().all(|c| c == b'.') {
            let parents = part.len() - 1;
            ensure!(parents <= parts.len(), "storage path escapes its root");
            parts.truncate(parts.len() - parents);
            directory = true;
        } else {
            parts.push(part);
            directory = false;
        }
    }
    let mut result = if absolute {
        String::from("/")
    } else {
        String::new()
    };
    result.push_str(&parts.join("/"));
    if (directory || path.ends_with('/')) && !result.is_empty() && !result.ends_with('/') {
        result.push('/');
    }
    Ok(result)
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
    #[test]
    fn cached_directory_observes_creates_renames_removals_and_content_writes() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("Scripts")).unwrap();
        let mut storage = Storage::open(project.path(), None, Limits::default()).unwrap();
        storage.add_search_path("scripts/").unwrap();
        assert!(storage.resolve("new.tjs").is_err());
        let file = project.path().join("Scripts/New.TJS");
        fs::write(&file, b"first").unwrap();
        assert_eq!(storage.read("new.tjs").unwrap(), b"first");
        fs::write(&file, b"updated").unwrap();
        assert_eq!(storage.read("new.tjs").unwrap(), b"updated");
        let renamed = project.path().join("Scripts/Renamed.TJS");
        fs::rename(&file, &renamed).unwrap();
        assert!(storage.resolve("new.tjs").is_err());
        assert_eq!(storage.read("renamed.tjs").unwrap(), b"updated");
        fs::remove_file(renamed).unwrap();
        assert!(storage.resolve("renamed.tjs").is_err());
    }

    #[test]
    fn directory_cache_still_enforces_entry_limits_after_a_change() {
        let project = tempfile::tempdir().unwrap();
        fs::write(project.path().join("a"), b"a").unwrap();
        let mut storage = Storage::open(
            project.path(),
            None,
            Limits {
                entries: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(storage.read("a").unwrap(), b"a");
        fs::write(project.path().join("b"), b"b").unwrap();
        assert!(
            storage
                .read("a")
                .unwrap_err()
                .to_string()
                .contains("entry limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn cached_directory_rechecks_retargeted_symlinks() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(project.path().join("inside.tjs"), b"inside").unwrap();
        fs::write(outside.path().join("secret.tjs"), b"outside").unwrap();
        let link = project.path().join("alias.tjs");
        std::os::unix::fs::symlink(project.path().join("inside.tjs"), &link).unwrap();
        let mut storage = Storage::open(project.path(), None, Limits::default()).unwrap();
        assert_eq!(storage.read("alias.tjs").unwrap(), b"inside");
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.tjs"), &link).unwrap();
        assert!(
            storage
                .read("alias.tjs")
                .unwrap_err()
                .to_string()
                .contains("escapes")
        );
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
