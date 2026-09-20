//! Window state shared by headless execution and the future platform presenter.
//! Bindings follow krkrz visual/WindowIntf.cpp; no OS window is created here.
use anyhow::{Context, Result, bail, ensure};
use krkrz_tjs::{Value, Vm, unsupported};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShowCommand {
    Minimize,
    Maximize,
    Restore,
}

#[derive(Clone, Debug)]
pub struct WindowState {
    pub constructed: bool,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    normal_bounds: Option<[i32; 4]>,
    restore_maximized: bool,
    pub caption: String,
    pub border_style: i32,
    pub inner_width: i32,
    pub inner_height: i32,
    /// Host-provided left, top, right and bottom non-client extents. Headless
    /// windows have no decorations; full-screen windows ignore these extents.
    pub frame_insets: [i32; 4],
    pub zoom_numer: i32,
    pub zoom_denom: i32,
    full_screen: Option<WindowedBounds>,
    pub left: i32,
    pub top: i32,
    pub primary_layer: Value,
    pub(crate) pointer: [i32; 2],
    pub(crate) input: crate::layer::input::State,
    pub(crate) draw_device: Value,
    pub(crate) registered_objects: Vec<Value>,
    pub(crate) invalidating: bool,
    pub(crate) resize_pending: bool,
}
#[derive(Clone, Debug)]
struct WindowedBounds {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    scale: [i32; 2],
    origin: [i32; 2],
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            constructed: false,
            visible: false,
            minimized: false,
            maximized: false,
            normal_bounds: None,
            restore_maximized: false,
            caption: String::new(),
            border_style: 2,
            inner_width: 10,
            inner_height: 10,
            frame_insets: [0; 4],
            zoom_numer: 1,
            zoom_denom: 1,
            full_screen: None,
            left: 0,
            top: 0,
            primary_layer: Value::NULL,
            pointer: [0; 2],
            input: Default::default(),
            draw_device: Value::Void,
            registered_objects: Vec::new(),
            invalidating: false,
            resize_pending: false,
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("Window")?;
    vm.register_native_property(&class, "mainWindow", Some("Window.get:mainWindow"), None)?;
    for name in [
        "Window",
        "finalize",
        "add",
        "remove",
        "setInnerSize",
        "setSize",
        "setZoom",
        "setPos",
        "onResize",
        "onClick",
        "onDoubleClick",
        "onMouseDown",
        "onMouseUp",
        "onMouseMove",
        "onMouseEnter",
        "onMouseLeave",
        "onMouseWheel",
        "onKeyDown",
        "onKeyUp",
        "onKeyPress",
        "onActivate",
        "onDeactivate",
        "onCloseQuery",
        "close",
    ] {
        vm.register_native(&format!("Window.{name}"))?;
    }
    for name in [
        "focusedLayer",
        "fullScreen",
        "visible",
        "caption",
        "borderStyle",
        "innerWidth",
        "innerHeight",
        "width",
        "height",
        "zoomNumer",
        "zoomDenom",
        "left",
        "top",
    ] {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("Window.get:{name}")),
            Some(&format!("Window.set:{name}")),
        )?;
    }
    vm.register_native_property(
        &class,
        "primaryLayer",
        Some("Window.get:primaryLayer"),
        None,
    )?;
    Ok(())
}
fn coordinate(value: &Value) -> Result<i32> {
    Ok(value.integer()? as i32)
}
fn dimension(value: &Value) -> Result<i32> {
    let value = coordinate(value)?;
    ensure!(value >= 0, "negative window dimension");
    if value > 32768 {
        return Err(unsupported("window dimension exceeds surface limit"));
    }
    Ok(value)
}
impl WindowState {
    pub fn client_rect(&self) -> [i32; 4] {
        let insets = self.effective_insets();
        [
            self.left.saturating_add(insets[0]),
            self.top.saturating_add(insets[1]),
            self.inner_width,
            self.inner_height,
        ]
    }
    pub fn window_rect(&self) -> [i32; 4] {
        let insets = self.effective_insets();
        [
            self.left,
            self.top,
            self.inner_width
                .saturating_add(insets[0])
                .saturating_add(insets[2]),
            self.inner_height
                .saturating_add(insets[1])
                .saturating_add(insets[3]),
        ]
    }
    pub fn normal_rect(&self) -> [i32; 4] {
        self.normal_bounds.unwrap_or_else(|| self.window_rect())
    }
    pub(crate) fn apply_show_command(&mut self, command: ShowCommand, work: [i32; 4]) {
        match command {
            ShowCommand::Maximize => {
                if !self.maximized {
                    if self.normal_bounds.is_none() {
                        self.normal_bounds = Some(self.window_rect());
                    }
                    self.left = work[0];
                    self.top = work[1];
                    self.set_outer_size(work[2], work[3]);
                }
                self.minimized = false;
                self.maximized = true;
                self.visible = true;
            }
            ShowCommand::Minimize => {
                if !self.minimized {
                    if self.normal_bounds.is_none() {
                        self.normal_bounds = Some(self.window_rect());
                    }
                    self.restore_maximized = self.maximized;
                    self.minimized = true;
                    self.maximized = false;
                    self.visible = true;
                }
            }
            ShowCommand::Restore => {
                if self.minimized && self.restore_maximized {
                    self.apply_show_command(ShowCommand::Maximize, work);
                } else {
                    if let Some([x, y, w, h]) = self.normal_bounds.take() {
                        self.left = x;
                        self.top = y;
                        self.set_outer_size(w, h);
                    }
                    self.minimized = false;
                    self.maximized = false;
                    self.visible = true;
                }
                self.restore_maximized = false;
            }
        }
    }
    fn effective_insets(&self) -> [i32; 4] {
        if self.is_full_screen() || self.border_style == 0 {
            [0; 4]
        } else {
            self.frame_insets
        }
    }
    pub fn is_full_screen(&self) -> bool {
        self.full_screen.is_some()
    }

