mod audio_output;
mod file_dialog;
mod file_dialog_egui;
mod input_dialog;
mod native_host;
mod presenter;
mod project;
mod shell_execute;
use anyhow::Result;
use clap::Parser;
use krkrz_runtime::Session;
use std::{io::Write, path::PathBuf};
#[derive(Parser)]
#[command(about = "Rust Kirikiri compatibility engine (experimental)")]
struct Args {
    #[arg(long, hide = true)]
    native_dialog: bool,
    #[arg(long, hide = true)]
    file_dialog: bool,
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
    /// Save directory (defaults to <project-dir>/savedata, including existing saves).
    #[arg(long)]
    save_dir: Option<PathBuf>,
    #[command(flatten)]
    project: project::ProjectArgs,
    #[arg(long, default_value_t = 100_000_000_000)]
    budget: u64,
    /// Write an execution trace to a new JSON file, including on startup failure.
    #[arg(long)]
    trace: Option<PathBuf>,
    /// Fix the Unix wall-clock origin for reproducible runs.
    #[arg(long)]
    epoch_ms: Option<i64>,
    /// Disable the audio device while continuing the script audio clock.
    #[arg(long)]
    no_audio: bool,
    /// Override the game's executable icon (ICO, PNG, JPEG, BMP or TLG).
    #[arg(long)]
    icon: Option<PathBuf>,
}
fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();
    if args.native_dialog {
        return input_dialog::run_child();
    }
    if args.file_dialog {
        return file_dialog::run_child();
    }
    let storage = args.project.open(&args.project_dir)?;
    let mut session = Session::from_storage(
        storage,
        args.save_dir.as_deref(),
        args.project.exe.as_deref(),
        args.budget,
    )?;
    session.services.trace_enabled = args.trace.is_some();
    if let Some(epoch) = args.epoch_ms {
        session.services.epoch_ms = epoch;
    }
    let icon = native_host::project_icon(
        &args.project_dir,
        args.icon.as_deref(),
        args.project.exe.as_deref(),
    )?;
    let result = native_host::run(&mut session, !args.no_audio, icon);
    if let Some(path) = args.trace {
        let path = krkrz_core::save_directory(&args.project_dir, Some(&path))?;
        anyhow::ensure!(
            !path.starts_with(args.project_dir.canonicalize()?),
            "trace output must be outside the game installation"
        );
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer_pretty(&mut output, &session.services.trace)?;
        output.write_all(b"\n")?;
    }
    result
}
