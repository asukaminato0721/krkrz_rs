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
