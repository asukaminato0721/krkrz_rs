use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::PathBuf,
};

#[derive(Default)]
pub(crate) struct AppLocks {
    files: Vec<File>,
}

impl AppLocks {
    pub fn acquire(&mut self, name: &str) -> Result<bool> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
            .context("application locks require XDG_CACHE_HOME or HOME")?;
        let directory = base.join("krkrz_rs/app-locks");
        std::fs::create_dir_all(&directory)?;
        let name = name.split('\0').next().unwrap_or_default();
        // Keep lock files after release: unlinking would allow two independent
        // file handles to lock different inodes under the same name.
        let path = directory.join(format!("{:x}", Sha256::digest(name.as_bytes())));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => {
                self.files.push(file);
                Ok(true)
            }
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }
}
