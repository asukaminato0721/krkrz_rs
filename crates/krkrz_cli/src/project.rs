//! Shared project configuration for the native host and inspection tool.
use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use krkrz_assets::{cx::CxEncryption, storage::Storage};
use krkrz_core::Limits;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, ValueEnum)]
pub enum Profile {
    #[default]
    Auto,
    None,
    OtomeDomain,
}

#[derive(Args)]
pub struct ProjectArgs {
    /// Archive cipher selection. Auto validates known profiles against archive checksums.
    #[arg(long, value_enum, default_value = "auto", global = true)]
    pub profile: Profile,
    /// Load a Cx encryption profile from JSON.
    #[arg(long, global = true, conflicts_with_all = ["profile", "otome_domain"])]
    pub cx_profile: Option<PathBuf>,
    /// Original executable used for System.exeName and the window icon (never executed).
    #[arg(long, global = true)]
    pub exe: Option<PathBuf>,
    /// Select the built-in Otome Domain cipher explicitly (legacy alias).
    #[arg(long, global = true, conflicts_with = "profile")]
    pub otome_domain: bool,
}

impl ProjectArgs {
    pub fn open(&self, project: &Path) -> Result<Storage> {
        let cipher = if let Some(path) = &self.cx_profile {
            Some(CxEncryption::from_json(
                &std::fs::read_to_string(path)
                    .with_context(|| format!("read Cx profile {}", path.display()))?,
            )?)
        } else if self.otome_domain || matches!(self.profile, Profile::OtomeDomain) {
            Some(CxEncryption::otome_domain()?)
        } else {
            None
        };
        let mut storage = Storage::open(project, cipher, Limits::default())?;
        if self.cx_profile.is_none() && !self.otome_domain && matches!(self.profile, Profile::Auto)
        {
            storage.detect_cipher().context("automatic archive cipher detection; use --cx-profile FILE for another Cx profile, or --profile none for unencrypted archives")?;
        }
        Ok(storage)
    }
}
