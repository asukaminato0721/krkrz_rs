//! Profile installed Otome Domain clicks using native frame scheduling.
//! Run with KRKRZ_PROJECT_DIR set and cargo run --release -p krkrz_runtime
//! --example transition_profile. Saves use a fresh temporary directory.
use anyhow::{Context, Result};
use krkrz_runtime::{InputEvent, Session, WindowFrameState};
use std::{path::Path, time::Instant};
fn main() -> Result<()> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut s = Session::open(
        Path::new(&project),
        Some(saves.path()),
        true,
        1_000_000_000_000,
    )?;
    s.services.epoch_ms = 0;
    s.startup()?;
    let w = s.evaluate("kag")?;
    let mut state = WindowFrameState::default();
    let mut times = Vec::new();
    for time in (0..=46000).step_by(16) {
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
        if [26000, 41008, 44000].contains(&time) {
            let (x, y) = if time == 26000 {
                (110, 650)
            } else {
                (640, 500)
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
    for (lo, hi) in [(0, 26000), (26000, 41008), (41008, 46001)] {
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
    eprintln!(
        "SCENE {} TEXT {}",
        s.evaluate("world_object.player.curSceneName")?.text(),
        s.evaluate("world_object.player.curTextId")?.text()
    );
    Ok(())
}
