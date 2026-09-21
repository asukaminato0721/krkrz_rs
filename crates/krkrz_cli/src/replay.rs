//! Deterministic host input, using the same Session entry points as the native host.
use anyhow::{Context, Result, ensure};
use krkrz_runtime::{InputEvent, Session};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
struct Entry {
    at_ms: u64,
    #[serde(default = "main_window")]
    window: String,
    #[serde(flatten)]
    action: Action,
}

fn main_window() -> String {
    "Window.mainWindow".into()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    PointerMove {
        x: i32,
        y: i32,
        #[serde(default)]
        shift: u32,
    },
    PointerDown {
        x: i32,
        y: i32,
        #[serde(default)]
        button: i32,
        #[serde(default)]
        shift: u32,
    },
    PointerUp {
        x: i32,
        y: i32,
        #[serde(default)]
        button: i32,
        #[serde(default)]
        shift: u32,
    },
    Click {
        x: i32,
        y: i32,
    },
    DoubleClick {
        x: i32,
        y: i32,
    },
    PointerLeave,
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
        #[serde(default)]
        shift: u32,
    },
    KeyDown {
        key: u32,
        #[serde(default)]
        shift: u32,
    },
    KeyUp {
        key: u32,
        #[serde(default)]
        shift: u32,
    },
    Text {
        text: String,
    },
    Focus {
        focused: bool,
    },
    Checkpoint {
        name: String,
        #[serde(default)]
        inspect: Vec<String>,
        frame: Option<PathBuf>,
    },
    Assert {
        expression: String,
    },
}

pub struct Replay {
    entries: Vec<Entry>,
}

impl Replay {
    pub fn read(path: &Path) -> Result<Self> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("read replay {}", path.display()))?;
        let mut entries: Vec<Entry> = serde_json::from_str(&source)
            .with_context(|| format!("parse replay {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        for entry in &mut entries {
            if let Action::Checkpoint {
                frame: Some(frame), ..
            } = &mut entry.action
            {
                *frame = base.join(&frame);
            }
        }
        Ok(Self { entries })
    }

    /// Check the whole replay before running startup or emitting any input.
    pub fn validate(&self, session: &Session, end: Option<u64>) -> Result<()> {
        let mut previous = session.services.time_ms;
        let mut frames = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            ensure!(
                entry.at_ms >= previous,
                "replay action {index}: timestamps must be nondecreasing"
            );
            previous = entry.at_ms;
            ensure!(
                !entry.window.trim().is_empty(),
                "replay action {index}: empty window expression"
            );
            match &entry.action {
                Action::PointerDown { button, .. } | Action::PointerUp { button, .. } => {
                    ensure!(
                        (0..=2).contains(button),
                        "replay action {index}: button must be 0 (left), 1 (right), or 2 (middle)"
                    );
                }
                Action::Checkpoint { name, frame, .. } => {
                    ensure!(
                        !name.trim().is_empty(),
                        "replay action {index}: empty checkpoint name"
                    );
                    if let Some(frame) = frame {
                        let path = frame_output(session, frame)?;
                        ensure!(
                            frames.insert(path),
                            "replay action {index}: duplicate frame output"
                        );
                    }
                }
                Action::Assert { expression } => {
                    ensure!(
                        !expression.trim().is_empty(),
                        "replay action {index}: empty assertion"
                    );
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            ensure!(
                end >= previous,
                "--advance-ms must be at least the final replay timestamp ({previous})"
            );
        }
        Ok(())
    }

    pub fn run(
        self,
        session: &mut Session,
        step_ms: u64,
        roots: &[krkrz_tjs::Value],
    ) -> Result<()> {
        let mut output = std::io::stderr().lock();
        self.run_with_output(session, step_ms, roots, &mut output)
    }

    fn run_with_output(
        self,
        session: &mut Session,
        step_ms: u64,
        roots: &[krkrz_tjs::Value],
        output: &mut impl Write,
    ) -> Result<()> {
        ensure!(step_ms > 0, "replay step must be positive");
        tick(session, session.services.time_ms, roots)?;
        for (index, entry) in self.entries.into_iter().enumerate() {
            let description = format!("replay action {index} at {} ms", entry.at_ms);
            advance(session, entry.at_ms, step_ms, roots).with_context(|| description.clone())?;
            apply(session, entry, output).with_context(|| description)?;
        }
        Ok(())
    }
}

