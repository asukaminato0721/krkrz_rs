//! layerExDraw geometry and registration. API reference: Wamsoft layerExDraw;
//! behavior reference: the installed DLL, exercised with synthetic fixtures.
use crate::{Services, plugins};
use anyhow::{Context, Result, bail, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Exports {
    methods: Vec<String>,
    properties: Vec<String>,
}
#[derive(Deserialize)]
struct Interface {
    constants: BTreeMap<String, i64>,
    classes: BTreeMap<String, Exports>,
    layer: Exports,
}
#[derive(Default)]
pub(crate) struct State {
    classes: BTreeMap<String, Value>,
    objects: BTreeMap<(usize, String), Geometry>,
}
#[derive(Clone)]
enum Geometry {
    Point([f32; 2]),
    Rect([f32; 4]),
    Matrix([f32; 6], i32),
}
const IDENTITY: [f32; 6] = [1., 0., 0., 1., 0., 0.];
fn real(n: f32) -> Value {
    Value::Real(n as f64)
}
fn number(v: &Value) -> Result<f32> {
    let n = v.real()? as f32;
    ensure!(
        n.is_finite(),
        "GdiPlus geometry requires finite coordinates"
    );
    Ok(n)
}
fn id(v: &Value) -> Result<usize> {
    match v {
        Value::Object(r) => r.object.context("GdiPlus requires a non-null context"),
        _ => bail!("GdiPlus requires an object context"),
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<State> {
    let exports: Interface = serde_json::from_str(include_str!("../data/layer_draw.json"))?;
    let root = vm.register_native_class("GdiPlus")?;
    for method in ["GdiPlus", "finalize", "addPrivateFont", "getFontList"] {
        vm.register_native_method(&root, method, &format!("GdiPlus.{method}"))?;
    }
    for (key, value) in exports.constants {
        vm.set_member(&root, &Value::string(&key), Value::Integer(value))?;
    }
    let mut state = State::default();
    for (name, exports) in exports.classes {
        let prefix = format!("GdiPlus.{name}");
        let class = vm.new_native_class(&name, &format!("{prefix}.@initialize"))?;
        for method in exports.methods {
            if name == "RectF" && method == "Union" {
                vm.register_native_static_method(&class, &method, &format!("{prefix}.{method}"))?;
            } else {
                vm.register_native_method(&class, &method, &format!("{prefix}.{method}"))?;
            }
        }
        for property in exports.properties {
            let writable = match name.as_str() {
                "PointF" => true,
                "RectF" => matches!(property.as_str(), "x" | "y" | "width" | "height"),
                "Font" => matches!(
                    property.as_str(),
                    "familyName" | "emSize" | "style" | "forceSelfPathDraw"
                ),
                _ => false,
            };
            vm.register_native_property(
                &class,
                &property,
                Some(&format!("{prefix}.get:{property}")),
                writable
                    .then(|| format!("{prefix}.set:{property}"))
                    .as_deref(),
            )?;
        }
        vm.set_member(&root, &Value::string(&name), class.clone())?;
        state.classes.insert(name, class);
    }
    if !vm.globals.contains_key("Layer") {
        plugins::declare_class(vm, "Layer")?;
    }
    let layer = vm.globals["Layer"].clone();
    for method in exports.layer.methods {
        vm.register_native_method(&layer, &method, &format!("Layer.{method}"))?;
    }
    for property in exports.layer.properties {
        vm.register_native_property(
            &layer,
            &property,
            Some(&format!("Layer.get:{property}")),
            Some(&format!("Layer.set:{property}")),
        )?;
    }
    Ok(state)
}
impl Services {
    fn geometry(&self, id: usize, kind: &str) -> Result<&Geometry> {
        self.layer_draw
            .objects
            .get(&(id, kind.into()))
            .context("context has no matching GdiPlus native instance")
    }
    fn converted<const N: usize>(
        &mut self,
        vm: &mut Vm,
        v: &Value,
        kind: &str,
        fields: [&str; N],
        budget: &mut u64,
    ) -> Result<[f32; N]> {
        if matches!(v, Value::Object(r) if r.object.is_none()) {
            bail!("GdiPlus cannot convert a null object to geometry");
        }
        if let Ok(id) = id(v) {
            if let Ok(object) = self.geometry(id, kind) {
                let data: &[f32] = match object {
                    Geometry::Point(p) => p,
                    Geometry::Rect(r) => r,
                    Geometry::Matrix(m, _) => m,
                };
                return Ok(data.try_into().expect("matching geometry dimensions"));
            }
            let array = vm.instance_of(v, "Array")?.integer()? != 0;
            let mut result = [0.; N];
            for (i, field) in fields.iter().enumerate().rev() {
                let key = if array {
                    Value::Integer(i as i64)
                } else {
                    Value::string(field)
                };
                // ncbind checks presence through PropGet, then reads the value again.
                vm.get_property(v, &key, true, false, self, budget)?;
                let value = vm.get_property(v, &key, true, false, self, budget)?;
                result[i] = number(&value)?;
            }
            Ok(result)
        } else {
            Ok([0.; N])
        }
    }
    fn new_geometry(
        &mut self,
        vm: &mut Vm,
        kind: &str,
        values: &[f32],
        budget: &mut u64,
    ) -> Result<Value> {
        let class = self.layer_draw.classes[kind].clone();
        vm.construct(
            &class,
            &values.iter().copied().map(real).collect::<Vec<_>>(),
            self,
            budget,
        )
    }
    pub(crate) fn layer_draw_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Some((kind, op)) = operation.split_once('.') else {
            return Err(unsupported(format!(
                "GdiPlus.{operation} is not implemented"
            )));
        };
        if kind == "RectF" && op == "Union" {
            ensure!(
                args.len() >= 3,
                "GdiPlus.RectF.Union requires three arguments"
            );
            // The native converter passes a temporary RectF for the output:
            // its changed coordinates are not copied back into the TJS object.
            self.converted(vm, &args[0], "RectF", ["x", "y", "width", "height"], budget)?;
            let a = self.converted(vm, &args[1], "RectF", ["x", "y", "width", "height"], budget)?;
            let b = self.converted(vm, &args[2], "RectF", ["x", "y", "width", "height"], budget)?;
            return Ok(Value::Integer(
                ((a[0] + a[2]).max(b[0] + b[2]) > a[0].min(b[0])
                    && (a[1] + a[3]).max(b[1] + b[3]) > a[1].min(b[1]))
                .into(),
            ));
        }
        let id = id(context)?;
        if op == "@invalidate" {
            self.layer_draw.objects.remove(&(id, kind.into()));
            return Ok(Value::Void);
        }
        if op == "@initialize" {
            let value = match kind {
                "PointF" => Geometry::Point([0.; 2]),
                "RectF" => Geometry::Rect([0.; 4]),
                "Matrix" => Geometry::Matrix(IDENTITY, 0),
                _ => {
                    return Err(unsupported(format!(
                        "GdiPlus.{kind} construction is not implemented"
                    )));
                }
            };
            self.layer_draw.objects.insert((id, kind.into()), value);
            return Ok(Value::Void);
        }
        let mut object = self.geometry(id, kind)?.clone();
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("GdiPlus.{kind}.{op} argument {} is required", i + 1))
        };
        let mut result = Value::Void;
        match &mut object {
            Geometry::Point(p) => match op {
                "PointF" => {
                    *p = [number(arg(0)?)?, number(arg(1)?)?];
                }
                "get:x" => result = real(p[0]),
                "get:y" => result = real(p[1]),
                "set:x" => p[0] = number(arg(0)?)?,
                "set:y" => p[1] = number(arg(0)?)?,
                "Equals" => {
                    let other = self.converted(vm, arg(0)?, "PointF", ["x", "y"], budget)?;
                    let Geometry::Point(current) = self.geometry(id, kind)? else {
                        unreachable!()
                    };
                    result = Value::Integer((*current == other).into());
                }
                "finalize" => {}
                _ => {
                    return Err(unsupported(format!(
                        "GdiPlus.PointF.{op} is not implemented"
                    )));
                }
            },
            Geometry::Rect(r) => match op {
                "RectF" => {
                    for (i, v) in r.iter_mut().enumerate() {
                        *v = number(arg(i)?)?;
                    }
                }
                "get:x" | "get:left" => result = real(r[0]),
                "get:y" | "get:top" => result = real(r[1]),
                "get:width" => result = real(r[2]),
                "get:height" => result = real(r[3]),
                "get:right" => result = real(r[0] + r[2]),
                "get:bottom" => result = real(r[1] + r[3]),
                "set:x" => r[0] = number(arg(0)?)?,
                "set:y" => r[1] = number(arg(0)?)?,
                "set:width" => r[2] = number(arg(0)?)?,
                "set:height" => r[3] = number(arg(0)?)?,
                "get:bounds" | "Clone" => return self.new_geometry(vm, "RectF", r, budget),
                "get:location" => return self.new_geometry(vm, "PointF", &r[..2], budget),
                "Equals" => {
                    let other = self.converted(
                        vm,
                        arg(0)?,
                        "RectF",
                        ["x", "y", "width", "height"],
                        budget,
                    )?;
                    let Geometry::Rect(current) = self.geometry(id, kind)? else {
                        unreachable!()
                    };
                    result = Value::Integer((*current == other).into());
                }
                "IsEmptyArea" => result = Value::Integer((r[2] <= 0. || r[3] <= 0.).into()),
                "IntersectsWith" => {
                    let b = self.converted(
                        vm,
                        arg(0)?,
                        "RectF",
                        ["x", "y", "width", "height"],
                        budget,
                    )?;
                    let Geometry::Rect(r) = self.geometry(id, kind)? else {
                        unreachable!()
                    };
                    result = Value::Integer(
                        (r[0] < b[0] + b[2]
                            && b[0] < r[0] + r[2]
                            && r[1] < b[1] + b[3]
                            && b[1] < r[1] + r[3])
                            .into(),
                    );
                }
                "Offset" => {
                    r[0] += number(arg(0)?)?;
                    r[1] += number(arg(1)?)?;
                }
                "Inflate" | "InflatePoint" => {
                    let p = if op == "Inflate" {
                        [number(arg(0)?)?, number(arg(1)?)?]
                    } else {
                        self.converted(vm, arg(0)?, "PointF", ["x", "y"], budget)?
                    };
                    let Geometry::Rect(current) = self.geometry(id, kind)? else {
                        unreachable!()
                    };
                    *r = *current;
                    r[0] -= p[0];
                    r[1] -= p[1];
                    r[2] += 2. * p[0];
                    r[3] += 2. * p[1];
                }
                "finalize" => {}
                _ => {
                    return Err(unsupported(format!(
                        "GdiPlus.RectF.{op} is not implemented"
                    )));
                }
            },
            Geometry::Matrix(m, status) => {
                let mut rc = 0;
                match op {
                    "Matrix" => {
                        ensure!(
                            args.is_empty() || args.len() == 6,
                            "GdiPlus.Matrix requires zero or six arguments"
                        );
                        if args.is_empty() {
                            *m = IDENTITY;
                        } else {
                            for (i, v) in m.iter_mut().enumerate() {
                                *v = number(arg(i)?)?;
                            }
                        }
                    }
                    "OffsetX" => return Ok(real(m[4])),
                    "OffsetY" => return Ok(real(m[5])),
                    "IsIdentity" => return Ok(Value::Integer((*m == IDENTITY).into())),
                    "IsInvertible" => return Ok(Value::Integer((determinant(m) != 0.).into())),
                    "GetLastStatus" => {
                        result = Value::Integer(*status as i64);
                        *status = 0;
                    }
                    "Equals" => {
                        let other = self.converted(
                            vm,
                            arg(0)?,
                            "Matrix",
                            ["m11", "m12", "m21", "m22", "dx", "dy"],
                            budget,
                        )?;
                        let Geometry::Matrix(current, _) = self.geometry(id, kind)? else {
                            unreachable!()
                        };
                        result = Value::Integer((*current == other).into());
                    }
                    "Reset" => {
                        *m = IDENTITY;
                        result = Value::Integer(0);
                    }
                    "SetElements" => {
                        for (i, v) in m.iter_mut().enumerate() {
                            *v = number(arg(i)?)?;
                        }
                        result = Value::Integer(0);
                    }
                    "Invert" => {
                        let det = determinant(m);
                        if det == 0. {
                            rc = 2;
                        } else {
                            let [a, b, c, d, x, y] = *m;
                            *m = [
                                d / det,
                                -b / det,
                                -c / det,
                                a / det,
                                (c * y - d * x) / det,
                                (b * x - a * y) / det,
                            ];
                        }
                        result = Value::Integer(rc);
                    }
                    "Multiply" | "Translate" | "Scale" | "Shear" | "Rotate" | "RotateAt" => {
                        let (n, order) = match op {
                            "Multiply" => (
                                self.converted(
                                    vm,
                                    arg(0)?,
                                    "Matrix",
                                    ["m11", "m12", "m21", "m22", "dx", "dy"],
                                    budget,
                                )?,
                                arg(1)?.integer()?,
                            ),
                            "Translate" => (
                                [1., 0., 0., 1., number(arg(0)?)?, number(arg(1)?)?],
                                arg(2)?.integer()?,
                            ),
                            "Scale" => (
                                [number(arg(0)?)?, 0., 0., number(arg(1)?)?, 0., 0.],
                                arg(2)?.integer()?,
                            ),
                            "Shear" => (
                                [1., number(arg(1)?)?, number(arg(0)?)?, 1., 0., 0.],
                                arg(2)?.integer()?,
                            ),
                            _ => {
                                let radians = number(arg(0)?)?.to_radians();
                                let (s, c) = radians.sin_cos();
                                let mut n = [c, s, -s, c, 0., 0.];
                                let order = if op == "RotateAt" {
                                    let [x, y] =
                                        self.converted(vm, arg(1)?, "PointF", ["x", "y"], budget)?;
                                    n[4] = x - c * x + s * y;
                                    n[5] = y - s * x - c * y;
                                    arg(2)?.integer()?
                                } else {
                                    arg(1)?.integer()?
                                };
                                (n, order)
                            }
                        };
                        let Geometry::Matrix(current, current_status) = self.geometry(id, kind)?
                        else {
                            unreachable!()
                        };
                        *m = *current;
                        *status = *current_status;
                        if order == 0 {
                            *m = multiply(&n, m);
                        } else if order == 1 {
                            *m = multiply(m, &n);
                        } else {
                            rc = 2;
                        }
                        result = Value::Integer(rc);
                    }
                    "finalize" => {}
                    _ => {
                        return Err(unsupported(format!(
                            "GdiPlus.Matrix.{op} is not implemented"
                        )));
                    }
                }
                if rc != 0 {
                    *status = rc as i32;
                }
            }
        }
        // A conversion can run script accessors which invalidate the receiver.
        ensure!(
            self.layer_draw.objects.contains_key(&(id, kind.into())),
            "GdiPlus receiver was invalidated during conversion"
        );
        if !matches!(op, "Equals" | "IntersectsWith" | "finalize") {
            self.layer_draw.objects.insert((id, kind.into()), object);
        }
        Ok(result)
    }
}
fn determinant(m: &[f32; 6]) -> f32 {
    m[0] * m[3] - m[1] * m[2]
}
fn multiply(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}
