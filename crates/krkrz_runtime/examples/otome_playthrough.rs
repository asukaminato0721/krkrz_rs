//! Installed-game acceptance runner. Uses normal New Game, held Ctrl, and choice clicks.
//! Usage: XDG_CACHE_HOME=/tmp/unique-cache cargo run --release -p krkrz_runtime
//! --example otome_playthrough -- PROJECT SAVE_DIR OUTPUT_DIR kaz|yuz|hin [LIMIT_MS]
//! All outputs must be separate from the original installation and saves.
use anyhow::{Context, Result, bail, ensure};
use krkrz_runtime::{InputEvent, Session};
use serde_json::json;
use std::{collections::BTreeSet, io::Write, path::Path, time::Instant};

fn event(session: &mut Session, event: InputEvent) -> Result<()> {
    let window = session.evaluate("Window.mainWindow")?;
    session.input(&window, event)
}

fn click(session: &mut Session, x: i32, y: i32) -> Result<()> {
    for input in [
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
        event(session, input)?;
    }
    Ok(())
}

fn control(session: &mut Session, down: bool) -> Result<()> {
    event(
        session,
        if down {
            InputEvent::KeyDown { key: 17, shift: 2 }
        } else {
            InputEvent::KeyUp { key: 17, shift: 0 }
        },
    )
}

fn capture(session: &mut Session, directory: &Path, name: &str) -> Result<()> {
    let window = session.evaluate("Window.mainWindow")?;
    session
        .capture_window(&window)?
        .write_png(&directory.join(name))
}

fn record(output: &mut impl Write, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *output, &value)?;
    writeln!(output)?;
    output.flush()?;
    eprintln!("{value}");
    Ok(())
}

fn run(
    session: &mut Session,
    route: &str,
    limit: u64,
    directory: &Path,
    output: &mut impl Write,
) -> Result<()> {
    session.services.epoch_ms = 0;
    session.startup()?;
    let started = Instant::now();
    let mut scenes = BTreeSet::new();
    let mut launched = false;
    let mut held = false;
    let mut last_choice = String::new();
    let mut choice_time = 0;
    let mut last_progress = String::new();
    let mut last_report_ms = 0;
    let mut reached_route = false;
    let route_prefix = match route {
        "kaz" => "ra",
        "yuz" => "rb",
        "hin" => "rc",
        _ => unreachable!(),
    };
    for at in (0..=limit).step_by(16) {
        if session.vm.should_collect_garbage() {
            session.collect_garbage(&[])?;
        }
        session
            .tick(at)
            .with_context(|| format!("playthrough tick {at} ms"))?;
        if at % 256 != 0 {
            continue;
        }
        let state = session.evaluate(
            "[kag.currentStorage,kag.currentLabel,kag.inStable,kag.selectShowing,\
             typeof world_object != 'undefined' && world_object.player ? world_object.player.curSceneName : '',\
             typeof world_object != 'undefined' && world_object.player ? world_object.player.curTextId : ''].join('\t')"
        )?.text();
        let fields = state.split('\t').collect::<Vec<_>>();
        ensure!(fields.len() == 6, "unexpected playthrough state: {state:?}");
        let scene = fields[4];
        let stable = fields[2] == "1";
        let selecting = fields[3] == "1";
        if scene.starts_with(route_prefix) {
            reached_route = true;
        }
        if state != last_progress || at - last_report_ms >= 5000 {
            record(
                output,
                json!({"event":"progress","at_ms":at,"wall_seconds":started.elapsed().as_secs(),
                "storage":fields[0],"label":fields[1],"stable":stable,"selecting":selecting,
                "scene":scene,"text_id":fields[5],"remaining_budget":session.budget}),
            )?;
            last_progress = state.clone();
            last_report_ms = at;
        }
        if fields[0] == "title.ks" && fields[1] == "*wait" {
            if !launched {
                capture(session, directory, "title.png")?;
                click(session, 110, 650)?;
                launched = true;
                continue;
            }
            if !scenes.is_empty() {
                if held {
                    control(session, false)?;
                }
                capture(session, directory, "returned-title.png")?;
                ensure!(
                    reached_route,
                    "returned to title before reaching requested route {route}"
                );
                record(
                    output,
                    json!({"event":"returned_title","at_ms":at,"route":route,"scenes":scenes}),
                )?;
                return Ok(());
            }
        }
        if launched && !scene.is_empty() && scenes.insert(scene.to_owned()) {
            capture(
                session,
                directory,
                &format!("scene-{:03}.png", scenes.len()),
            )?;
            record(output, json!({"event":"scene","at_ms":at,"scene":scene}))?;
        }
        if selecting && stable && scene != last_choice {
            if held {
                control(session, false)?;
                held = false;
            }
            let index = if scene.starts_with("ky04_2.txt*dummyselect1") {
                usize::from(route != "kaz")
            } else if scene.starts_with("ky02_2.txt")
                || scene.starts_with("ky03_1.txt")
                || scene.starts_with("ky04_1.txt")
            {
                usize::from(route == "hin")
            } else {
                0
            };
            let coordinates = session.evaluate(&format!(
                "(function(){{var b=kag.selectLayer.selects[{index}];return b && b.nodeVisible && b.nodeEnabled ? [int(b.left+b.width/2),int(b.top+b.height/2)].join(',') : '';}})()"
            ))?.text();
            if !coordinates.is_empty() {
                let (x, y) = coordinates.split_once(',').context("choice coordinates")?;
                capture(session, directory, &format!("choice-{at}.png"))?;
                click(session, x.parse()?, y.parse()?)?;
                last_choice = scene.to_owned();
                choice_time = at;
                record(
                    output,
                    json!({"event":"choice","at_ms":at,"scene":scene,"index":index}),
                )?;
            }
        } else if launched
            && !scene.is_empty()
            && stable
            && !selecting
            && !held
            && at > choice_time + 1000
        {
            control(session, true)?;
            held = true;
        }
    }
    if held {
        control(session, false)?;
    }
    capture(session, directory, "limit.png")?;
    bail!(
        "playthrough limit reached before returning to title; visited {} scenes",
        scenes.len()
    )
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        (4..=5).contains(&args.len()),
        "usage: otome_playthrough PROJECT SAVE_DIR OUTPUT_DIR kaz|yuz|hin [LIMIT_MS]"
    );
    ensure!(
        ["kaz", "yuz", "hin"].contains(&args[3].as_str()),
        "unknown route"
    );
    let project = Path::new(&args[0]).canonicalize()?;
    std::fs::create_dir_all(&args[2])?;
    let directory = Path::new(&args[2]).canonicalize()?;
    ensure!(
        !directory.starts_with(&project),
        "outputs must be outside the game installation"
    );
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("progress.jsonl"))?;
    let limit = args
        .get(4)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(3_600_000);
    let mut session = Session::open(&project, Some(Path::new(&args[1])), true, 1_000_000_000_000)?;
    session.services.trace_enabled = false;
    let result = run(&mut session, &args[3], limit, &directory, &mut output);
    if let Err(error) = &result {
        record(
            &mut output,
            json!({"event":"error","at_ms":session.services.time_ms,"error":format!("{error:#}")}),
        )?;
        let _ = capture(&mut session, &directory, "error.png");
        for message in session.services.messages.iter().rev().take(12).rev() {
            eprintln!("{message}");
        }
    }
    result
}