    /// Destination rectangle for the primary layer in physical client pixels.
    /// A native presenter uses this same layout as a headless frame capture.
    pub fn draw_rect(&self, width: i32, height: i32) -> [i32; 4] {
        let (scale, origin) = self
            .full_screen
            .as_ref()
            .map_or(([self.zoom_numer, self.zoom_denom], [0, 0]), |saved| {
                (saved.scale, saved.origin)
            });
        [
            origin[0],
            origin[1],
            mul_div(width, scale[0], scale[1]).max(1),
            mul_div(height, scale[0], scale[1]).max(1),
        ]
    }

    pub(crate) fn leave_full_screen(&mut self) {
        if let Some(saved) = self.full_screen.take() {
            self.left = saved.left;
            self.top = saved.top;
            self.set_size(saved.width, saved.height);
            self.visible = true;
        }
    }
    pub(crate) fn enter_full_screen(&mut self, screen: [i32; 4]) -> Result<()> {
        if self.is_full_screen() {
            return Ok(());
        }
        ensure!(
            screen[2] > 0 && screen[3] > 0 && screen[2] <= 32768 && screen[3] <= 32768,
            "invalid full-screen display size"
        );
        let (width, height) = (self.inner_width.max(1), self.inner_height.max(1));
        let (sw, sh) = (screen[2], screen[3]);
        let mut scale = if i64::from(sw) * i64::from(height) < i64::from(sh) * i64::from(width) {
            [sw, width]
        } else {
            [sh, height]
        };
        let ratio = scale[0] as f64 / scale[1] as f64;
        // Kirikiri avoids stretching for a negligible increase in size.
        if ratio > 1.0 && ratio < 1.034 {
            scale = [1, 1];
        }
        let target_width = (i64::from(width) * i64::from(scale[0]) / i64::from(scale[1])) as i32;
        let target_height = (i64::from(height) * i64::from(scale[0]) / i64::from(scale[1])) as i32;
        self.full_screen = Some(WindowedBounds {
            left: self.left,
            top: self.top,
            width: self.inner_width,
            height: self.inner_height,
            scale,
            origin: [(sw - target_width) / 2, (sh - target_height) / 2],
        });
        self.left = screen[0];
        self.top = screen[1];
        self.set_size(sw, sh);
        self.visible = true;
        self.minimized = false;
        Ok(())
    }
    fn set_zoom(&mut self, numer: i32, denom: i32) -> Result<()> {
        // Preserve the sign chosen by the original signed Euclidean algorithm.
        // Widen intermediates to avoid i32::MIN / -1 overflow.
        let (mut a, mut b) = (i64::from(numer), i64::from(denom));
        while b != 0 {
            (a, b) = (b, a % b);
        }
        ensure!(a != 0, "window zoom ratio is undefined (0/0)");
        self.zoom_numer = (i64::from(numer) / a) as i32;
        self.zoom_denom = (i64::from(denom) / a) as i32;
        Ok(())
    }

