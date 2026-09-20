use anyhow::{Context, Result};
use krkrz_assets::{cx::CxEncryption, media::Audio, sli::LoopInfo, storage::Storage, text};
use krkrz_core::Limits;
use krkrz_runtime::audio::SoundStream;
use std::{path::Path, sync::Arc};

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn original_startup_completes_framework_initialization() -> Result<()> {
    use krkrz_runtime::Session;
    use krkrz_tjs::Value;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut session = Session::open(Path::new(&project), Some(saves.path()), true, 2_000_000_000)?;
    session.startup()?;
    assert!(session.budget > 0);
    assert_eq!(session.services.fonts.face_count(), 8);
    assert_eq!(
        session.evaluate("kag instanceof 'KAGWindow'")?,
        Value::Integer(1)
    );
    assert_eq!(
        session.evaluate("kag.fore.base instanceof 'Layer'")?,
        Value::Integer(1)
    );
    for time in (0..=10000).step_by(16) {
        session.tick(time)?;
    }
    session.tick(10000)?;
    assert_eq!(
        session.evaluate("kag.currentStorage")?,
        Value::string("custom.ks")
    );
    let window = session.evaluate("kag")?;
    let frame = session.capture_window(&window)?;
    assert_eq!((frame.width, frame.height), (1280, 720));
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[..3] != frame.rgba[..3])
    );
    assert_eq!(
        session.evaluate("int(kag.inTransition)")?,
        Value::Integer(0)
    );
    assert!(session.budget > 0);
    Ok(())
}

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn embedded_true_type_and_cff_fonts_rasterize_japanese() -> Result<()> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let mut storage = Storage::open(
        Path::new(&project),
        Some(CxEncryption::otome_domain()?),
        Limits::default(),
    )?;
    let mut book = krkrz_runtime::fonts::FontBook::default();
    let names: Vec<_> = storage
        .catalog
        .keys()
        .filter(|name| name.ends_with(".ttf") || name.ends_with(".otf"))
        .cloned()
        .collect();
    assert_eq!(names.len(), 8);
    for name in names {
        assert_eq!(book.add(storage.read(&name)?.to_vec())?, 1, "{name}");
    }
    assert_eq!(book.face_count(), 8);
    for face in [
        "モトヤLシータ゛3等幅",
        "モトヤLマルベリ3等幅",
        "源ノ角ゴシック JP Regular",
        "源ノ角ゴシック JP Heavy",
    ] {
        let glyph = book.rasterize(face, 'あ', 32.0)?;
        assert!(glyph.coverage.iter().any(|v| *v != 0), "{face}");
        assert!(
            glyph.advance > 0.0 && glyph.width <= 64 && glyph.height <= 64,
            "{face}"
        );
    }
    Ok(())
}

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

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn native_sound_reads_archive_pcm_and_sli() -> Result<()> {
    use krkrz_runtime::Session;
    use krkrz_tjs::Value;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut session = Session::open(Path::new(&project), Some(saves.path()), true, 10_000)?;
    session
        .evaluate(r#"Scripts.exec('var w=new WaveSoundBuffer(null); w.open("bgm/bgm01.ogg");')"#)?;
    let sound = session.evaluate("w")?;
    assert_eq!(session.evaluate("w.frequency")?, Value::Integer(44_100));
    let audio = Arc::new(Audio::decode_vorbis(
        &session.services.storage.read("bgm/bgm01.ogg")?,
        10_000_000,
    )?);
    let info = LoopInfo::parse(&text::decode(
        &session.services.storage.read("bgm/bgm01.ogg.sli")?,
    )?)?;
    let (from, to) = (info.links[0].from, info.links[0].to);
    let mut expected = SoundStream::new(audio, info)?;
    expected.seek(from - 1200)?;
    session.evaluate(&format!("w.samplePosition={}", from - 1200))?;
    session.evaluate("w.play()")?;
    assert_eq!(
        session.render_sound_source(&sound, 2400)?,
        expected.render(2400)?
    );
    assert_eq!(
        session.evaluate("w.samplePosition")?,
        Value::Integer((to + 1200) as i64)
    );
    Ok(())
}

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn original_storage_data_reads_binary_scenario() -> Result<()> {
    use krkrz_assets::psb::{Document, Value as Psb};
    use krkrz_runtime::Session;
    use krkrz_tjs::Value;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut session = Session::open(Path::new(&project), Some(saves.path()), true, 1_000_000)?;
    session.evaluate("Plugins.link('psbfile.dll')")?;
    session.evaluate("Plugins.link('ScriptsEx.dll')")?;
    session.execute_storage("system/storagedata.tjs")?;
    let Psb::Object(root) =
        Document::parse(&session.services.storage.read("scn/ra01_0.txt.scn")?)?.root
    else {
        anyhow::bail!("scenario root is not a dictionary");
    };
    let Psb::List(scenes) = &root["scenes"] else {
        anyhow::bail!("scenario scenes is not an array");
    };
    session.evaluate(r#"Scripts.exec('var scenario=new StorageData("ra01_0.txt","scn/");')"#)?;
    assert_eq!(
        session.evaluate("scenario.getScenes().count")?,
        Value::Integer(scenes.len() as i64)
    );
    let Psb::Object(first) = &scenes[0] else {
        anyhow::bail!("first scene is not a dictionary");
    };
    let Psb::String(label) = &first["label"] else {
        anyhow::bail!("first scene has no label");
    };
    let Psb::List(texts) = &first["texts"] else {
        anyhow::bail!("first scene has no text array");
    };
    assert_eq!(
        session.evaluate("scenario.findScene('').label")?,
        Value::string(label)
    );
    assert_eq!(
        session.evaluate("scenario.getScenes()[0].textCount")?,
        Value::Integer(texts.len() as i64)
    );
    Ok(())
}

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn original_text_render_wrapper_uses_native_layout() -> Result<()> {
    use krkrz_runtime::Session;
    use krkrz_tjs::Value;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut session = Session::open(Path::new(&project), Some(saves.path()), true, 100_000)?;
    session.execute_storage("system/textrender.tjs")?;
    // Fixed metrics isolate the unchanged wrapper/native interface. This does
    // not validate fonts, glyph pixels, or presentation of the game's dialogue.
    session.evaluate(r#"Scripts.exec('var renderer=new TextRender();var font=%[getTextWidth:function(s){return s.length*10;}];renderer.render(25,200,font,"ABC");')"#)?;
    assert_eq!(session.evaluate("renderer.renderCount")?, Value::Integer(3));
    assert_eq!(session.evaluate("renderer.renderLines")?, Value::Integer(2));
    assert_eq!(
        session.evaluate("renderer.renderText")?,
        Value::string("AB\nC\n")
    );
    assert_eq!(
        session.evaluate("renderer.getCharacters(0,0)[2].y")?,
        Value::Real(30.0)
    );
    Ok(())
}

#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn original_voice_track_constructs_with_phase_vocoder() -> Result<()> {
    use krkrz_runtime::Session;
    use krkrz_tjs::Value;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut session = Session::open(Path::new(&project), Some(saves.path()), true, 100_000)?;
    session.evaluate("Plugins.link('getSample.dll')")?;
    session.execute_storage("system/voice.tjs")?;
    session.evaluate("Scripts.exec('global.voice=new VoiceSoundBuffer(%[],0);')")?;
    assert_eq!(session.evaluate("voice.status")?, Value::string("unload"));
    assert_eq!(
        session.evaluate("voice.vocoder.window")?,
        Value::Integer(256)
    );
    assert_eq!(session.evaluate("voice.filters.count")?, Value::Integer(0));
    session.evaluate("invalidate voice")?;
    Ok(())
}