fn apply(session: &mut Session, entry: Entry, output: &mut impl Write) -> Result<()> {
    let event = match entry.action {
        Action::PointerMove { x, y, shift } => InputEvent::PointerMove { x, y, shift },
        Action::PointerDown {
            x,
            y,
            button,
            shift,
        } => InputEvent::PointerDown {
            x,
            y,
            button,
            shift,
        },
        Action::PointerUp {
            x,
            y,
            button,
            shift,
        } => InputEvent::PointerUp {
            x,
            y,
            button,
            shift,
        },
        Action::Click { x, y } => InputEvent::Click { x, y },
        Action::DoubleClick { x, y } => InputEvent::DoubleClick { x, y },
        Action::PointerLeave => InputEvent::PointerLeave,
        Action::Wheel { x, y, delta, shift } => InputEvent::Wheel { x, y, delta, shift },
        Action::KeyDown { key, shift } => InputEvent::KeyDown { key, shift },
        Action::KeyUp { key, shift } => InputEvent::KeyUp { key, shift },
        Action::Text { text } => InputEvent::Text(text),
        Action::Focus { focused } => InputEvent::Focus(focused),
        Action::Assert { expression } => {
            let value = session.evaluate(&expression)?;
            ensure!(
                value.truth()?,
                "replay assertion failed: {expression}; got {value:?}"
            );
            return Ok(());
        }
        Action::Checkpoint {
            name,
            inspect,
            frame,
        } => {
            let mut values = Vec::new();
            for expression in inspect {
                let value = session.evaluate(&expression)?;
                values.push(serde_json::json!({"expression": expression, "value": value}));
            }
            if let Some(frame) = &frame {
                capture(session, &entry.window, frame)?;
            }
            serde_json::to_writer(
                &mut *output,
                &serde_json::json!({
                    "checkpoint": name, "at_ms": session.services.time_ms,
                    "inspect": values, "frame": frame,
                }),
            )?;
            writeln!(output)?;
            output.flush()?;
            return Ok(());
        }
    };
    let window = session.evaluate(&entry.window)?;
    session.input(&window, event)
}

pub fn tick(session: &mut Session, at_ms: u64, roots: &[krkrz_tjs::Value]) -> Result<()> {
    if session.vm.should_collect_garbage() {
        session.collect_garbage(roots)?;
    }
    session.tick(at_ms).map_err(|error| {
        for message in session.services.messages.iter().rev().take(8).rev() {
            eprintln!("{message}");
        }
        error.context(format!(
            "session at {} ms (requested tick {at_ms})",
            session.services.time_ms
        ))
    })
}

pub fn advance(
    session: &mut Session,
    end: u64,
    step_ms: u64,
    roots: &[krkrz_tjs::Value],
) -> Result<()> {
    ensure!(step_ms > 0, "host tick step must be positive");
    ensure!(
        end >= session.services.time_ms,
        "session clock cannot move backwards"
    );
    while session.services.time_ms < end {
        tick(
            session,
            session.services.time_ms.saturating_add(step_ms).min(end),
            roots,
        )?;
    }
    Ok(())
}

fn frame_output(session: &Session, output: &Path) -> Result<PathBuf> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&session.services.storage.project),
        "frame output must be outside the game installation"
    );
    let path = parent.join(
        output
            .file_name()
            .context("frame output needs a file name")?,
    );
    ensure!(
        !path.try_exists()?,
        "frame output already exists: {}",
        path.display()
    );
    Ok(path)
}

