//! Host events use the same LayerManager dispatch path in native and replay hosts.
use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub capture: Option<usize>,
    pub release_capture: bool,
    hover: Option<usize>,
    hit: bool,
    pub keys: std::collections::BTreeSet<u32>,
    pub pressed: std::collections::BTreeSet<u32>,
}
#[derive(Clone, Debug)]
pub enum InputEvent {
    PointerMove {
        x: i32,
        y: i32,
        shift: u32,
    },
    PointerDown {
        x: i32,
        y: i32,
        button: i32,
        shift: u32,
    },
    PointerUp {
        x: i32,
        y: i32,
        button: i32,
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
        shift: u32,
    },
    KeyDown {
        key: u32,
        shift: u32,
    },
    KeyUp {
        key: u32,
        shift: u32,
    },
    Text(String),
    Focus(bool),
}
pub(crate) fn window_event_keys(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "onResize" | "onMouseEnter" | "onMouseLeave" | "onActivate" | "onDeactivate" => &[],
        "onClick" | "onDoubleClick" => &["x", "y"],
        "onMouseDown" | "onMouseUp" => &["x", "y", "button", "shift"],
        "onMouseMove" => &["x", "y", "shift"],
        "onMouseWheel" => &["shift", "delta", "x", "y"],
        "onKeyDown" | "onKeyUp" => &["key", "shift"],
        "onKeyPress" => &["key"],
        _ => return None,
    })
}
fn numbers(values: &[i64]) -> Vec<Value> {
    values.iter().copied().map(Value::Integer).collect()
}
impl Services {
    pub(super) fn layer_input_default(
        &mut self,
        vm: &mut Vm,
        id: usize,
        name: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<()> {
        let Some(layer) = self.layers.get(&id) else {
            return Ok(());
        };
        let (window, root, parent) = (layer.window, layer.root, layer.parent);
        match name {
            "onHitTest" => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.input.hit = args.get(2).context("onHitTest requires hit")?.truth()?;
                }
            }
            "onKeyDown" | "onKeyUp" | "onKeyPress" => {
                let process_index = if name == "onKeyPress" { 1 } else { 2 };
                if !args
                    .get(process_index)
                    .map(Value::truth)
                    .transpose()?
                    .unwrap_or(true)
                {
                    return Ok(());
                }
                let key = if name == "onKeyPress" {
                    args[0].text().encode_utf16().next().unwrap_or(0) as i64
                } else {
                    args[0].integer()?
                };
                let shift = if name == "onKeyPress" {
                    0
                } else {
                    args[1].integer()?
                };
                if name == "onKeyDown"
                    && ((matches!(key, 9 | 39 | 40) && shift & 7 == 0)
                        || (key == 9 && shift & 7 == 1)
                        || matches!(key, 37 | 38))
                {
                    self.layer_step_focus(
                        vm,
                        root,
                        !(matches!(key, 37 | 38) || key == 9 && shift & 1 != 0),
                        budget,
                    )?;
                } else if matches!(key, 13 | 27)
                    && shift & 7 == 0
                    && let Some(parent) = parent
                    && self.layer_node_enabled(parent, false)
                {
                    self.layer_event(vm, parent, name, args, budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn host_hit(
        &mut self,
        vm: &mut Vm,
        id: usize,
        point: [i32; 2],
        budget: &mut u64,
        depth: usize,
    ) -> Result<Option<Option<usize>>> {
        ensure!(depth < 128, "input layer depth exceeds limit");
        *budget = budget
            .checked_sub(1)
            .context("input hit test budget exhausted")?;
        let Some(l) = self.layers.get(&id) else {
            return Ok(None);
        };
        let [x, y] = point;
        if !l.visible || x < 0 || y < 0 || x >= l.width || y >= l.height {
            return Ok(None);
        }
        let children = l.children.clone();
        for child in children.into_iter().rev() {
            if let Some(c) = self.layers.get(&child) {
                let p = [x.saturating_sub(c.left), y.saturating_sub(c.top)];
                if let Some(hit) = self.host_hit(vm, child, p, budget, depth + 1)? {
                    return Ok(Some(hit));
                }
            }
        }
        let Some(l) = self.layers.get(&id) else {
            return Ok(None);
        };
        let px = x.saturating_sub(l.image_left);
        let py = y.saturating_sub(l.image_top);
        let hit = match l.hit_type {
            0 => l.image.as_ref().map_or(l.hit_threshold <= 0, |image| {
                px >= 0
                    && py >= 0
                    && px < image.width as i32
                    && py < image.height as i32
                    && i32::from(
                        image.rgba[(py as usize * image.width as usize + px as usize) * 4 + 3],
                    ) >= l.hit_threshold
            }),
            1 => l.province.as_ref().is_some_and(|p| {
                px >= 0
                    && py >= 0
                    && (px as usize) < p.width
                    && (py as usize) < p.height
                    && p.pixels[py as usize * p.width + px as usize] != 0
            }),
            _ => true,
        };
        if !hit {
            return Ok(None);
        }
        let window = l.window;
        self.windows.get_mut(&window).unwrap().input.hit = true;
        self.layer_event(
            vm,
            id,
            "onHitTest",
            &numbers(&[x.into(), y.into(), 1]),
            budget,
        )?;
        if !self.windows.get(&window).is_some_and(|w| w.input.hit) || !self.layers.contains_key(&id)
        {
            return Ok(None);
        }
        Ok(Some(self.layer_node_enabled(id, false).then_some(id)))
    }
    fn host_target(
        &mut self,
        vm: &mut Vm,
        window: usize,
        point: [i32; 2],
        capture: bool,
        budget: &mut u64,
    ) -> Result<Option<usize>> {
        let Some(w) = self.windows.get(&window) else {
            return Ok(None);
        };
        if capture
            && let Some(id) = w.input.capture
            && self.layers.contains_key(&id)
        {
            return Ok(Some(id));
        }
        let Some(root) = object(&w.primary_layer)? else {
            return Ok(None);
        };
        Ok(self.host_hit(vm, root, point, budget, 0)?.flatten())
    }
    fn host_layer_pointer(
        &mut self,
        vm: &mut Vm,
        id: usize,
        name: &str,
        point: [i32; 2],
        tail: &[i64],
        budget: &mut u64,
    ) -> Result<()> {
        let offset = self.layer_pointer_offset(id);
        let mut args = numbers(&[
            point[0].saturating_sub(offset[0]).into(),
            point[1].saturating_sub(offset[1]).into(),
        ]);
        args.extend(numbers(tail));
        self.layer_event(vm, id, name, &args, budget)
    }
    fn host_hover(
        &mut self,
        vm: &mut Vm,
        window: usize,
        point: [i32; 2],
        shift: u32,
        send_move: bool,
        budget: &mut u64,
    ) -> Result<()> {
        let old = self.windows.get(&window).and_then(|w| w.input.hover);
        let mut target = self.host_target(vm, window, point, true, budget)?;
        if old != target {
            if let Some(id) = old {
                self.layer_event(vm, id, "onMouseLeave", &[], budget)?;
            }
            target = self.host_target(vm, window, point, true, budget)?;
            if let Some(id) = target {
                self.layer_event(vm, id, "onMouseEnter", &[], budget)?;
            }
            let rechecked = self.host_target(vm, window, point, true, budget)?;
            if rechecked != target {
                if let Some(id) = target {
                    self.layer_event(vm, id, "onMouseLeave", &[], budget)?;
                }
                target = rechecked;
                if let Some(id) = target {
                    self.layer_event(vm, id, "onMouseEnter", &[], budget)?;
                }
            }
            if let Some(w) = self.windows.get_mut(&window) {
                w.input.hover = target;
            }
        }
        if send_move && let Some(id) = target {
            self.host_layer_pointer(vm, id, "onMouseMove", point, &[shift.into()], budget)?;
        }
        Ok(())
    }
}
impl crate::Session {
    fn host_window_event(&mut self, window: usize, name: &str, args: &[Value]) -> Result<()> {
        if !self.services.windows.contains_key(&window) {
            return Ok(());
        }
        let context = bound(window);
        let callback = self.vm.get_member(&context, &Value::string(name), true)?;
        if !matches!(callback, Value::Void) {
            self.vm.call_function(
                &callback,
                &context,
                args,
                &mut self.services,
                &mut self.budget,
            )?;
        }
        Ok(())
    }
    /// Physical client coordinates are converted using the window's draw rectangle.
    pub fn input(&mut self, window: &Value, event: InputEvent) -> Result<()> {
        let window = object(window)?.context("input requires a Window")?;
        let state = self
            .services
            .windows
            .get(&window)
            .context("input Window is invalid")?;
        if !state.visible || state.minimized {
            return Ok(());
        }
        let primary = object(&state.primary_layer)?;
        let convert = |x: i32, y: i32| {
            if let Some(layer) = primary.and_then(|id| self.services.layers.get(&id)) {
                let [ox, oy, w, h] = state.draw_rect(layer.width, layer.height);
                [
                    ((i64::from(x) - i64::from(ox)) * i64::from(layer.width) / i64::from(w.max(1)))
                        as i32,
                    ((i64::from(y) - i64::from(oy)) * i64::from(layer.height) / i64::from(h.max(1)))
                        as i32,
                ]
            } else {
                [x, y]
            }
        };
        let (name, args, point) = match &event {
            InputEvent::PointerMove { x, y, shift } => (
                "onMouseMove",
                numbers(&[(*x).into(), (*y).into(), (*shift).into()]),
                convert(*x, *y),
            ),
            InputEvent::PointerDown {
                x,
                y,
                button,
                shift,
            } => (
                "onMouseDown",
                numbers(&[(*x).into(), (*y).into(), (*button).into(), (*shift).into()]),
                convert(*x, *y),
            ),
            InputEvent::PointerUp {
                x,
                y,
                button,
                shift,
            } => (
                "onMouseUp",
                numbers(&[(*x).into(), (*y).into(), (*button).into(), (*shift).into()]),
                convert(*x, *y),
            ),
            InputEvent::Click { x, y } => (
                "onClick",
                numbers(&[(*x).into(), (*y).into()]),
                convert(*x, *y),
            ),
            InputEvent::DoubleClick { x, y } => (
                "onDoubleClick",
                numbers(&[(*x).into(), (*y).into()]),
                convert(*x, *y),
            ),
            InputEvent::Wheel { x, y, delta, shift } => (
                "onMouseWheel",
                numbers(&[(*shift).into(), (*delta).into(), (*x).into(), (*y).into()]),
                convert(*x, *y),
            ),
            InputEvent::KeyDown { key, shift } => (
                "onKeyDown",
                numbers(&[(*key).into(), (*shift).into()]),
                [0; 2],
            ),
            InputEvent::KeyUp { key, shift } => (
                "onKeyUp",
                numbers(&[(*key).into(), (*shift).into()]),
                [0; 2],
            ),
            InputEvent::Text(text) => ("onKeyPress", vec![Value::string(text)], [0; 2]),
            InputEvent::PointerLeave => ("onMouseLeave", vec![], [-1, -1]),
            InputEvent::Focus(focused) => (
                if *focused {
                    "onActivate"
                } else {
                    "onDeactivate"
                },
                vec![],
                [0; 2],
            ),
        };
        if let Some(state) = self.services.windows.get_mut(&window) {
            match &event {
                InputEvent::KeyDown { key, .. } => { state.input.keys.insert(*key); state.input.pressed.insert(*key); }
                InputEvent::KeyUp { key, .. } => { state.input.keys.remove(key); }
                InputEvent::PointerDown { button, .. } => { let key = [1,2,4].get(*button as usize).copied().unwrap_or(0); state.input.keys.insert(key); state.input.pressed.insert(key); }
                InputEvent::PointerUp { button, .. } => { let key = [1,2,4].get(*button as usize).copied().unwrap_or(0); state.input.keys.remove(&key); }
                InputEvent::Focus(false) => state.input.keys.clear(),
                _ => {}
            }
        }
        self.host_window_event(window, name, &args)?;
        if !self.services.windows.contains_key(&window) {
            return Ok(());
        }
        let s = &mut self.services;
        match event {
            InputEvent::PointerMove { shift, .. } | InputEvent::PointerDown { shift, .. } => {
                let moved = s.windows[&window].pointer != point;
                s.windows.get_mut(&window).unwrap().pointer = point;
                s.host_hover(&mut self.vm, window, point, shift, moved, &mut self.budget)?;
                if let InputEvent::PointerDown { button, .. } = event {
                    let target =
                        s.host_target(&mut self.vm, window, point, true, &mut self.budget)?;
                    s.windows.get_mut(&window).unwrap().input.release_capture = false;
                    if let Some(id) = target {
                        s.host_layer_pointer(
                            &mut self.vm,
                            id,
                            name,
                            point,
                            &[button.into(), shift.into()],
                            &mut self.budget,
                        )?;
                    }
                    if let Some(w) = s.windows.get_mut(&window)
                        && !w.input.release_capture
                    {
                        w.input.capture = target.filter(|id| s.layers.contains_key(id));
                    }
                }
            }
            InputEvent::PointerUp { button, shift, .. } => {
                if let Some(id) =
                    s.host_target(&mut self.vm, window, point, true, &mut self.budget)?
                {
                    s.host_layer_pointer(
                        &mut self.vm,
                        id,
                        name,
                        point,
                        &[button.into(), shift.into()],
                        &mut self.budget,
                    )?;
                }
                if shift & 56 == 0
                    && let Some(w) = s.windows.get_mut(&window)
                {
                    w.input.capture = None;
                    s.host_hover(&mut self.vm, window, point, shift, false, &mut self.budget)?;
                }
            }
            InputEvent::Click { .. } | InputEvent::DoubleClick { .. } => {
                if let Some(id) =
                    s.host_target(&mut self.vm, window, point, false, &mut self.budget)?
                    && (matches!(event, InputEvent::DoubleClick { .. })
                        || s.windows[&window].input.capture == Some(id))
                {
                    s.host_layer_pointer(&mut self.vm, id, name, point, &[], &mut self.budget)?;
                }
            }
            InputEvent::PointerLeave => {
                s.host_hover(&mut self.vm, window, [-1, -1], 0, false, &mut self.budget)?;
            }
            InputEvent::Focus(false) => {
                s.windows.get_mut(&window).unwrap().input.capture = None;
            }
            InputEvent::KeyDown { .. }
            | InputEvent::KeyUp { .. }
            | InputEvent::Text(_)
            | InputEvent::Wheel { .. } => {
                if let Some(root) = primary
                    && let Some(layer) = s.layers.get(&root)
                {
                    let focused = layer.focused_layer;
                    let mut args = args;
                    if !matches!(event, InputEvent::Wheel { .. }) {
                        args.push(Value::Integer(1));
                    }
                    if let Some(id) = focused {
                        s.layer_event(&mut self.vm, id, name, &args, &mut self.budget)?;
                    } else if !matches!(event, InputEvent::Wheel { .. }) {
                        s.layer_input_default(&mut self.vm, root, name, &args, &mut self.budget)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn request_window_close(&mut self, window: &Value) -> Result<()> {
        let id = object(window)?.context("close requires a Window")?;
        self.host_window_event(id, "onCloseQuery", &[Value::Integer(1)])
    }
    pub fn resize_window(&mut self, window: &Value, width: u32, height: u32) -> Result<()> {
        ensure!(
            width <= 32768 && height <= 32768,
            "window size exceeds limit"
        );
        let id = object(window)?.context("resize requires a Window")?;
        let w = self
            .services
            .windows
            .get_mut(&id)
            .context("resize Window is invalid")?;
        if (w.inner_width, w.inner_height) != (width as i32, height as i32) {
            w.inner_width = width as i32;
            w.inner_height = height as i32;
            w.resize_pending = true;
        }
        Ok(())
    }
}
