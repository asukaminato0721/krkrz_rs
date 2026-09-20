use anyhow::Result;
use clap::Parser;
use krkrz_runtime::Session;
use std::{io::Write, path::PathBuf};
#[derive(Parser)]
#[command(about = "Experimental Rust Kirikiri startup executor (playable slice not yet supported)")]
struct Args {
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
    #[arg(long)]
    save_dir: Option<PathBuf>,
    /// Explicitly select Otome Domain's Cx encryption profile.
    #[arg(long)]
    otome_domain: bool,
    #[arg(long, default_value_t = 1_000_000)]
    budget: u64,
    /// Write an execution trace to a new JSON file, including on startup failure.
    #[arg(long)]
    trace: Option<PathBuf>,
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
    let result = session.startup();
    for message in &session.services.messages {
        eprintln!("{message}");
    }
    if let Some(path) = args.trace {
        let path = krkrz_core::save_directory(&args.project_dir, Some(&path))?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        serde_json::to_writer_pretty(&mut output, &session.services.trace)?;
        output.write_all(b"\n")?;
    }
    result?;
    anyhow::bail!(
        "startup script returned; native presentation and the gameplay event loop are not implemented"
    )
}
