use anyhow::{Context, Result};
use krkrz_assets::{cx::CxEncryption, media::Audio, sli::LoopInfo, storage::Storage, text};
use krkrz_core::Limits;
use krkrz_runtime::audio::SoundStream;
use std::{path::Path, sync::Arc};

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn bgm_crossfade_is_independent_of_host_buffer_size() -> Result<()> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let mut storage = Storage::open(
        Path::new(&project),
        Some(CxEncryption::otome_domain()?),
        Limits::default(),
    )?;
    let audio = Arc::new(Audio::decode_vorbis(
        &storage.read("bgm/bgm01.ogg")?,
        10_000_000,
    )?);
    let info = LoopInfo::parse(&text::decode(&storage.read("bgm/bgm01.ogg.sli")?)?)?;
    let (from, to) = (info.links[0].from, info.links[0].to);
    let mut whole = SoundStream::new(audio.clone(), info.clone())?;
    whole.seek(from - 1200)?;
    let expected = whole.render(2400)?;
    assert_eq!(whole.position(), to + 1200);
    for size in [1, 257, 4096] {
        let mut split = SoundStream::new(audio.clone(), info.clone())?;
        split.seek(from - 1200)?;
        let mut actual = Vec::new();
        let mut remaining = 2400;
        while remaining > 0 {
            let count = size.min(remaining);
            actual.extend(split.render(count)?.samples);
            remaining -= count;
        }
        assert_eq!(actual, expected.samples);
        assert_eq!(split.position(), whole.position());
    }
    Ok(())
}

/// This tests only the startup-to-framework handoff. The host deliberately stops
/// there; it is not a successful framework or gameplay checkpoint.
#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn original_startup_reaches_framework_handoff() -> Result<()> {
    use krkrz_tjs::{Host, Value, Vm, VmAbort, compile, unsupported};
    struct Handoff {
        storage: Option<String>,
    }
    impl Host for Handoff {
        fn call(&mut self, _: &mut Vm, name: &str, args: &[Value], _: &mut u64) -> Result<Value> {
            if name == "Scripts.execStorage" {
                self.storage = args.first().map(Value::text);
                return Err(unsupported("test stopped at framework handoff"));
            }
            anyhow::bail!("unexpected native call during startup handoff test: {name}")
        }
    }
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let mut storage = Storage::open(
        Path::new(&project),
        Some(CxEncryption::otome_domain()?),
        Limits::default(),
    )?;
    let source = text::decode(&storage.read("startup.tjs")?)?;
    let program = compile("startup.tjs", &source)?;
    let mut vm = Vm::default();
    vm.register_native("Plugins.link")?;
    vm.register_native("Scripts.execStorage")?;
    let mut host = Handoff { storage: None };
    let error = vm.execute(&program, &mut host, &mut 10_000).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert_eq!(host.storage.as_deref(), Some("system/Initialize.tjs"));
    Ok(())
}
