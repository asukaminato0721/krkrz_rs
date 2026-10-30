use super::Delivery;
use gilrs::{Axis, Button, EventType, Gilrs, GilrsBuilder};
use krkrz_runtime::{InputEvent, gamepad::PadKey};
use std::collections::{BTreeMap, BTreeSet};

pub struct Gamepads {
    backend: Option<Gilrs>,
    state: State,
}

impl Gamepads {
    pub fn new() -> Self {
        let backend = match GilrsBuilder::new().with_force_feedback(false).build() {
            Ok(backend) => Some(backend),
            Err(error) => {
                eprintln!("Could not initialize gamepad input: {error}");
                None
            }
        };
        Self {
            backend,
            state: State::default(),
        }
    }

    pub fn poll(&mut self, target: Option<usize>, now: u64) -> Vec<Delivery> {
        let mut events = self.state.set_target(target, now);
        if let Some(backend) = &mut self.backend {
            while let Some(event) = backend.next_event() {
                let id = usize::from(event.id);
                match event.event {
                    EventType::ButtonPressed(button, _) | EventType::ButtonReleased(button, _) => {
                        if let Some(key) = button_key(button) {
                            self.state
                                .pads
                                .entry(id)
                                .or_default()
                                .button(key, matches!(event.event, EventType::ButtonPressed(..)));
                        }
                    }
                    EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                        self.state
                            .pads
                            .entry(id)
                            .or_default()
                            .axis(StickAxis::X, value);
                    }
                    EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                        self.state
                            .pads
                            .entry(id)
                            .or_default()
                            .axis(StickAxis::Y, value);
                    }
                    EventType::Disconnected => {
                        self.state.pads.remove(&id);
                    }
                    _ => {}
                }
                // Preserve a press and release that both arrive in this frame.
                events.extend(self.state.sync(now));
            }
            backend.inc();
        }
        events.extend(self.state.repeat(now));
        events
    }

    pub fn release_window(&mut self, window: usize) -> Vec<Delivery> {
        if self.state.target == Some(window) {
            self.state.set_target(None, 0)
        } else {
            Vec::new()
        }
    }
}

