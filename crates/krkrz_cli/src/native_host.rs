use crate::{audio_output::AudioOutput, presenter::Presenter};
use anyhow::{Context, Result};
use krkrz_runtime::{InputEvent, Session, display::Monitor, window::WindowState};
use krkrz_tjs::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};

struct NativeWindow {
    window: Arc<Window>,
    presenter: Presenter,
    state: WindowState,
    pointer: [i32; 2],
    modifiers: ModifiersState,
    buttons: u32,
    keys: BTreeSet<u32>,
    pressed_at: Option<[i32; 2]>,
    last_click: Option<(Instant, [i32; 2])>,
    suspended: bool,
}
pub fn run(session: &mut Session, audio_enabled: bool) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut host = Host {
        session,
        audio: if audio_enabled {
            Some(AudioOutput::open()?)
        } else {
            None
        },
        windows: BTreeMap::new(),
        started: false,
        origin: Instant::now(),
        next_tick: Instant::now(),
        error: None,
        messages: 0,
    };
    event_loop.run_app(&mut host)?;
    host.flush_messages();
    host.error.map_or(Ok(()), Err)
}
struct Host<'a> {
    session: &'a mut Session,
    audio: Option<AudioOutput>,
    windows: BTreeMap<usize, NativeWindow>,
    started: bool,
    origin: Instant,
    next_tick: Instant,
    error: Option<anyhow::Error>,
    messages: usize,
}
impl Host<'_> {
    fn flush_messages(&mut self) {
        for message in &self.session.services.messages[self.messages..] {
            eprintln!("{message}");
        }
        self.messages = self.session.services.messages.len();
    }
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error.context(format!(
            "native host at {} ms",
            self.session.services.time_ms
        )));
        event_loop.exit();
    }
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let primary = event_loop.primary_monitor();
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let displays = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let p = m.position();
                let size = m.size();
                let bounds = [p.x, p.y, size.width as i32, size.height as i32];
                Monitor {
                    name: m.name().unwrap_or_else(|| format!("Monitor {i}")),
                    primary: primary.as_ref().map_or(i == 0, |p| p == m),
                    bounds,
                    work: bounds,
                }
            })
            .collect::<Vec<_>>();
        if let Some(primary) = displays.iter().find(|m| m.primary) {
            self.session.services.screen_size =
                (primary.bounds[2] as u32, primary.bounds[3] as u32);
        }
        if !displays.is_empty() {
            self.session.services.set_displays(displays)?;
        }
        self.session.startup()?;
        self.started = true;
        self.origin = Instant::now();
        self.sync_windows(event_loop)
    }
    fn sync_windows(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        self.windows
            .retain(|id, _| self.session.services.windows.contains_key(id));
        for (&id, state) in &self.session.services.windows {
            if !state.constructed {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = self.windows.entry(id) {
                let attributes = Window::default_attributes()
                    .with_title(&state.caption)
                    .with_visible(false)
                    .with_inner_size(PhysicalSize::new(
                        state.inner_width.max(1) as u32,
                        state.inner_height.max(1) as u32,
                    ))
                    .with_position(PhysicalPosition::new(state.left, state.top))
                    .with_decorations(state.border_style != 0)
                    .with_resizable(matches!(state.border_style, 2 | 5));
                let window = Arc::new(event_loop.create_window(attributes)?);
                let presenter = Presenter::new(window.clone())?;
                window.set_ime_allowed(true);
                window.set_visible(state.visible);
                window.set_maximized(state.maximized);
                window.set_minimized(state.minimized);
                if state.is_full_screen() {
                    window.set_fullscreen(Some(Fullscreen::Borderless(window.current_monitor())));
                }
                entry.insert(NativeWindow {
                    window,
                    presenter,
                    state: state.clone(),
                    pointer: [0; 2],
                    modifiers: ModifiersState::empty(),
                    buttons: 0,
                    keys: BTreeSet::new(),
                    pressed_at: None,
                    last_click: None,
                    suspended: false,
                });
            }
            let native = self.windows.get_mut(&id).unwrap();
            if state.caption != native.state.caption {
                native.window.set_title(&state.caption);
            }
            if state.visible != native.state.visible {
                native.window.set_visible(state.visible);
            }
            if state.minimized != native.state.minimized {
                native.window.set_minimized(state.minimized);
            }
            if state.maximized != native.state.maximized {
                native.window.set_maximized(state.maximized);
            }
            if state.border_style != native.state.border_style {
                native.window.set_decorations(state.border_style != 0);
                native
                    .window
                    .set_resizable(matches!(state.border_style, 2 | 5));
            }
            if state.is_full_screen() != native.state.is_full_screen() {
                native.window.set_fullscreen(
                    state
                        .is_full_screen()
                        .then(|| Fullscreen::Borderless(native.window.current_monitor())),
                );
            }
            if (state.inner_width, state.inner_height)
                != (native.state.inner_width, native.state.inner_height)
                && !state.is_full_screen()
            {
                let _ = native.window.request_inner_size(PhysicalSize::new(
                    state.inner_width.max(1) as u32,
                    state.inner_height.max(1) as u32,
                ));
            }
            if (state.left, state.top) != (native.state.left, native.state.top)
                && !state.is_full_screen()
            {
                native
                    .window
                    .set_outer_position(PhysicalPosition::new(state.left, state.top));
            }
            let cursor = self.session.window_cursor(&Value::object(id))?;
            native
                .window
                .set_cursor_visible(state.mouse_cursor_state == 0 && cursor != -1);
            native.window.set_cursor(cursor_icon(cursor));
            native.state = state.clone();
        }
        if self.started && self.windows.is_empty() {
            event_loop.exit();
        }
        Ok(())
    }
    fn event(&mut self, id: usize, event: WindowEvent) -> Result<()> {
        let target = Value::object(id);
        let native = self
            .windows
            .get_mut(&id)
            .context("event window disappeared")?;
        let mut input = Vec::new();
        match event {
            WindowEvent::CloseRequested => self.session.request_window_close(&target)?,
            WindowEvent::Resized(size) => {
                native.suspended = size.width == 0 || size.height == 0;
                if !native.suspended {
                    native.presenter.resize(size.width, size.height);
                    self.session
                        .resize_window(&target, size.width, size.height)?;
                    native.state.inner_width = size.width as i32;
                    native.state.inner_height = size.height as i32;
                }
            }
            WindowEvent::Moved(position) => {
                if let Some(state) = self.session.services.windows.get_mut(&id) {
                    state.left = position.x;
                    state.top = position.y;
                    native.state.left = position.x;
                    native.state.top = position.y;
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => native.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                native.pointer = [position.x as i32, position.y as i32];
                input.push(InputEvent::PointerMove {
                    x: native.pointer[0],
                    y: native.pointer[1],
                    shift: shift(native),
                });
            }
            WindowEvent::CursorLeft { .. } => input.push(InputEvent::PointerLeave),
            WindowEvent::MouseInput { state, button, .. } => {
                let (button, bit) = match button {
                    MouseButton::Left => (0, 8),
                    MouseButton::Right => (1, 16),
                    MouseButton::Middle => (2, 32),
                    _ => return Ok(()),
                };
                let [x, y] = native.pointer;
                if state == ElementState::Pressed {
                    native.buttons |= bit;
                    input.push(InputEvent::PointerDown {
                        x,
                        y,
                        button,
                        shift: shift(native),
                    });
                    if button == 0 {
                        native.pressed_at = Some([x, y]);
                    }
                } else {
                    native.buttons &= !bit;
                    if button == 0
                        && native
                            .pressed_at
                            .take()
                            .is_some_and(|p| (p[0] - x).abs() <= 4 && (p[1] - y).abs() <= 4)
                    {
                        let now = Instant::now();
                        let double = native.last_click.is_some_and(|(time, p)| {
                            now.duration_since(time) < Duration::from_millis(500)
                                && (p[0] - x).abs() <= 4
                                && (p[1] - y).abs() <= 4
                        });
                        input.push(if double {
                            InputEvent::DoubleClick { x, y }
                        } else {
                            InputEvent::Click { x, y }
                        });
                        native.last_click = (!double).then_some((now, [x, y]));
                    }
                    input.push(InputEvent::PointerUp {
                        x,
                        y,
                        button,
                        shift: shift(native),
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 120.,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                input.push(InputEvent::Wheel {
                    x: native.pointer[0],
                    y: native.pointer[1],
                    delta: delta.round() as i32,
                    shift: shift(native),
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(key) = virtual_key(code)
                {
                    let shift = shift(native) | if event.repeat { 128 } else { 0 };
                    if event.state == ElementState::Pressed {
                        native.keys.insert(key);
                        input.push(InputEvent::KeyDown { key, shift });
                    } else {
                        native.keys.remove(&key);
                        input.push(InputEvent::KeyUp { key, shift });
                    }
                }
                if event.state == ElementState::Pressed
                    && let Some(text) = event.text
                    && !text.is_empty()
                {
                    input.push(InputEvent::Text(text.to_string()));
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => input.push(InputEvent::Text(text)),
            WindowEvent::Focused(focused) => {
                if !focused {
                    for key in std::mem::take(&mut native.keys) {
                        input.push(InputEvent::KeyUp { key, shift: 0 });
                    }
                    native.buttons = 0;
                    native.modifiers = ModifiersState::empty();
                    native.pressed_at = None;
                }
                input.push(InputEvent::Focus(focused));
            }
            WindowEvent::RedrawRequested => {
                if !native.suspended
                    && let Some(state) = self.session.services.windows.get(&id)
                    && state.visible
                    && !state.minimized
                    && state.primary_layer != Value::NULL
                {
                    let image = self.session.capture_window(&target)?;
                    if let Some(state) = self.session.services.windows.get(&id) {
                        native.presenter.present(
                            &image,
                            state.draw_rect(image.width as i32, image.height as i32),
                        )?;
                    }
                }
            }
            _ => {}
        }
        for event in input {
            if self.session.services.windows.contains_key(&id) {
                self.session.input(&target, event)?;
            }
        }
        Ok(())
    }
}
impl ApplicationHandler for Host<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.started
            && let Err(error) = self.start(event_loop)
        {
            self.fail(event_loop, error);
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        if let Some(id) = self
            .windows
            .iter()
            .find_map(|(id, n)| (n.window.id() == window).then_some(*id))
            && let Err(error) = self.event(id, event)
        {
            self.fail(event_loop, error);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() || !self.started {
            return;
        }
        if Instant::now() >= self.next_tick {
            let time = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let result = self.session.tick(time).and_then(|_| {
                let samples = self.session.take_audio();
                if let Some(audio) = &self.audio {
                    audio.submit(&samples)?;
                }
                self.sync_windows(event_loop)
            });
            if let Err(error) = result {
                self.fail(event_loop, error);
                return;
            }
            for native in self.windows.values() {
                if native.state.visible && !native.state.minimized && !native.suspended {
                    native.window.request_redraw();
                }
            }
            self.next_tick = Instant::now() + Duration::from_millis(16);
            self.flush_messages();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
    }
}
fn shift(native: &NativeWindow) -> u32 {
    native.buttons
        | u32::from(native.modifiers.shift_key())
        | (u32::from(native.modifiers.alt_key()) * 2)
        | (u32::from(native.modifiers.control_key()) * 4)
}
fn virtual_key(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    Some(match code {
        KeyA => 65,
        KeyB => 66,
        KeyC => 67,
        KeyD => 68,
        KeyE => 69,
        KeyF => 70,
        KeyG => 71,
        KeyH => 72,
        KeyI => 73,
        KeyJ => 74,
        KeyK => 75,
        KeyL => 76,
        KeyM => 77,
        KeyN => 78,
        KeyO => 79,
        KeyP => 80,
        KeyQ => 81,
        KeyR => 82,
        KeyS => 83,
        KeyT => 84,
        KeyU => 85,
        KeyV => 86,
        KeyW => 87,
        KeyX => 88,
        KeyY => 89,
        KeyZ => 90,
        Digit0 => 48,
        Digit1 => 49,
        Digit2 => 50,
        Digit3 => 51,
        Digit4 => 52,
        Digit5 => 53,
        Digit6 => 54,
        Digit7 => 55,
        Digit8 => 56,
        Digit9 => 57,
        F1 => 112,
        F2 => 113,
        F3 => 114,
        F4 => 115,
        F5 => 116,
        F6 => 117,
        F7 => 118,
        F8 => 119,
        F9 => 120,
        F10 => 121,
        F11 => 122,
        F12 => 123,
        Escape => 27,
        Enter | NumpadEnter => 13,
        Space => 32,
        Tab => 9,
        Backspace => 8,
        Delete => 46,
        Insert => 45,
        Home => 36,
        End => 35,
        PageUp => 33,
        PageDown => 34,
        ArrowLeft => 37,
        ArrowUp => 38,
        ArrowRight => 39,
        ArrowDown => 40,
        ShiftLeft | ShiftRight => 16,
        ControlLeft | ControlRight => 17,
        AltLeft | AltRight => 18,
        SuperLeft => 91,
        SuperRight => 92,
        ContextMenu => 93,
        CapsLock => 20,
        NumLock => 144,
        ScrollLock => 145,
        Pause => 19,
        PrintScreen => 44,
        Numpad0 => 96,
        Numpad1 => 97,
        Numpad2 => 98,
        Numpad3 => 99,
        Numpad4 => 100,
        Numpad5 => 101,
        Numpad6 => 102,
        Numpad7 => 103,
        Numpad8 => 104,
        Numpad9 => 105,
        NumpadMultiply => 106,
        NumpadAdd => 107,
        NumpadSubtract => 109,
        NumpadDecimal => 110,
        NumpadDivide => 111,
        Semicolon => 186,
        Equal => 187,
        Comma => 188,
        Minus => 189,
        Period => 190,
        Slash => 191,
        Backquote => 192,
        BracketLeft => 219,
        Backslash => 220,
        BracketRight => 221,
        Quote => 222,
        IntlBackslash => 226,
        _ => return None,
    })
}

fn cursor_icon(cursor: i32) -> winit::window::CursorIcon {
    use winit::window::CursorIcon::*;
    match cursor {
        -3 => Crosshair,
        -4 => Text,
        -5 | -22 => Move,
        -6 => NeswResize,
        -7 => NsResize,
        -8 => NwseResize,
        -9 => EwResize,
        -10 => NResize,
        -11 | -17 => Wait,
        -12 | -16 => Grab,
        -13 => NoDrop,
        -14 => ColResize,
        -15 => RowResize,
        -18 => NotAllowed,
        -19 => Progress,
        -20 => Help,
        -21 => Pointer,
        1 => VerticalText,
        _ => Default,
    }
}
