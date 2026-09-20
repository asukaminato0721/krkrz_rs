mod native_host;
mod presenter;
use anyhow::Result;
use clap::Parser;
use krkrz_runtime::Session;
use std::{io::Write, path::PathBuf};
#[derive(Parser)]
#[command(about = "Rust Kirikiri compatibility engine (experimental)")]
struct Args {
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
    #[arg(long)]
    save_dir: Option<PathBuf>,
    /// Explicitly select Otome Domain's Cx encryption profile.
    #[arg(long)]
    otome_domain: bool,
    #[arg(long, default_value_t = 100_000_000_000)]
    budget: u64,
    /// Write an execution trace to a new JSON file, including on startup failure.
    #[arg(long)]
    trace: Option<PathBuf>,
    /// Fix the Unix wall-clock origin for reproducible runs.
    #[arg(long)]
    epoch_ms: Option<i64>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let mut session = Session::open(
        &args.project_dir,
        args.save_dir.as_deref(),
        args.otome_domain || args.project_dir.join("otomedomain.exe").is_file(),
        args.budget,
    )?;
    session.services.trace_enabled = args.trace.is_some();
    if let Some(epoch) = args.epoch_ms {
        session.services.epoch_ms = epoch;
    }
    let result = native_host::run(&mut session);
    if let Some(path) = args.trace {
        let path = krkrz_core::save_directory(&args.project_dir, Some(&path))?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer_pretty(&mut output, &session.services.trace)?;
        output.write_all(b"\n")?;
    }
    result
}
