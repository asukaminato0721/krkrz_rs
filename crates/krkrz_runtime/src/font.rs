//! Script Font state. Rasterization is supplied by the session's FontBook.
use crate::{Services, layer::object};
use anyhow::{Context, Result};
use krkrz_tjs::{Value, Vm, unsupported};
pub(crate) struct Font {
    face: String,
    height: i32,
    angle: i32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}
impl Default for Font {
    fn default() -> Self {
        Self {
            face: "ＭＳ ゴシック".into(),
            height: -12,
            angle: 0,
            bold: false,
            italic: false,
            underline: false,
            strikeout: false,
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("Font")?;
    for method in [
        "Font",
        "finalize",
        "getTextWidth",
        "getTextHeight",
        "getEscWidthX",
        "getEscWidthY",
        "getEscHeightX",
        "getEscHeightY",
        "getList",
        "getGlyphDrawRect",
        "mapPrerenderedFont",
        "unmapPrerenderedFont",
    ] {
        vm.register_native_method(&class, method, &format!("Font.{method}"))?;
    }
    for key in [
        "face",
        "height",
        "angle",
        "bold",
        "italic",
        "underline",
        "strikeout",
    ] {
        vm.register_native_property(
            &class,
            key,
            Some(&format!("Font.get:{key}")),
            Some(&format!("Font.set:{key}")),
        )?;
    }
    Ok(())
}
impl Services {
    pub(crate) fn font_call(&mut self, op: &str, context: &Value, args: &[Value]) -> Result<Value> {
        let id = object(context)?.context("Font requires non-null context")?;
        if op == "@initialize" {
            self.font_objects.entry(id).or_default();
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            self.font_objects.remove(&id);
            return Ok(Value::Void);
        }
        let f = self
            .font_objects
            .get_mut(&id)
            .context("context has no Font native instance")?;
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("Font.{op}: missing argument {i}"))
        };
        if let Some(key) = op.strip_prefix("get:") {
            return Ok(match key {
                "face" => Value::string(&f.face),
                "height" => Value::Integer(f.height.into()),
                "angle" => Value::Integer(f.angle.into()),
                "bold" => Value::Integer(f.bold.into()),
                "italic" => Value::Integer(f.italic.into()),
                "underline" => Value::Integer(f.underline.into()),
                "strikeout" => Value::Integer(f.strikeout.into()),
                _ => return Err(unsupported(format!("unsupported Font property: {key}"))),
            });
        }
        match op {
            "Font" => {
                if !args.is_empty() {
                    return Err(unsupported(
                        "explicit Font layer constructor is not implemented",
                    ));
                }
            }
            "finalize" => {}
            "set:face" => f.face = arg(0)?.text(),
            "set:height" => f.height = (arg(0)?.integer()? as i32).wrapping_abs(),
            "set:angle" => f.angle = arg(0)?.integer()? as i32,
            "set:bold" => f.bold = arg(0)?.truth()?,
            "set:italic" => f.italic = arg(0)?.truth()?,
            "set:underline" => f.underline = arg(0)?.truth()?,
            "set:strikeout" => f.strikeout = arg(0)?.truth()?,
            "getTextHeight" => {
                arg(0)?;
                return Ok(Value::Integer(f.height.wrapping_abs().into()));
            }
            _ => return Err(unsupported(format!("unsupported Font operation: {op}"))),
        }
        Ok(Value::Void)
    }
}
