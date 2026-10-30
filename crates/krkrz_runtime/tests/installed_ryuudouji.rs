//! Checks the external installation without changing its scripts or saves.
use anyhow::{Context, Result, ensure};
use krkrz_assets::storage::Storage;
use krkrz_core::Limits;
use krkrz_runtime::Session;
use std::path::Path;

#[test]
#[ignore = "requires KRKRZ_RYUUDOUJI_PROJECT_DIR and the installed game"]
fn original_mpeg_wrapper_plays_all_installed_movies() -> Result<()> {
    let project = std::env::var_os("KRKRZ_RYUUDOUJI_PROJECT_DIR")
        .context("set KRKRZ_RYUUDOUJI_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut storage = Storage::open(Path::new(&project), None, Limits::default())?;
    storage.detect_cipher()?;
    let movies: Vec<_> = storage
        .catalog
        .keys()
        .filter(|name| name.ends_with(".mpg"))
        .cloned()
        .collect();
    ensure!(movies.len() == 11, "expected the game's 11 MPEG animations");
    let mut session = Session::from_storage(storage, Some(saves.path()), None, 1_000_000_000)?;
    session.startup()?;
    session.evaluate("kag.conductor.stop()")?;
    session.execute_storage("system/gfx_movie.tjs")?;
    session.evaluate("Scripts.exec('global.movie=new MovieControlLayer(kag);global.updates=0;global.sink=%[assignImagesForMovie:function(layer){global.updates++;},notifyStopFlip:function(){}];movie.flipEntry(sink);')")?;
    let mut now = 0;
    for name in movies {
        let name = serde_json::to_string(&name)?;
        session.evaluate(&format!("movie.start({name},false,0)"))?;
        let duration = session.evaluate("movie.totalTime")?.integer()? as u64;
        let frames = session.evaluate("movie.numberOfFrame")?.integer()?;
        ensure!(duration > 0 && frames > 1, "movie has no animation: {name}");
        let first = session.evaluate("movie.targetLayer.getMainPixel(0,0)")?;
        session.tick(now)?;
        let before = session.evaluate("updates")?.integer()?;
        now += 200;
        session.tick(now).with_context(|| format!("play {name}"))?;
        ensure!(
            session.evaluate("movie.frame")?.integer()? > 0,
            "movie frame did not advance: {name}"
        );
        ensure!(
            session.evaluate("updates")?.integer()? > before,
            "original wrapper did not deliver pixels: {name}"
        );
        session.evaluate("movie.pause()")?;
        let paused = session.evaluate("movie.frame")?;
        now += 200;
        session.tick(now)?;
        ensure!(
            session.evaluate("movie.frame")? == paused,
            "pause advanced the movie: {name}"
        );
        session.evaluate("movie.frame=0")?;
        ensure!(
            session.evaluate("movie.targetLayer.getMainPixel(0,0)")? == first,
            "rewind changed the first frame: {name}"
        );
        session.evaluate("movie.play()")?;
        now += duration + 100;
        session
            .tick(now)
            .with_context(|| format!("finish {name}"))?;
        ensure!(
            session.evaluate("movie.status")?.text() == "stop",
            "movie did not stop at EOF: {name}"
        );
        session.evaluate("movie.close()")?;
        eprintln!("played {name}: {frames} frames, {duration} ms");
    }
    Ok(())
}