pub fn capture(session: &mut Session, window: &str, output: &Path) -> Result<()> {
    let output = frame_output(session, output)?;
    let window = session.evaluate(window)?;
    session.capture_window(&window)?.write_png(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use krkrz_tjs::Value;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Session) {
        let project = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("startup.tjs"), r#"
            var events = [];
            class TestWindow extends Window {
                function TestWindow() { super.Window(); visible = true; setInnerSize(20, 10); }
                function onKeyDown(key, shift) { events.add("key:" + key + ":" + System.getTickCount()); }
                function onClick(x, y) { events.add("click:" + x + ":" + y + ":" + System.getTickCount()); }
            }
            var w = new TestWindow();
            var layer = new Layer(w, null); layer.setSize(20, 10); layer.visible = true;
        "#).unwrap();
        let mut session =
            Session::open(project.path(), Some(output.path()), None, 1_000_000).unwrap();
        session.startup().unwrap();
        (project, output, session)
    }

    fn read(dir: &Path, source: &str) -> Result<Replay> {
        let path = dir.join("replay.json");
        std::fs::write(&path, source)?;
        Replay::read(&path)
    }

    #[test]
    fn exact_time_input_order_checkpoints_and_frames() {
        let (_project, output, mut session) = fixture();
        let replay = read(output.path(), r#"[
            {"at_ms": 19, "window":"w", "action":"key_down", "key":13},
            {"at_ms": 19, "window":"w", "action":"click", "x":4, "y":5},
            {"at_ms": 20, "window":"w", "action":"key_up", "key":13},
            {"at_ms": 23, "action":"assert", "expression":"events.count == 2"},
            {"at_ms": 23, "window":"w", "action":"checkpoint", "name":"after input", "inspect":["events.join('|')"], "frame":"frame.png"}
        ]"#).unwrap();
        replay.validate(&session, Some(23)).unwrap();
        let mut checkpoints = Vec::new();
        replay
            .run_with_output(&mut session, 16, &[], &mut checkpoints)
            .unwrap();
        assert_eq!(session.services.time_ms, 23);
        assert_eq!(
            session.evaluate("events.join('|')").unwrap(),
            Value::string("key:13:19|click:4:5:19")
        );
        let record: serde_json::Value = serde_json::from_slice(&checkpoints).unwrap();
        assert_eq!(record["checkpoint"], "after input");
        assert_eq!(record["at_ms"], 23);
        let frame = krkrz_assets::media::Image::decode(
            &std::fs::read(output.path().join("frame.png")).unwrap(),
        )
        .unwrap();
        assert_eq!((frame.width, frame.height), (20, 10));
    }

    #[test]
    fn rejects_bad_schema_and_time_before_input() {
        let (_project, output, session) = fixture();
        for source in [
            r#"[{"at_ms":1,"action":"key_down","key":13,"shfit":8}]"#,
            r#"[{"at_ms":-1,"action":"click","x":0,"y":0}]"#,
            r#"[{"at_ms":1,"action":"click","x":0}]"#,
            r#"[{"at_ms":1,"action":"typo"}]"#,
        ] {
            assert!(read(output.path(), source).is_err(), "accepted {source}");
        }
        let replay = read(
            output.path(),
            r#"[
            {"at_ms":10,"action":"click","x":0,"y":0},
            {"at_ms":9,"action":"click","x":0,"y":0}
        ]"#,
        )
        .unwrap();
        assert!(replay.validate(&session, None).is_err());
        let replay = read(
            output.path(),
            r#"[{"at_ms":10,"action":"click","x":0,"y":0}]"#,
        )
        .unwrap();
        assert!(replay.validate(&session, Some(9)).is_err());
        assert!(replay.validate(&session, Some(10)).is_ok());
    }

    #[test]
    fn refuses_existing_and_installation_frame_outputs() {
        let (project, output, session) = fixture();
        assert!(frame_output(&session, &project.path().join("frame.png")).is_err());
        std::fs::write(output.path().join("existing.png"), "keep").unwrap();
        assert!(frame_output(&session, &output.path().join("existing.png")).is_err());
        let replay = read(
            output.path(),
            r#"[
            {"at_ms":0,"action":"checkpoint","name":"one","frame":"new.png"},
            {"at_ms":1,"action":"checkpoint","name":"two","frame":"./new.png"}
        ]"#,
        )
        .unwrap();
        assert!(replay.validate(&session, None).is_err());
    }

    #[test]
    fn failed_assertion_preserves_prior_checkpoint_and_stops_later_actions() {
        let (_project, output, mut session) = fixture();
        let replay = read(
            output.path(),
            r#"[
            {"at_ms":0,"action":"checkpoint","name":"before"},
            {"at_ms":1,"action":"assert","expression":"false"},
            {"at_ms":2,"window":"w","action":"click","x":0,"y":0}
        ]"#,
        )
        .unwrap();
        let mut checkpoints = Vec::new();
        let error = replay
            .run_with_output(&mut session, 16, &[], &mut checkpoints)
            .unwrap_err();
        assert!(format!("{error:#}").contains("replay action 1 at 1 ms"));
        assert_eq!(session.evaluate("events.count").unwrap(), Value::Integer(0));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&checkpoints).unwrap()["checkpoint"],
            "before"
        );
    }
}