    pub(crate) fn call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        ensure!(self.constructed, "Window constructor has not run");
        let arg = |index: usize| {
            args.get(index)
                .with_context(|| format!("Window.{name}: missing argument {index}"))
        };
        if let Some(key) = name.strip_prefix("get:") {
            return Ok(match key {
                "visible" => Value::Integer(i64::from(self.visible)),
                "fullScreen" => Value::Integer(self.is_full_screen().into()),
                "caption" => Value::string(&self.caption),
                "borderStyle" => Value::Integer(self.border_style.into()),
                "innerWidth" => Value::Integer(self.inner_width.into()),
                "innerHeight" => Value::Integer(self.inner_height.into()),
                "width" => Value::Integer(self.window_rect()[2].into()),
                "height" => Value::Integer(self.window_rect()[3].into()),
                "zoomNumer" => Value::Integer(self.zoom_numer.into()),
                "zoomDenom" => Value::Integer(self.zoom_denom.into()),
                "left" => Value::Integer(self.left.into()),
                "top" => Value::Integer(self.top.into()),
                "primaryLayer" => self.primary_layer.clone(),
                "drawDevice" => self.draw_device.clone(),
                _ => return Err(unsupported(format!("unsupported Window property: {key}"))),
            });
        }
        match name {
            "finalize" => {}
            "add" | "remove" => {
                let object = arg(0)?;
                ensure!(
                    matches!(object, Value::Object(_)),
                    "Window.{name} requires an Object"
                );
                if !self.invalidating {
                    if name == "remove" {
                        self.registered_objects.retain(|value| value != object);
                    } else if !self.registered_objects.contains(object) {
                        ensure!(
                            self.registered_objects.len() < 100_000,
                            "Window object limit exceeded"
                        );
                        self.registered_objects.push(object.clone());
                    }
                }
            }
            "set:visible" => self.visible = arg(0)?.truth()?,
            "set:caption" => self.caption = arg(0)?.text(),
            "set:borderStyle" => self.border_style = coordinate(arg(0)?)?,
            "set:innerWidth" => self.set_size(dimension(arg(0)?)?, self.inner_height),
            "set:innerHeight" => self.set_size(self.inner_width, dimension(arg(0)?)?),
            "set:width" => self.set_outer_size(dimension(arg(0)?)?, self.window_rect()[3]),
            "set:height" => self.set_outer_size(self.window_rect()[2], dimension(arg(0)?)?),
            "setSize" => self.set_outer_size(dimension(arg(0)?)?, dimension(arg(1)?)?),
            "set:zoomNumer" => self.set_zoom(coordinate(arg(0)?)?, self.zoom_denom)?,
            "set:zoomDenom" => self.set_zoom(self.zoom_numer, coordinate(arg(0)?)?)?,
            "setZoom" => self.set_zoom(coordinate(arg(0)?)?, coordinate(arg(1)?)?)?,
            "set:left" => self.left = coordinate(arg(0)?)?,
            "set:top" => self.top = coordinate(arg(0)?)?,
            "setInnerSize" => self.set_size(dimension(arg(0)?)?, dimension(arg(1)?)?),
            "setPos" => {
                self.left = coordinate(arg(0)?)?;
                self.top = coordinate(arg(1)?)?;
            }
            _ => bail!("unknown registered Window operation: {name}"),
        }
        Ok(Value::Void)
    }
    fn set_size(&mut self, width: i32, height: i32) {
        if (width, height) != (self.inner_width, self.inner_height) {
            self.inner_width = width;
            self.inner_height = height;
            self.resize_pending = true;
        }
    }
    fn set_outer_size(&mut self, width: i32, height: i32) {
        let [l, t, r, b] = self.effective_insets();
        self.set_size(
            width.saturating_sub(l).saturating_sub(r).max(0),
            height.saturating_sub(t).saturating_sub(b).max(0),
        );
    }
}

fn mul_div(value: i32, numer: i32, denom: i32) -> i32 {
    // Win32 MulDiv rounds to the nearest integer and returns -1 on failure.
    if denom == 0 {
        return -1;
    }
    let product = i64::from(value) * i64::from(numer);
    let denominator = i64::from(denom).abs();
    let rounded = (product.abs() + denominator / 2) / denominator;
    let rounded = if (product < 0) != (denom < 0) {
        -rounded
    } else {
        rounded
    };
    i32::try_from(rounded).unwrap_or(-1)
}

impl crate::Services {
    pub(crate) fn window_full_screen(&mut self, id: usize, enabled: bool) -> Result<()> {
        ensure!(
            self.windows.get(&id).is_some_and(|w| w.constructed),
            "context has no constructed Window native instance"
        );
        if enabled {
            let screen = self
                .display_for_rect(self.windows[&id].window_rect(), true)
                .context("no full-screen display is available")?
                .bounds;
            ensure!(
                screen[2] > 0 && screen[3] > 0 && screen[2] <= 32768 && screen[3] <= 32768,
                "invalid full-screen display size"
            );
            for (&other, window) in &mut self.windows {
                if other != id {
                    window.leave_full_screen();
                }
            }
            self.windows
                .get_mut(&id)
                .unwrap()
                .enter_full_screen(screen)?;
        } else {
            self.windows.get_mut(&id).unwrap().leave_full_screen();
        }
        Ok(())
    }
}
