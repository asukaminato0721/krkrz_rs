//! Host display geometry and windowEx monitor queries (miahmie, Kirikiri license).
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm};

/// Screen and usable work rectangles, expressed as x, y, width, height.
#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub primary: bool,
    pub bounds: [i32; 4],
    pub work: [i32; 4],
}
fn edges(rect: [i32; 4]) -> [i64; 4] {
    let [x, y, w, h] = rect.map(i64::from);
    [x, y, x + w, y + h]
}
fn intersection(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let a = edges(a);
    let b = edges(b);
    let (x, y, r, d) = (
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    );
    (x < r && y < d).then_some([x as i32, y as i32, (r - x) as i32, (d - y) as i32])
}
fn distance(a: [i32; 4], b: [i32; 4]) -> i128 {
    let a = edges(a);
    let b = edges(b);
    let x = (a[0] - b[2]).max(b[0] - a[2]).max(0) as i128;
    let y = (a[1] - b[3]).max(b[1] - a[3]).max(0) as i128;
    x * x + y * y
}
pub(crate) fn rectangle(vm: &mut Vm, rect: [i32; 4]) -> Result<Value> {
    let result = vm.new_dictionary()?;
    for (key, value) in ["x", "y", "w", "h"].into_iter().zip(rect) {
        vm.set_member(&result, &Value::string(key), Value::Integer(value.into()))?;
    }
    Ok(result)
}
fn dictionary(vm: &mut Vm, monitor: &Monitor, intersect: Option<[i32; 4]>) -> Result<Value> {
    let result = vm.new_dictionary()?;
    let bounds = rectangle(vm, monitor.bounds)?;
    let work = rectangle(vm, monitor.work)?;
    for (key, value) in [
        ("name", Value::string(&monitor.name)),
        ("primary", Value::Integer(monitor.primary.into())),
        ("monitor", bounds),
        ("work", work),
    ] {
        vm.set_member(&result, &Value::string(key), value)?;
    }
    if let Some(rect) = intersect {
        let rect = rectangle(vm, rect)?;
        vm.set_member(&result, &Value::string("intersect"), rect)?;
    }
    Ok(result)
}
fn arguments_rect(args: &[Value]) -> Result<[i32; 4]> {
    let mut result = [0; 4];
    for (output, input) in result.iter_mut().zip(args) {
        *output = input.integer()? as i32;
    }
    ensure!(
        result[0].checked_add(result[2]).is_some() && result[1].checked_add(result[3]).is_some(),
        "monitor query rectangle exceeds coordinate range"
    );
    Ok(result)
}
impl Services {
    pub fn set_displays(&mut self, displays: Vec<Monitor>) -> Result<()> {
        ensure!(
            !displays.is_empty() && displays.len() <= 64,
            "display count must be between 1 and 64"
        );
        ensure!(
            displays.iter().filter(|m| m.primary).count() == 1,
            "exactly one display must be primary"
        );
        for monitor in &displays {
            for rect in [monitor.bounds, monitor.work] {
                ensure!(
                    rect[2] > 0 && rect[3] > 0 && rect[2] <= 32768 && rect[3] <= 32768,
                    "invalid display rectangle size"
                );
                ensure!(
                    rect[0].checked_add(rect[2]).is_some()
                        && rect[1].checked_add(rect[3]).is_some(),
                    "display rectangle exceeds coordinate range"
                );
            }
            ensure!(
                intersection(monitor.bounds, monitor.work) == Some(monitor.work),
                "display work area must be inside its bounds"
            );
        }
        self.displays = displays;
        Ok(())
    }
    pub(crate) fn primary_display(&self) -> Monitor {
        self.displays
            .iter()
            .find(|m| m.primary)
            .cloned()
            .unwrap_or_else(|| {
                let bounds = [
                    0,
                    0,
                    self.screen_size.0.min(i32::MAX as u32) as i32,
                    self.screen_size.1.min(i32::MAX as u32) as i32,
                ];
                Monitor {
                    name: "Headless".into(),
                    primary: true,
                    bounds,
                    work: bounds,
                }
            })
    }
    fn display_list(&self) -> Vec<Monitor> {
        if self.displays.is_empty() {
            vec![self.primary_display()]
        } else {
            self.displays.clone()
        }
    }
    pub(crate) fn application_display(&self) -> Monitor {
        self.main_window
            .and_then(|id| self.windows.get(&id))
            .and_then(|window| self.display_for_rect(window.window_rect(), false))
            .unwrap_or_else(|| self.primary_display())
    }
    pub(crate) fn desktop_bounds(&self) -> [i32; 4] {
        self.application_display().work
    }
    pub(crate) fn display_for_rect(&self, mut rect: [i32; 4], nearest: bool) -> Option<Monitor> {
        // MonitorFromRect treats an empty rectangle as its upper-left point.
        if rect[2] <= 0 || rect[3] <= 0 {
            rect[2] = 1;
            rect[3] = 1;
        }
        let list = self.display_list();
        let mut best = None;
        let mut area = 0i64;
        for monitor in &list {
            if let Some(overlap) = intersection(rect, monitor.bounds) {
                let size = i64::from(overlap[2]) * i64::from(overlap[3]);
                if size > area {
                    area = size;
                    best = Some(monitor.clone());
                }
            }
        }
        if best.is_some() || !nearest {
            return best;
        }
        list.into_iter()
            .min_by_key(|monitor| distance(rect, monitor.bounds))
    }
    pub(crate) fn display_call(
        &self,
        vm: &mut Vm,
        operation: &str,
        args: &[Value],
    ) -> Result<Value> {
        if operation == "System.getDisplayMonitors" {
            ensure!(
                args.is_empty() || args.len() == 4,
                "getDisplayMonitors requires 0 or 4 arguments"
            );
            let rect = (!args.is_empty())
                .then(|| arguments_rect(args))
                .transpose()?;
            let mut values = Vec::new();
            for monitor in self.display_list() {
                let clip = rect.map_or(Some(monitor.bounds), |r| intersection(r, monitor.bounds));
                if clip.is_some() {
                    values.push(dictionary(vm, &monitor, clip)?);
                }
            }
            return vm.new_native_array(values);
        }
        let nearest = args.first().map(Value::integer).transpose()?.unwrap_or(0) != 0;
        let monitor = match args.len() {
            0 => Some(self.primary_display()),
            2 => {
                let id =
                    crate::layer::object(&args[1])?.context("getMonitorInfo requires a Window")?;
                let window = self
                    .windows
                    .get(&id)
                    .filter(|w| w.constructed)
                    .context("getMonitorInfo requires a constructed Window")?;
                self.display_for_rect(window.window_rect(), nearest)
            }
            3 => self.display_for_rect(
                [args[1].integer()? as i32, args[2].integer()? as i32, 1, 1],
                nearest,
            ),
            5 => self.display_for_rect(arguments_rect(&args[1..])?, nearest),
            _ => anyhow::bail!("getMonitorInfo requires 0, 2, 3 or 5 arguments"),
        };
        monitor
            .as_ref()
            .map(|m| dictionary(vm, m, None))
            .transpose()
            .map(|v| v.unwrap_or(Value::Void))
    }
}
