//! Profile installed Otome Domain gallery tabs using native frame scheduling.
//! Run with KRKRZ_PROJECT_DIR set and cargo run --release -p krkrz_runtime
//! --example gallery_profile. Requires gallery-unlocked saves; copies their KSD
//! files to a temporary directory, leaving the installation unchanged.
use anyhow::{Context, Result, ensure};
use krkrz_runtime::{InputEvent, Session, WindowFrameState};
use std::{path::Path, time::Instant};
fn main() -> Result<()> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut s = Session::open(
        Path::new(&project),
        Some(saves.path()),
        Some(krkrz_assets::cx::CxEncryption::otome_domain().unwrap()),
        1_000_000_000_000,
    )?;
    for entry in std::fs::read_dir(Path::new(&project).join("savedata"))? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "ksd") {
            std::fs::copy(entry.path(), saves.path().join(entry.file_name()))?;
        }
    }
    s.services.epoch_ms = 0;
    s.startup()?;
    let w = s.evaluate("kag")?;
    let mut state = WindowFrameState::default();
    let mut times = Vec::new();
    let mut expected = "kag.currentStorage == 'title.ks' && kag.inStable";
    for time in (0..=92000).step_by(16) {
        let start = Instant::now();
        if s.vm.should_collect_garbage() {
            s.collect_garbage(std::slice::from_ref(&w))?;
        }
        let gc = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let mut completed = false;
        s.tick_with_frame_sink(time, |_, _| completed = true)?;
        s.take_audio();
        let tick = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let changed = if completed {
            state = WindowFrameState::default();
            true
        } else {
            s.capture_window_if_changed(&w, &mut state)?.is_some()
        };
        let capture = start.elapsed().as_secs_f64() * 1000.;
        times.push((time, gc, tick, capture, changed));
        if gc + tick + capture > 100. {
            eprintln!("STALL {time} gc={gc:.2} tick={tick:.2} capture={capture:.2}");
        }
        if time >= 26000 && (time - 26000) % 6000 == 0 {
            ensure!(
                s.evaluate(expected)?.truth()?,
                "unexpected gallery state at {time}: {expected}"
            );
            let index = ((time - 26000) / 6000) as usize;
            let tabs = [
                "title",
                "tab_music",
                "tab_scene",
                "tab_sdcg",
                "tab_stand",
                "tab_cg",
                "tab_music",
                "tab_scene",
                "tab_sdcg",
                "tab_stand",
                "tab_cg",
            ];
            if index >= tabs.len() {
                continue;
            }
            let tab = tabs[index];
            expected = match tab {
                "title" | "tab_cg" => {
                    "kag.currentStorage == 'cgmode.ks' && kag.inStable && !tf.sdmode"
                }
                "tab_sdcg" => "kag.currentStorage == 'cgmode.ks' && kag.inStable && tf.sdmode",
                "tab_music" => {
                    "kag.currentStorage == 'scenemode.ks' && kag.inStable && tf.moviemode && SoundModeInstance !== void"
                }
                "tab_scene" => {
                    "kag.currentStorage == 'scenemode.ks' && kag.inStable && !tf.moviemode"
                }
                "tab_stand" => "kag.currentStorage == 'exchview.ks' && kag.inStable",
                _ => unreachable!(),
            };
            eprintln!(
                "STATE {} {} -> {tab}",
                time,
                s.evaluate("kag.currentStorage")?.text()
            );
            let (x, y) = if index == 0 {
                (630, 650)
            } else {
                let x = s
                    .evaluate(&format!("kag.current.names['{tab}'].left + 20"))?
                    .integer()? as i32;
                let y = s
                    .evaluate(&format!("kag.current.names['{tab}'].top + 20"))?
                    .integer()? as i32;
                (x, y)
            };
            let start = Instant::now();
            for e in [
                InputEvent::PointerMove { x, y, shift: 0 },
                InputEvent::PointerDown {
                    x,
                    y,
                    button: 0,
                    shift: 8,
                },
                InputEvent::Click { x, y },
                InputEvent::PointerUp {
                    x,
                    y,
                    button: 0,
                    shift: 0,
                },
            ] {
                s.input(&w, e)?;
            }
            eprintln!(
                "CLICK {time} {:.2} ms",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    }
    ensure!(
        s.evaluate(expected)?.truth()?,
        "unexpected final gallery state: {expected}"
    );
    for (lo, hi) in
        std::iter::once((0, 26000)).chain((26000..92000).step_by(6000).map(|lo| (lo, lo + 6000)))
    {
        let t: Vec<_> = times.iter().filter(|t| t.0 >= lo && t.0 < hi).collect();
        let mut sorted: Vec<_> = t.iter().map(|t| t.1 + t.2 + t.3).collect();
        sorted.sort_by(f64::total_cmp);
        eprintln!(
            "RANGE {lo}..{hi} mean {:.2} p95 {:.2} max {:.2} changed {}",
            sorted.iter().sum::<f64>() / sorted.len() as f64,
            sorted[sorted.len() * 95 / 100],
            sorted.last().unwrap(),
            t.iter().filter(|t| t.4).count()
        );
    }
    times.sort_by(|a, b| (b.1 + b.2 + b.3).total_cmp(&(a.1 + a.2 + a.3)));
    for t in times.iter().take(25) {
        eprintln!("SLOW {t:?}");
    }
    Ok(())
}
