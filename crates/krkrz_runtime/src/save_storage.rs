//! Writable storage overlay. Installed resources are never write targets.
use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn path(project: &Path, root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.contains('>') && !name.contains('\0'),
        "archive or NUL in writable storage name"
    );
    let normalized = name.replace('\\', "/");
    let candidate = Path::new(krkrz_core::local_storage_path(&normalized));
    let relative = if candidate.is_absolute() {
        candidate
            .strip_prefix(root)
            .or_else(|_| candidate.strip_prefix(project))
            .context("writable storage is outside the game and save directories")?
    } else {
        candidate
    };
    let relative = krkrz_core::storage_name(&relative.to_string_lossy())?;
    let target = root.join(relative);
    let mut ancestor = target.as_path();
    let mut suffix = vec![];
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(ancestor.file_name().context("invalid save storage path")?);
                ancestor = ancestor.parent().context("invalid save storage ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut resolved = ancestor.canonicalize()?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    ensure!(
        resolved.starts_with(root) && !resolved.starts_with(project),
        "save storage symlink escapes the save directory"
    );
    Ok(resolved)
}

pub(crate) fn write(path: &Path, bytes: &[u8], offset: Option<usize>) -> Result<()> {
    const LIMIT: usize = 512 << 20;
    ensure!(bytes.len() <= LIMIT, "save storage exceeds size limit");
    let parent = path.parent().context("save storage has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(offset) = offset {
        ensure!(
            offset <= LIMIT && bytes.len() <= LIMIT - offset,
            "save offset exceeds limit"
        );
        let metadata =
            std::fs::metadata(path).context("offset writes require an existing save file")?;
        ensure!(
            metadata.len() <= LIMIT as u64,
            "existing save exceeds size limit"
        );
        let mut old = std::fs::read(path)?;
        old.resize(old.len().max(offset + bytes.len()), 0);
        old[offset..offset + bytes.len()].copy_from_slice(bytes);
        output.write_all(&old)?;
    } else {
        output.write_all(bytes)?;
    }
    output.as_file().sync_all()?;
    output.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `oN` is a byte offset, independent of the selected text cipher.
pub(crate) fn offset_mode(mode: &str) -> Result<(String, Option<usize>)> {
    if let Some(start) = mode.find('o') {
        let end = mode[start + 1..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count()
            + start
            + 1;
        let digits = &mode[start + 1..end];
        let offset = if digits.is_empty() {
            0
        } else {
            digits.parse()?
        };
        Ok((format!("{}{}", &mode[..start], &mode[end..]), Some(offset)))
    } else {
        Ok((mode.into(), None))
    }
}
