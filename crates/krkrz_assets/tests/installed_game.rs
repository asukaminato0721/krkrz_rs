//! Proprietary data stays outside the repository. These tests require explicit opt-in.
use anyhow::{Context, Result};
use krkrz_assets::{
    cx::CxEncryption,
    media::{Audio, Image},
    psb::Document,
    sli::LoopInfo,
    storage::Storage,
    text,
};
use krkrz_core::Limits;
fn game() -> Result<Storage> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR")
        .context("set KRKRZ_PROJECT_DIR to the installed game directory")?;
    Storage::open(
        std::path::Path::new(&project),
        Some(CxEncryption::otome_domain()?),
        Limits::default(),
    )
}
#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn scripts_and_psb_scenarios() -> Result<()> {
    let mut game = game()?;
    let names = game
        .catalog
        .keys()
        .filter(|n| n.ends_with(".tjs") || n.ends_with(".scn"))
        .cloned()
        .collect::<Vec<_>>();
    anyhow::ensure!(!names.is_empty(), "missing game scripts");
    let mut scripts = 0;
    let mut scenarios = 0;
    for name in names {
        game.verify(&name)?;
        let bytes = game.read(&name)?;
        if name.ends_with(".scn") {
            Document::parse(&bytes).with_context(|| name.clone())?;
            scenarios += 1;
        } else {
            text::decode(&bytes).with_context(|| name.clone())?;
            scripts += 1;
        }
    }
    eprintln!("verified and decoded {scripts} source scripts and {scenarios} PSB scenarios");
    Ok(())
}
#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn representative_image_and_bgm() -> Result<()> {
    let mut game = game()?;
    let image = Image::decode(&game.read("image/emotion/ang_1.tlg")?)?;
    assert_eq!((image.width, image.height), (100, 4600));
    let audio = Audio::decode_vorbis(&game.read("bgm/bgm01.ogg")?, 10_000_000)?;
    assert_eq!(
        (audio.channels, audio.sample_rate, audio.frames()),
        (2, 44100, 6715771)
    );
    let loops = LoopInfo::parse(&text::decode(&game.read("bgm/bgm01.ogg.sli")?)?)?;
    loops.validate_frames(audio.frames() as u64)?;
    assert_eq!((loops.links[0].from, loops.links[0].to), (5970432, 254240));
    assert!(loops.links[0].smooth);
    Ok(())
}
#[test]
#[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
fn loop_metadata() -> Result<()> {
    let mut game = game()?;
    let names = game
        .catalog
        .keys()
        .filter(|n| n.ends_with(".sli"))
        .cloned()
        .collect::<Vec<_>>();
    anyhow::ensure!(!names.is_empty(), "missing game loop metadata");
    let (mut links, mut labels) = (0, 0);
    for name in &names {
        let info =
            LoopInfo::parse(&text::decode(&game.read(name)?)?).with_context(|| name.clone())?;
        links += info.links.len();
        labels += info.labels.len();
    }
    eprintln!(
        "parsed {} SLI files, {links} links, {labels} labels",
        names.len()
    );
    Ok(())
}
