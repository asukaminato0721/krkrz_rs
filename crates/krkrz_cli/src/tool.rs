use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use krkrz_assets::{
    cx::CxEncryption,
    media::{Audio, Image},
    sli::LoopInfo,
    storage::Storage,
    text,
};
use krkrz_core::Limits;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    path::PathBuf,
};
#[derive(Clone, Copy, ValueEnum)]
enum Profile {
    None,
    OtomeDomain,
}
#[derive(Parser)]
#[command(about = "Inspect and verify Kirikiri resources directly from XP3 archives")]
struct Args {
    #[arg(long, default_value = ".", global = true)]
    project_dir: PathBuf,
    #[arg(long, value_enum, default_value = "none", global = true)]
    profile: Profile,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// List resolved resources after archive patch overrides.
    List {
        #[arg(long)]
        contains: Option<String>,
    },
    /// Read complete resources and check their decrypted Adler-32 checksums.
    Verify {
        #[arg(long)]
        contains: Option<String>,
    },
    /// Extract one resource to an explicit, new file outside the installation.
    Extract { name: String, output: PathBuf },
    /// Decode a source script (UTF-16, UTF-8 or Shift-JIS).
    Script { name: String },
    /// Inspect a PSB v2 container as JSON.
    Psb { name: String },
    /// Inspect KAG tokens and attribute order as JSON.
    Kag { name: String },
    /// Compile the supported TJS subset to diagnostic register instructions.
    Compile { name: String },
    /// Evaluate a TJS primitive expression with a bounded interpreter.
    Eval {
        expression: String,
        #[arg(long, default_value_t = 10000)]
        budget: u64,
    },
    /// Decode an image and write a new PNG outside the game installation.
    Image { name: String, output: PathBuf },
    /// Decode Vorbis and report channel, sample-rate and frame counts.
    Audio {
        name: String,
        #[arg(long, default_value_t = 28_800_000)]
        max_frames: usize,
    },
    /// Inspect SLI loop links, conditions and sample-frame labels.
    Loops { name: String },
    /// Report archive contents and static script/plugin references; does not execute scripts.
    Inventory,
}
fn main() -> Result<()> {
    match run() {
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        result => result,
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    let cipher = match args.profile {
        Profile::None => None,
        Profile::OtomeDomain => Some(CxEncryption::otome_domain()?),
    };
    let mut storage = Storage::open(&args.project_dir, cipher, Limits::default())?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    match args.command {
        Command::Loops { name } => serde_json::to_writer_pretty(
            &mut out,
            &LoopInfo::parse(&text::decode(&storage.read(&name)?)?)?,
        )?,
        Command::List { contains } => {
            for (name, index) in &storage.catalog {
                if contains.as_ref().is_none_or(|s| name.contains(s)) {
                    let archive = &storage.archives[*index];
                    let entry = &archive.entries[name];
                    writeln!(
                        out,
                        "{}\t{}\t{}\t{}",
                        entry.size,
                        if entry.encrypted { "cx" } else { "raw" },
                        archive
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        name
                    )?;
                }
            }
        }
        Command::Verify { contains } => {
            let names = storage
                .catalog
                .keys()
                .filter(|n| contains.as_ref().is_none_or(|s| n.contains(s)))
                .cloned()
                .collect::<Vec<_>>();
            anyhow::ensure!(!names.is_empty(), "no matching resources");
            let mut failures = 0;
            for name in &names {
                if let Err(e) = storage.verify(name) {
                    failures += 1;
                    eprintln!("FAIL {name}: {e:#}");
                }
            }
            writeln!(out, "verified {} resources; {failures} failed", names.len())?;
            out.flush()?;
            anyhow::ensure!(failures == 0, "verification failed");
        }
        Command::Extract { name, output } => storage.extract(&name, &output)?,
        Command::Psb { name } => serde_json::to_writer_pretty(
            &mut out,
            &krkrz_assets::psb::Document::parse(&storage.read(&name)?)?,
        )?,
        Command::Kag { name } => serde_json::to_writer_pretty(
            &mut out,
            &krkrz_kag::Scenario::parse(&name, &text::decode(&storage.read(&name)?)?)?,
        )?,
        Command::Compile { name } => serde_json::to_writer_pretty(
            &mut out,
            &krkrz_tjs::compile(&name, &text::decode(&storage.read(&name)?)?)?,
        )?,
        Command::Eval {
            expression,
            mut budget,
        } => {
            let program = krkrz_tjs::compile_expression("<command>", &expression)?;
            let value = krkrz_tjs::Vm::default().execute(&program, &mut (), &mut budget)?;
            serde_json::to_writer_pretty(&mut out, &value)?;
        }
        Command::Script { name } => write!(out, "{}", text::decode(&storage.read(&name)?)?)?,
        Command::Image { name, output } => {
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."))
                .canonicalize()?;
            anyhow::ensure!(
                !parent.starts_with(&storage.project),
                "image output must be outside the game installation"
            );
            let image = Image::decode(&storage.read(&name)?)?;
            image.write_png(&output)?;
            writeln!(
                out,
                "{} x {} RGBA: {}",
                image.width,
                image.height,
                output.display()
            )?;
        }
        Command::Audio { name, max_frames } => {
            let audio = Audio::decode_vorbis(&storage.read(&name)?, max_frames)?;
            serde_json::to_writer_pretty(
                &mut out,
                &serde_json::json!({"channels":audio.channels,"sample_rate":audio.sample_rate,"frames":audio.frames()}),
            )?;
        }
        Command::Inventory => {
            let mut extensions = BTreeMap::<String, usize>::new();
            let mut total_bytes = 0u64;
            let mut script_bytes = 0u64;
            let mut scripts = vec![];
            let mut plugins = BTreeSet::new();
            let mut calls = BTreeSet::new();
            let mut lex_errors = BTreeMap::new();
            for name in storage.catalog.keys() {
                let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
                *extensions.entry(ext.to_owned()).or_default() += 1;
                total_bytes += storage.entry(name)?.1.size;
                if ext == "tjs" {
                    scripts.push(name.clone());
                    script_bytes += storage.entry(name)?.1.size;
                }
            }
            for name in &scripts {
                let bytes = storage.read(name)?;
                if bytes.starts_with(b"TJS2") {
                    continue;
                }
                let source = text::decode(&bytes).with_context(|| format!("decode {name}"))?;
                let tokens = match krkrz_tjs::lexer::lex(name, &source) {
                    Ok(tokens) => tokens,
                    Err(error) => {
                        lex_errors.insert(name.clone(), error.to_string());
                        Vec::new()
                    }
                };
                for token in tokens {
                    if let krkrz_tjs::lexer::Kind::Literal(krkrz_tjs::Value::String(s)) = token.kind
                    {
                        let s = String::from_utf16_lossy(&s);
                        if s.to_ascii_lowercase().ends_with(".dll") {
                            plugins.insert(s);
                        }
                    }
                }
                // Candidates only: a full startup trace is required to establish reachability.
                for prefix in ["Plugins.", "Scripts.", "Storages.", "System.", "Debug."] {
                    for (_, suffix) in source
                        .match_indices(prefix)
                        .map(|(i, s)| (s, &source[i + prefix.len()..]))
                    {
                        let name = suffix
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect::<String>();
                        if !name.is_empty() {
                            calls.insert(format!("{prefix}{name}"));
                        }
                    }
                }
            }
            serde_json::to_writer_pretty(
                &mut out,
                &serde_json::json!({"script_lex_errors":lex_errors,"resolved_entries":storage.catalog.len(),"declared_bytes":total_bytes,"extensions":extensions,"source_scripts":scripts.len(),"script_bytes":script_bytes,"plugin_string_candidates":plugins,"native_reference_candidates":calls,"archives":storage.archives.iter().map(|a|serde_json::json!({"name":a.path.file_name().unwrap_or_default().to_string_lossy(),"entries":a.entries.len()})).collect::<Vec<_>>() }),
            )?;
        }
    }
    out.flush()?;
    Ok(())
}
