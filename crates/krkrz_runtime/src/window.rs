//! Window state shared by headless execution and the future platform presenter.
//! Bindings follow krkrz visual/WindowIntf.cpp; no OS window is created here.
use anyhow::{Context, Result, bail, ensure};
use krkrz_tjs::{Value, Vm, unsupported};

#[derive(Clone, Debug)]
pub struct WindowState {
    pub constructed: bool,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub caption: String,
    pub border_style: i32,
    pub inner_width: i32,
    pub inner_height: i32,
    pub left: i32,
    pub top: i32,
    pub primary_layer: Value,
    pub(crate) registered_objects: Vec<Value>,
    pub(crate) invalidating: bool,
    pub(crate) resize_pending: bool,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            constructed: false,
            visible: false,
            minimized: false,
            maximized: false,
            caption: String::new(),
            border_style: 2,
            inner_width: 10,
            inner_height: 10,
            left: 0,
            top: 0,
            primary_layer: Value::NULL,
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
        "setPos",
        "onResize",
    ] {
        vm.register_native(&format!("Window.{name}"))?;
    }
    for name in [
        "visible",
        "caption",
        "borderStyle",
        "innerWidth",
        "innerHeight",
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
    pub(crate) fn call(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        ensure!(self.constructed, "Window constructor has not run");
        let arg = |index: usize| {
            args.get(index)
                .with_context(|| format!("Window.{name}: missing argument {index}"))
        };
        if let Some(key) = name.strip_prefix("get:") {
            return Ok(match key {
                "visible" => Value::Integer(i64::from(self.visible)),
                "caption" => Value::string(&self.caption),
                "borderStyle" => Value::Integer(self.border_style.into()),
                "innerWidth" => Value::Integer(self.inner_width.into()),
                "innerHeight" => Value::Integer(self.inner_height.into()),
                "left" => Value::Integer(self.left.into()),
                "top" => Value::Integer(self.top.into()),
                "primaryLayer" => self.primary_layer.clone(),
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
}