fn button_key(button: Button) -> Option<PadKey> {
    use PadKey::*;
    Some(match button {
        Button::South => Button1,
        Button::East => Button2,
        Button::West => Button3,
        Button::North => Button4,
        Button::LeftTrigger => Button5,
        Button::RightTrigger => Button6,
        Button::LeftTrigger2 => Button7,
        Button::RightTrigger2 => Button8,
        Button::Select => Button9,
        Button::Start => Button10,
        Button::DPadLeft => Left,
        Button::DPadUp => Up,
        Button::DPadRight => Right,
        Button::DPadDown => Down,
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum StickAxis {
    X,
    Y,
}

#[derive(Default)]
struct PadState {
    buttons: BTreeSet<PadKey>,
    stick: [Option<PadKey>; 2],
}

impl PadState {
    fn button(&mut self, key: PadKey, pressed: bool) {
        if pressed {
            self.buttons.insert(key);
        } else {
            self.buttons.remove(&key);
        }
    }

    fn axis(&mut self, axis: StickAxis, value: f32) {
        let (index, negative, positive) = match axis {
            StickAxis::X => (0, PadKey::Left, PadKey::Right),
            // Gilrs normalizes positive Y as up on each platform.
            StickAxis::Y => (1, PadKey::Down, PadKey::Up),
        };
        let previous = self.stick[index];
        // Use separate press/release thresholds to prevent drift and chatter.
        self.stick[index] = if !value.is_finite() {
            None
        } else if value <= -0.55 || (previous == Some(negative) && value < -0.35) {
            Some(negative)
        } else if value >= 0.55 || (previous == Some(positive) && value > 0.35) {
            Some(positive)
        } else {
            None
        };
    }

    fn keys(&self) -> impl Iterator<Item = PadKey> + '_ {
        self.buttons
            .iter()
            .copied()
            .chain(self.stick.iter().flatten().copied())
    }
}

#[derive(Default)]
struct State {
    pads: BTreeMap<usize, PadState>,
    target: Option<usize>,
    // Each key has one owner, even when several controls hold it.
    delivered: BTreeMap<PadKey, u64>,
}

impl State {
    fn set_target(&mut self, target: Option<usize>, now: u64) -> Vec<Delivery> {
        let mut events = Vec::new();
        if self.target != target {
            if let Some(window) = self.target {
                for key in self.delivered.keys() {
                    events.push(Delivery {
                        window,
                        event: InputEvent::KeyUp {
                            key: key.key_code(),
                            shift: 0,
                        },
                    });
                }
            }
            self.delivered.clear();
            self.target = target;
            events.extend(self.sync(now));
        }
        events
    }

    fn sync(&mut self, now: u64) -> Vec<Delivery> {
        let mut events = Vec::new();
        if let Some(window) = self.target {
            let held: BTreeSet<_> = self.pads.values().flat_map(PadState::keys).collect();
            self.delivered.retain(|key, _| {
                if held.contains(key) {
                    true
                } else {
                    events.push(Delivery {
                        window,
                        event: InputEvent::KeyUp {
                            key: key.key_code(),
                            shift: 0,
                        },
                    });
                    false
                }
            });
            for key in held {
                if let std::collections::btree_map::Entry::Vacant(entry) = self.delivered.entry(key)
                {
                    entry.insert(now.saturating_add(500));
                    events.push(Delivery {
                        window,
                        event: InputEvent::KeyDown {
                            key: key.key_code(),
                            shift: 0,
                        },
                    });
                }
            }
        }
        events
    }

    fn repeat(&mut self, now: u64) -> Vec<Delivery> {
        let mut events = Vec::new();
        if let Some(window) = self.target {
            for (&key, next) in &mut self.delivered {
                if now >= *next {
                    // Emit at most once per host frame after a stalled dialog.
                    *next = now.saturating_add(30);
                    events.push(Delivery {
                        window,
                        event: InputEvent::KeyDown {
                            key: key.key_code(),
                            shift: 128,
                        },
                    });
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(events: Vec<Delivery>) -> Vec<(usize, u32, bool, u32)> {
        events
            .into_iter()
            .map(|delivery| match delivery.event {
                InputEvent::KeyDown { key, shift } => (delivery.window, key, true, shift),
                InputEvent::KeyUp { key, shift } => (delivery.window, key, false, shift),
                _ => panic!("pad input must produce key events"),
            })
            .collect()
    }

    #[test]
    fn shared_controls_and_disconnect_preserve_the_last_owner() {
        let mut s = State::default();
        s.set_target(Some(10), 0);
        s.pads.entry(1).or_default().button(PadKey::Left, true);
        assert_eq!(events(s.sync(0)), [(10, 0x1b5, true, 0)]);
        s.pads.get_mut(&1).unwrap().axis(StickAxis::X, -1.0);
        s.pads.entry(2).or_default().button(PadKey::Left, true);
        assert!(s.sync(10).is_empty());
        s.pads.get_mut(&1).unwrap().button(PadKey::Left, false);
        assert!(s.sync(20).is_empty());
        s.pads.remove(&2);
        assert!(s.sync(30).is_empty());
        s.pads.remove(&1);
        assert_eq!(events(s.sync(40)), [(10, 0x1b5, false, 0)]);
        assert!(s.repeat(1000).is_empty());
    }

    #[test]
    fn stick_hysteresis_releases_before_reversing_and_supports_diagonals() {
        let mut s = State::default();
        s.set_target(Some(10), 0);
        s.pads.entry(1).or_default().axis(StickAxis::X, -0.2);
        assert!(s.sync(0).is_empty());
        s.pads.get_mut(&1).unwrap().axis(StickAxis::X, -0.6);
        assert_eq!(events(s.sync(10)), [(10, 0x1b5, true, 0)]);
        s.pads.get_mut(&1).unwrap().axis(StickAxis::X, -0.4);
        assert!(s.sync(20).is_empty());
        s.pads.get_mut(&1).unwrap().axis(StickAxis::X, 0.8);
        assert_eq!(
            events(s.sync(30)),
            [(10, 0x1b5, false, 0), (10, 0x1b7, true, 0)]
        );
        s.pads.get_mut(&1).unwrap().axis(StickAxis::Y, 1.0);
        assert_eq!(events(s.sync(40)), [(10, 0x1b6, true, 0)]);
        s.pads.get_mut(&1).unwrap().axis(StickAxis::X, 0.3);
        s.pads.get_mut(&1).unwrap().axis(StickAxis::Y, f32::NAN);
        assert_eq!(
            events(s.sync(50)),
            [(10, 0x1b6, false, 0), (10, 0x1b7, false, 0)]
        );
    }

    #[test]
    fn focus_changes_release_old_window_and_background_taps_stay_in_background() {
        let mut s = State::default();
        s.set_target(Some(10), 0);
        s.pads.entry(1).or_default().button(PadKey::Button1, true);
        assert_eq!(events(s.sync(0)), [(10, 0x1c0, true, 0)]);
        assert_eq!(events(s.set_target(None, 1)), [(10, 0x1c0, false, 0)]);
        assert!(s.repeat(1000).is_empty());
        s.pads.get_mut(&1).unwrap().button(PadKey::Button1, false);
        assert!(s.sync(1001).is_empty());
        s.pads.get_mut(&1).unwrap().button(PadKey::Button2, true);
        assert!(s.sync(1002).is_empty());
        s.pads.get_mut(&1).unwrap().button(PadKey::Button2, false);
        assert!(s.sync(1003).is_empty());
        assert!(s.set_target(Some(20), 1004).is_empty());
    }

    #[test]
    fn short_taps_and_repeat_deadlines_preserve_event_order() {
        let mut s = State::default();
        s.set_target(Some(10), 0);
        s.pads.entry(1).or_default().button(PadKey::Button1, true);
        let mut tap = s.sync(100);
        s.pads.get_mut(&1).unwrap().button(PadKey::Button1, false);
        tap.extend(s.sync(100));
        assert_eq!(events(tap), [(10, 0x1c0, true, 0), (10, 0x1c0, false, 0)]);
        s.pads.get_mut(&1).unwrap().button(PadKey::Button1, true);
        s.sync(200);
        assert!(s.repeat(699).is_empty());
        assert_eq!(events(s.repeat(700)), [(10, 0x1c0, true, 128)]);
        assert!(s.repeat(729).is_empty());
        assert_eq!(events(s.repeat(730)), [(10, 0x1c0, true, 128)]);
        assert_eq!(events(s.repeat(5000)), [(10, 0x1c0, true, 128)]);
        assert!(s.repeat(5000).is_empty());
        assert_eq!(
            events(s.set_target(Some(20), 5010)),
            [(10, 0x1c0, false, 0), (20, 0x1c0, true, 0)]
        );
        assert!(s.repeat(5509).is_empty());
    }

    #[test]
    #[ignore = "requires the installed Otome Domain game and KRKRZ_PROJECT_DIR"]
    fn original_gamepad_navigates_title_and_advances_dialogue() -> anyhow::Result<()> {
        use anyhow::Context;
        use krkrz_runtime::Session;
        use krkrz_tjs::Value;

        fn advance(
            session: &mut Session,
            window: &Value,
            start: u64,
            end: u64,
        ) -> anyhow::Result<()> {
            for time in (start..end).step_by(16) {
                if session.vm.should_collect_garbage() {
                    session.collect_garbage(std::slice::from_ref(window))?;
                }
                session.tick(time)?;
            }
            session.tick(end)
        }

        fn tap(
            state: &mut State,
            session: &mut Session,
            button: Button,
            now: u64,
        ) -> anyhow::Result<()> {
            let key = button_key(button).context("test button must have a pad mapping")?;
            for pressed in [true, false] {
                state.pads.entry(1).or_default().button(key, pressed);
                for delivery in state.sync(now) {
                    session.input(&Value::object(delivery.window), delivery.event)?;
                }
                // Model a host frame while the physical button remains held.
                // KAG checks its current pad state during focus callbacks.
                session.dispatch_events()?;
            }
            Ok(())
        }

        let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
        let saves = tempfile::tempdir()?;
        let cipher = krkrz_assets::cx::CxEncryption::otome_domain()?.into();
        let mut session = Session::open(
            std::path::Path::new(&project),
            Some(saves.path()),
            Some(cipher),
            100_000_000_000,
        )?;
        session.services.epoch_ms = 0;
        session.startup()?;
        let window = session.evaluate("kag")?;
        let Value::Object(reference) = &window else {
            anyhow::bail!("game must create a Window")
        };
        let mut state = State::default();
        state.set_target(reference.object, 0);
        advance(&mut session, &window, 0, 26000)?;
        assert_eq!(
            session.evaluate("kag.currentStorage")?,
            Value::string("title.ks")
        );
        let unselected = session.capture_window(&window)?;
        tap(&mut state, &mut session, Button::DPadRight, 26000)?;
        assert_eq!(
            session.evaluate("kag.focusedLayer !== null")?,
            Value::Integer(1)
        );
        let selected = session.capture_window(&window)?;
        assert_eq!(
            session.evaluate("kag.focusedLayer.links[0].object.Butt_mouseOn")?,
            Value::Integer(1)
        );
        if let Some(directory) = std::env::var_os("KRKRZ_GAMEPAD_FRAME_DIR") {
            let directory = std::path::Path::new(&directory);
            unselected.write_png(&directory.join("title-unselected.png"))?;
            selected.write_png(&directory.join("title-selected.png"))?;
        }
        assert!(
            unselected.rgba != selected.rgba,
            "pad navigation must highlight the selected title button"
        );
        tap(&mut state, &mut session, Button::DPadRight, 26000)?;
        let next_selected = session.capture_window(&window)?;
        assert_eq!(session.evaluate("!kag.focusedLayer.links[0].object.Butt_mouseOn && kag.focusedLayer.links[1].object.Butt_mouseOn")?, Value::Integer(1));
        assert!(
            selected.rgba != next_selected.rgba,
            "the highlight must move to Load Game"
        );
        tap(&mut state, &mut session, Button::DPadLeft, 26000)?;
        assert_eq!(
            selected,
            session.capture_window(&window)?,
            "returning to New Game must restore its highlight"
        );
        tap(&mut state, &mut session, Button::South, 26000)?;
        advance(&mut session, &window, 26016, 41000)?;
        assert_eq!(
            session.evaluate("kag.currentStorage")?,
            Value::string("start.ks")
        );
        assert_eq!(
            session.evaluate("world_object.player.curTextId")?,
            Value::Integer(1)
        );
        tap(&mut state, &mut session, Button::South, 41000)?;
        advance(&mut session, &window, 41016, 44000)?;
        assert_eq!(
            session.evaluate("world_object.player.curTextId")?,
            Value::Integer(2)
        );
        Ok(())
    }
}
