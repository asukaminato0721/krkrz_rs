//! TextRenderBase layout and TJS callbacks. The installed plugin is the behavior
//! reference; community TextRender implementations supplied API research leads.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Host, Value, Vm, unsupported};
use std::collections::BTreeMap;

const METHODS: &[&str] = &[
    "TextRenderBase",
    "finalize",
    "calcLineOffset",
    "calcShowCount",
    "clear",
    "done",
    "getCharacters",
    "getKeyWait",
    "newline",
    "onEval",
    "render",
    "resetFont",
    "resetStyle",
    "setDefault",
    "setFont",
    "setOption",
    "setRenderSize",
    "setStyle",
];
const OUTPUTS: &[&str] = &[
    "maxScrollLine",
    "maxScrollOffset",
    "renderBottom",
    "renderCount",
    "renderDelay",
    "renderLeft",
    "renderLines",
    "renderOver",
    "renderRight",
    "renderText",
    "renderTop",
];
// Property suffix, tag attribute, conversion, default. Attributes are case sensitive.
const FIELDS: &[(&str, &str, char, f64)] = &[
    ("Align", "align", 'i', -1.0),
    ("BigFontSize", "bigfontsize", 'f', 48.0),
    ("Bold", "bold", 'b', 0.0),
    ("ChColor", "color", 'c', 4294967295.0),
    ("Edge", "edge", 'b', 0.0),
    ("EdgeColor", "edgecolor", 'c', 4278223103.0),
    ("Face", "face", 's', 0.0),
    ("FontSize", "fontsize", 'f', 24.0),
    ("Italic", "", 'b', 0.0),
    ("LineSize", "linesize", 'f', 24.0),
    ("LineSpacing", "linespacing", 'f', 6.0),
    ("Pitch", "pitch", 'f', 0.0),
    ("RubyOffset", "rubyoffset", 'f', -2.0),
    ("RubySize", "rubysize", 'f', 10.0),
    ("Shadow", "shadow", 'b', 1.0),
    ("ShadowColor", "shadowcolor", 'c', 4278190080.0),
    ("ShadowDiff", "shadowdiff", 'i', 1.0),
    ("SmallFontSize", "smallfontsize", 'f', 12.0),
    ("Valign", "valign", 'i', -1.0),
];
type Format = BTreeMap<&'static str, Value>;
fn defaults() -> Format {
    FIELDS
        .iter()
        .map(|&(key, _, kind, n)| {
            (
                key,
                match kind {
                    's' => Value::string("normal"),
                    'f' => Value::Real(n),
                    _ => Value::Integer(n as i64),
                },
            )
        })
        .collect()
}
fn numeric(value: &Value) -> Result<f32> {
    let n = value.real()? as f32;
    ensure!(n.is_finite(), "TextRender numeric value must be finite");
    if n.abs() > 1e9 {
        return Err(unsupported("TextRender numeric range limit exceeded"));
    }
    Ok(n)
}
fn convert(value: &Value, kind: char) -> Result<Value> {
    Ok(match kind {
        's' => value.unary("string")?,
        'f' => real(numeric(value)?),
        'b' => Value::Integer((value.integer()? != 0).into()),
        'c' => Value::Integer(value.integer()? as u32 as i64),
        _ => Value::Integer(value.integer()? as i32 as i64),
    })
}
fn real(n: f32) -> Value {
    Value::Real(n as f64)
}
fn number(format: &Format, key: &str) -> f32 {
    format[key].real().expect("typed TextRender field") as f32
}
fn font_signature(f: &Format) -> [Value; 3] {
    [f["Bold"].clone(), f["Italic"].clone(), f["Face"].clone()]
}
fn style_field(key: &str) -> bool {
    matches!(
        key,
        "Align" | "Valign" | "Pitch" | "LineSize" | "LineSpacing"
    )
}
fn charge(budget: &mut u64) -> Result<()> {
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| unsupported("TextRender execution budget exceeded"))?;
    Ok(())
}
fn dictionary(
    vm: &mut Vm,
    entries: impl IntoIterator<Item = (&'static str, Value)>,
) -> Result<Value> {
    let out = vm.new_dictionary()?;
    for (key, value) in entries {
        vm.set_member(&out, &Value::string(key), value)?;
    }
    Ok(out)
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.register_native_class("TextRenderBase")?;
    for method in METHODS {
        vm.register_native(&format!("TextRenderBase.{method}"))?;
    }
    for &(suffix, _, _, _) in FIELDS {
        let name = format!("default{suffix}");
        vm.register_native_property(
            &class,
            &name,
            Some(&format!("TextRenderBase.get:{name}")),
            Some(&format!("TextRenderBase.set:{name}")),
        )?;
    }
    for name in ["fontScale", "timeScale", "vertical"] {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("TextRenderBase.get:{name}")),
            Some(&format!("TextRenderBase.set:{name}")),
        )?;
    }
    for name in OUTPUTS {
        vm.register_native_property(
            &class,
            name,
            Some(&format!("TextRenderBase.get:{name}")),
            None,
        )?;
    }
    Ok(())
}
#[derive(Clone)]
struct Character {
    text: Vec<u16>,
    format: Format,
    size: f32,
    width: f32,
    delay: f32,
    x: f32,
    y: f32,
    ruby: Option<Ruby>,
}
#[derive(Clone)]
struct Ruby {
    text: Vec<u16>,
    size: f32,
    width: f32,
    x: f32,
    y: f32,
}
impl Character {
    fn value(&self, vm: &mut Vm) -> Result<Value> {
        let f = &self.format;
        // Preserve the native dictionary's observed bucket-chain enumeration.
        let mut fields = vec![
            ("face", f["Face"].clone()),
            ("y", real(self.y)),
            ("edgeColor", f["EdgeColor"].clone()),
            ("color", f["ChColor"].clone()),
            ("vertical", Value::Integer(0)),
            ("shadow", f["Shadow"].clone()),
            ("cw", real(self.width)),
            ("bold", f["Bold"].clone()),
            ("text", Value::String(self.text.clone())),
            ("delay", real(self.delay)),
            ("shadowColor", f["ShadowColor"].clone()),
            ("x", real(self.x)),
            ("edge", f["Edge"].clone()),
            ("italic", f["Italic"].clone()),
            ("graph", Value::Integer(0)),
            ("shadowDiff", f["ShadowDiff"].clone()),
            ("size", real(self.size)),
        ];
        if let Some(ruby) = &self.ruby {
            let data = dictionary(
                vm,
                [
                    ("size", real(ruby.size)),
                    ("x", real(ruby.x)),
                    ("text", Value::String(ruby.text.clone())),
                    ("y", real(ruby.y)),
                ],
            )?;
            let data = vm.new_native_array(vec![data])?;
            fields.insert(10, ("ruby", data));
        }
        dictionary(vm, fields.into_iter().rev())
    }
}
struct Line {
    characters: Vec<Character>,
    height: f32,
    spacing: f32,
    offset: f32,
}
pub(crate) struct Renderer {
    defaults: Format,
    current: Format,
    options: BTreeMap<String, Value>,
    width: f32,
    height: f32,
    font_scale: f32,
    time_scale: f32,
    vertical: bool,
    lines: Vec<Line>,
    pending: Vec<Character>,
    published: Vec<Character>,
    render_text: Vec<u16>,
    count: usize,
    delay: f32,
    over: bool,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    initialized: bool,
    busy: bool,
    key_waits: Vec<usize>,
    line_indent: f32,
    next_indent: f32,
    indent_stack: Vec<f32>,
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            defaults: defaults(),
            current: defaults(),
            options: BTreeMap::new(),
            width: 0.0,
            height: 0.0,
            font_scale: 1.0,
            time_scale: 1.0,
            vertical: false,
            lines: vec![],
            pending: vec![],
            published: vec![],
            render_text: vec![],
            count: 0,
            delay: 0.0,
            over: false,
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            initialized: false,
            busy: false,
            key_waits: vec![],
            line_indent: 0.0,
            next_indent: 0.0,
            indent_stack: vec![],
        }
    }
}
impl Renderer {
    fn clear(&mut self) {
        self.current = self.defaults.clone();
        self.lines.clear();
        self.pending.clear();
        self.key_waits.clear();
        self.line_indent = 0.0;
        self.next_indent = 0.0;
        self.indent_stack.clear();
        self.published.clear();
        self.render_text.clear();
        self.count = 0;
        self.delay = 0.0;
        self.over = false;
        self.left = if self.vertical { self.width } else { 0.0 };
        self.right = self.left;
        self.top = 0.0;
        self.bottom = 0.0;
        self.initialized = true;
    }
    fn offset(&self) -> f32 {
        self.lines
            .last()
            .map(|l| l.offset + l.height + l.spacing)
            .unwrap_or(0.0)
    }
    fn advance(&self, ch: &Character) -> f32 {
        (if self.vertical { ch.size } else { ch.width }) + number(&ch.format, "Pitch")
    }
    fn option_contains(&self, name: &str, unit: u16) -> bool {
        matches!(self.options.get(name),Some(Value::String(s)) if s.contains(&unit))
    }
    fn pending_width(&self) -> f32 {
        self.pending.iter().map(|c| self.advance(c)).sum()
    }
    fn reset(&mut self, style: bool) {
        for &(key, _, _, _) in FIELDS {
            if style_field(key) == style {
                self.current.insert(key, self.defaults[key].clone());
            }
        }
    }
    fn getter(&self, name: &str) -> Result<Value> {
        if let Some(field) = name.strip_prefix("default") {
            return Ok(self.defaults[field].clone());
        }
        match name {
            "fontScale" => return Ok(real(self.font_scale)),
            "timeScale" => return Ok(real(self.time_scale)),
            "vertical" => return Ok(Value::Integer(self.vertical.into())),
            _ => (),
        }
        ensure!(
            self.initialized,
            "TextRender layout requires clear or setRenderSize"
        );
        Ok(match name {
            "renderText" => Value::String(self.render_text.clone()),
            "renderCount" => Value::Integer(self.count as i64),
            "renderLines" => Value::Integer(self.lines.len() as i64),
            "renderDelay" => real(self.delay * self.time_scale),
            "renderOver" => Value::Integer(self.over.into()),
            "renderLeft" => real(self.left),
            "renderRight" => real(self.right),
            "renderTop" => real(self.top),
            "renderBottom" => real(self.bottom),
            "maxScrollOffset" => real(if self.vertical {
                self.width - self.left
            } else {
                self.height - self.bottom
            }),
            "maxScrollLine" => Value::Integer(0),
            _ => return Err(unsupported(format!("TextRenderBase property {name}"))),
        })
    }
}
impl Services {
    fn renderer(&mut self, id: usize) -> Result<&mut Renderer> {
        self.text_renderers
            .get_mut(&id)
            .context("context has no TextRenderBase native instance")
    }
    pub(crate) fn text_render_call(
        &mut self,
        vm: &mut Vm,
        op: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("TextRenderBase requires an object context");
        };
        let id = reference
            .object
            .context("TextRenderBase requires a non-null context")?;
        if op == "@initialize" {
            self.text_renderers.insert(id, Renderer::default());
            return Ok(Value::Void);
        }
        if op == "@invalidate" {
            self.text_renderers.remove(&id);
            return Ok(Value::Void);
        }
        let arg = |i| {
            args.get(i)
                .with_context(|| format!("TextRenderBase.{op}: missing argument {i}"))
        };
        if let Some(name) = op.strip_prefix("get:") {
            return self.renderer(id)?.getter(name);
        }
        if let Some(name) = op.strip_prefix("set:") {
            let value = arg(0)?;
            let r = self.renderer(id)?;
            if let Some(field) = name.strip_prefix("default") {
                let &(key, _, kind, _) = FIELDS
                    .iter()
                    .find(|f| f.0 == field)
                    .context("unknown TextRender field")?;
                r.defaults.insert(key, convert(value, kind)?);
            } else {
                match name {
                    "fontScale" => r.font_scale = numeric(value)?,
                    "timeScale" => r.time_scale = numeric(value)?,
                    "vertical" => r.vertical = value.integer()? != 0,
                    _ => return Err(unsupported(format!("TextRenderBase property {name}"))),
                }
            }
            return Ok(Value::Void);
        }
        match op {
            "TextRenderBase" => {
                ensure!(
                    !self.renderer(id)?.busy,
                    "cannot reconstruct TextRender during a callback"
                );
                self.text_renderers.insert(id, Renderer::default());
            }
            "finalize" => (),
            "onEval" => {
                return self
                    .call(vm, "Scripts.eval", &[arg(0)?.clone()], budget)?
                    .unary("string");
            }
            "clear" | "setRenderSize" => {
                ensure!(
                    !self.renderer(id)?.busy,
                    "cannot reset TextRender during a callback"
                );
                if op == "setRenderSize" {
                    let (width, height) = (numeric(arg(0)?)?, numeric(arg(1)?)?);
                    let r = self.renderer(id)?;
                    r.width = width;
                    r.height = height;
                }
                let before = font_signature(&self.renderer(id)?.current);
                self.renderer(id)?.clear();
                if before != font_signature(&self.renderer(id)?.current) {
                    self.font_changed(vm, id, context, budget)?;
                }
            }
            "resetFont" | "resetStyle" => {
                let before = font_signature(&self.renderer(id)?.current);
                self.renderer(id)?.reset(op == "resetStyle");
                if before != font_signature(&self.renderer(id)?.current) {
                    self.font_changed(vm, id, context, budget)?;
                }
            }
            "setDefault" | "setFont" | "setStyle" => {
                let value = arg(0)?;
                ensure!(
                    matches!(value, Value::Object(o) if o.object.is_some()),
                    "TextRender settings require an object"
                );
                let before = font_signature(&self.renderer(id)?.current);
                for &(key, attribute, kind, _) in FIELDS {
                    if attribute.is_empty()
                        || op != "setDefault" && style_field(key) != (op == "setStyle")
                    {
                        continue;
                    }
                    let field = vm.get_property(
                        value,
                        &Value::string(attribute),
                        true,
                        false,
                        self,
                        budget,
                    )?;
                    if field != Value::Void {
                        let field = convert(&field, kind)?;
                        let r = self.renderer(id)?;
                        if op == "setDefault" {
                            r.defaults.insert(key, field);
                        } else {
                            r.current.insert(key, field);
                        }
                    }
                }
                if op == "setFont" && before != font_signature(&self.renderer(id)?.current) {
                    self.font_changed(vm, id, context, budget)?;
                }
            }
            "setOption" => {
                let value = arg(0)?;
                ensure!(
                    matches!(value, Value::Object(o) if o.object.is_some()),
                    "TextRender options require an object"
                );
                for name in [
                    "following",
                    "leading",
                    "begin",
                    "end",
                    "kinsoku_max",
                    "vertical",
                    "word_break",
                    "width_time_scale",
                ] {
                    let value =
                        vm.get_property(value, &Value::string(name), true, false, self, budget)?;
                    if value == Value::Void {
                        continue;
                    }
                    if name == "vertical" {
                        self.renderer(id)?.vertical = value.integer()? != 0;
                    } else {
                        self.renderer(id)?.options.insert(name.into(), value);
                    }
                }
            }
            "getCharacters" => {
                let start = arg(0)?.integer()?;
                let count = arg(1)?.integer()?;
                ensure!(
                    start >= 0 && count >= 0,
                    "TextRender character range must be nonnegative"
                );
                let r = self.renderer(id)?;
                let start = usize::try_from(start)?.min(r.published.len());
                let end = if count == 0 {
                    r.published.len()
                } else {
                    start
                        .saturating_add(usize::try_from(count)?)
                        .min(r.published.len())
                };
                let chars = r.published[start..end].to_vec();
                let mut values = Vec::with_capacity(chars.len());
                for ch in chars {
                    charge(budget)?;
                    values.push(ch.value(vm)?);
                }
                return vm.new_native_array(values);
            }
            "getKeyWait" => {
                let positions = self.renderer(id)?.key_waits.clone();
                let mut values = vec![];
                for pos in positions {
                    charge(budget)?;
                    values.push(dictionary(
                        vm,
                        [("pos", Value::Integer(pos as i64)), ("time", real(0.0))],
                    )?);
                }
                return vm.new_native_array(values);
            }
            "calcShowCount" => {
                let time = numeric(arg(0)?)?;
                let r = self.renderer(id)?;
                return Ok(Value::Integer(
                    r.published
                        .iter()
                        .take_while(|c| c.delay * r.time_scale <= time)
                        .count() as i64,
                ));
            }
            "calcLineOffset" => {
                let line = arg(0)?.integer()?;
                let r = self.renderer(id)?;
                if line <= 0 {
                    return Ok(real(0.0));
                }
                if line as usize >= r.lines.len() {
                    return Ok(real(r.bottom));
                }
                // The published block fits the render area, so no scroll is
                // needed for an in-range line. Out-of-range requests use its end.
                return Ok(real(0.0));
            }
            "render" | "done" | "newline" => {
                ensure!(
                    !self.renderer(id)?.busy,
                    "reentrant TextRender layout is not supported"
                );
                ensure!(
                    self.renderer(id)?.initialized,
                    "TextRender layout requires clear or setRenderSize"
                );
                self.renderer(id)?.busy = true;
                let result = if op == "render" {
                    self.render_text(vm, id, context, args, budget)
                } else {
                    self.finish_text_line(vm, id, context, budget).map(|_| {
                        if op == "done" {
                            let r = self.text_renderers.get_mut(&id).expect("checked renderer");
                            r.published =
                                r.lines.iter().flat_map(|l| l.characters.clone()).collect();
                            r.left = if r.vertical { r.width } else { 0.0 };
                            r.right = r.left;
                            r.top = 0.0;
                            r.bottom = 0.0;
                            for ch in &r.published {
                                if r.vertical {
                                    r.left = r.left.min(ch.x - ch.size);
                                    r.bottom = r.bottom.max(ch.y + ch.size);
                                } else {
                                    r.left = r.left.min(ch.x);
                                    r.right = r.right.max(ch.x + ch.width);
                                    r.bottom = r.bottom.max(ch.y + ch.size);
                                }
                                if let Some(ruby) = &ch.ruby {
                                    r.left = r.left.min(ruby.x);
                                    r.right = r.right.max(ruby.x + ruby.width);
                                    r.top = r.top.min(ruby.y);
                                }
                            }
                            if !r.published.is_empty() {
                                let valign = number(&r.current, "Valign");
                                let available = if r.vertical {
                                    -(r.width - r.left)
                                } else {
                                    r.height - r.bottom
                                };
                                let offset = if valign == 0.0 {
                                    (available / 2.0).trunc()
                                } else if valign == 1.0 {
                                    available
                                } else {
                                    0.0
                                };
                                for ch in &mut r.published {
                                    if r.vertical {
                                        ch.x += offset;
                                    } else {
                                        ch.y += offset;
                                    }
                                    if let Some(ruby) = &mut ch.ruby {
                                        if r.vertical {
                                            ruby.x += offset;
                                        } else {
                                            ruby.y += offset;
                                        }
                                    }
                                }
                                if r.vertical {
                                    r.left += offset;
                                    r.right += offset;
                                } else {
                                    r.top += offset;
                                    r.bottom += offset;
                                }
                                // The DLL changes stored coordinates, including on repeated done().
                                let mut published = r.published.iter();
                                for ch in r.lines.iter_mut().flat_map(|l| &mut l.characters) {
                                    *ch =
                                        published.next().expect("published line character").clone();
                                }
                            }
                            r.published.sort_by(|a, b| a.delay.total_cmp(&b.delay));
                        }
                        Value::Void
                    })
                };
                if let Some(r) = self.text_renderers.get_mut(&id) {
                    r.busy = false;
                }
                return result;
            }
            _ => {
                return Err(unsupported(format!(
                    "TextRenderBase.{op} is not implemented"
                )));
            }
        }
        Ok(Value::Void)
    }
    fn text_callback(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        name: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let function = vm.get_property(context, &Value::string(name), true, false, self, budget)?;
        if function == Value::Void {
            return Ok(Value::Void);
        }
        vm.call_function(&function, context, args, self, budget)
    }
    fn text_width(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        text: Vec<u16>,
        size: f32,
        budget: &mut u64,
    ) -> Result<f32> {
        let result = self.text_callback(
            vm,
            context,
            "onGetTextWidth",
            &[Value::String(text), real(size)],
            budget,
        )?;
        ensure!(
            result != Value::Void,
            "TextRender requires an onGetTextWidth callback"
        );
        let width = numeric(&result)?;
        ensure!(width >= 0.0, "TextRender width must be nonnegative");
        Ok(width)
    }
    fn font_changed(
        &mut self,
        vm: &mut Vm,
        id: usize,
        context: &Value,
        budget: &mut u64,
    ) -> Result<()> {
        let f = &self.renderer(id)?.current;
        let data = dictionary(
            vm,
            [
                ("bold", f["Bold"].clone()),
                ("italic", f["Italic"].clone()),
                ("face", f["Face"].clone()),
            ],
        )?;
        self.text_callback(vm, context, "onFontChange", &[data], budget)?;
        Ok(())
    }
    fn finish_text_line(
        &mut self,
        vm: &mut Vm,
        id: usize,
        context: &Value,
        budget: &mut u64,
    ) -> Result<()> {
        if self.renderer(id)?.pending.is_empty() || self.renderer(id)?.over {
            return Ok(());
        }
        let r = self.renderer(id)?;
        let h = r
            .pending
            .iter()
            .map(|c| c.size)
            .fold(number(&r.current, "LineSize") * r.font_scale, f32::max);
        let y = r.offset();
        let limit = if r.vertical { r.width } else { r.height };
        if limit > 0.0 && y + h > limit {
            r.over = true;
            r.pending.clear();
            return Ok(());
        }
        let space_size = number(&r.current, "FontSize") * r.font_scale;
        let measured_space = self.text_width(vm, context, vec![0x3000], space_size, budget)?;
        let r = self.renderer(id)?;
        let space = if r.vertical {
            space_size
        } else {
            measured_space
        };
        let size = r.pending_width();
        let limit = if r.vertical { r.height } else { r.width };
        let align = number(&r.current, "Align");
        let mut x = if align == 0.0 {
            ((limit - size) / 2.0).trunc()
        } else if align == 1.0 {
            limit - size
        } else {
            r.line_indent
        };
        let spaces = if space > 0.0 {
            (x / space).max(0.0) as usize
        } else {
            0
        };
        if spaces > 100_000 {
            return Err(unsupported("TextRender indentation limit exceeded"));
        }
        r.render_text.extend(std::iter::repeat_n(0x3000, spaces));
        let mut characters = std::mem::take(&mut r.pending);
        for ch in &mut characters {
            charge(budget)?;
            if r.vertical {
                ch.x = r.width - y;
                ch.y = x;
            } else {
                ch.x = x;
                ch.y = y + h - ch.size;
            }
            if let Some(ruby) = &mut ch.ruby {
                ruby.x += ch.x;
                ruby.y += ch.y;
            }
            x += r.advance(ch);
            r.render_text.extend_from_slice(&ch.text);
        }
        r.render_text.push(10);
        r.line_indent = r.next_indent;
        r.lines.push(Line {
            characters,
            height: h,
            spacing: number(&r.current, "LineSpacing"),
            offset: y,
        });
        Ok(())
    }
    fn render_text(
        &mut self,
        vm: &mut Vm,
        id: usize,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        ensure!(
            args.len() >= 5,
            "TextRenderBase.render requires five arguments"
        );
        let Value::String(mut units) = args[0].unary("string")? else {
            unreachable!()
        };
        if units.len() > 4 << 20 {
            return Err(unsupported("TextRender input limit exceeded"));
        }
        let auto_indent = args[1].integer()?;
        let base_delay = numeric(&args[2])?;
        let total = numeric(&args[3])?;
        let same = args[4].integer()? != 0;
        if !same {
            let r = self.renderer(id)?;
            for ch in r
                .lines
                .iter_mut()
                .flat_map(|l| &mut l.characters)
                .take(r.published.len())
            {
                ch.delay = 0.0;
            }
        }
        if self.renderer(id)?.options.contains_key("kinsoku_max") {
            return Err(unsupported(
                "TextRender configurable kinsoku limit is not implemented",
            ));
        }
        for name in ["word_break", "width_time_scale"] {
            if self
                .renderer(id)?
                .options
                .get(name)
                .is_some_and(|v| v.integer().unwrap_or(1) != 0)
            {
                return Err(unsupported(format!(
                    "TextRender layout option {name} is not implemented"
                )));
            }
        }
        self.font_changed(vm, id, context, budget)?;
        let size = {
            let r = self.renderer(id)?;
            number(&r.current, "FontSize") * r.font_scale
        };
        self.text_width(vm, context, vec![0x3000], size, budget)?;
        let start_count = self.renderer(id)?.count;
        let (mut i, mut delay, mut step) = (0usize, 0.0f32, base_delay);
        self.renderer(id)?.delay = 0.0;
        let mut ruby_pending: Option<(Vec<u16>, usize, usize)> = None;
        while i < units.len() && units[i] != 0 && !self.renderer(id)?.over {
            charge(budget)?;
            let mut unit = units[i];
            i += 1;
            let escaped = unit == 92;
            if escaped {
                unit = *units.get(i).context("unterminated TextRender escape")?;
                i += 1;
                match unit {
                    110 => {
                        self.finish_text_line(vm, id, context, budget)?;
                        continue;
                    }
                    107 => {
                        let r = self.renderer(id)?;
                        r.key_waits.push(r.count);
                        continue;
                    }
                    120 => continue,
                    119 => {
                        let size = {
                            let r = self.renderer(id)?;
                            number(&r.current, "FontSize") * r.font_scale
                        };
                        let width = self.text_width(vm, context, vec![0x3000], size, budget)?;
                        let r = self.renderer(id)?;
                        if r.pending.is_empty() {
                            r.line_indent += width;
                        }
                        if let Some(last) = r.pending.last_mut() {
                            last.format
                                .insert("Pitch", real(number(&last.format, "Pitch") + width));
                        }
                        continue;
                    }
                    116 => unit = 9,
                    105 => {
                        let r = self.renderer(id)?;
                        r.next_indent = r.line_indent + r.pending_width();
                        continue;
                    }
                    114 => {
                        self.renderer(id)?.next_indent = 0.0;
                        continue;
                    }
                    _ => (),
                }
            }
            if !escaped && unit == 37 {
                let command = *units
                    .get(i)
                    .context("unterminated TextRender format command")?;
                i += 1;
                match command {
                    98 | 105 | 115 | 101 => {
                        let flag = *units.get(i).context("missing TextRender format flag")?;
                        i += 1;
                        let key = match command {
                            98 => "Bold",
                            105 => "Italic",
                            115 => "Shadow",
                            _ => "Edge",
                        };
                        let r = self.renderer(id)?;
                        let value = match flag {
                            48 => Value::Integer(0),
                            49 => Value::Integer(1),
                            _ => r.defaults[key].clone(),
                        };
                        r.current.insert(key, value);
                        if command == 98 || command == 105 {
                            self.font_changed(vm, id, context, budget)?;
                        }
                    }
                    66 | 83 => {
                        let r = self.renderer(id)?;
                        let value = r.defaults[if command == 66 {
                            "BigFontSize"
                        } else {
                            "SmallFontSize"
                        }]
                        .clone();
                        r.current.insert("FontSize", value);
                    }
                    114 => {
                        self.renderer(id)?.reset(false);
                        self.font_changed(vm, id, context, budget)?;
                    }
                    67 | 82 | 76 => {
                        self.renderer(id)?.current.insert(
                            "Align",
                            Value::Integer(match command {
                                67 => 0,
                                82 => 1,
                                _ => -1,
                            }),
                        );
                    }
                    102 => {
                        let text = terminated(&units, &mut i, 59)?;
                        self.renderer(id)?
                            .current
                            .insert("Face", Value::String(text));
                        self.font_changed(vm, id, context, budget)?;
                    }
                    112 | 100 | 119 | 68 | 48..=57 => {
                        if (48..=57).contains(&command) {
                            i -= 1;
                        }
                        if command == 68 && units.get(i) == Some(&36) {
                            i += 1;
                            let name = terminated(&units, &mut i, 59)?;
                            let value = self.text_callback(
                                vm,
                                context,
                                "onLabel",
                                &[Value::String(name)],
                                budget,
                            )?;
                            delay = delay.max(numeric(&value)?);
                        } else {
                            let token = terminated(&units, &mut i, 59)?;
                            let value = String::from_utf16(&token)?
                                .parse::<f32>()
                                .context("invalid TextRender numeric command")?;
                            ensure!(value.is_finite(), "TextRender command must be finite");
                            match command {
                                112 => {
                                    self.renderer(id)?.current.insert("Pitch", real(value));
                                }
                                100 => step = base_delay * value / 100.0,
                                119 => delay += base_delay * value / 100.0,
                                68 => delay = delay.max(value),
                                _ => {
                                    let r = self.renderer(id)?;
                                    let size = number(&r.defaults, "FontSize") * value / 100.0;
                                    r.current.insert("FontSize", real(size));
                                }
                            }
                        }
                    }
                    _ => {
                        return Err(unsupported(format!(
                            "TextRender format command {}",
                            char::from_u32(command as u32).unwrap_or('?')
                        )));
                    }
                }
                continue;
            }
            if !escaped && unit == 35 {
                let token = terminated(&units, &mut i, 59)?;
                let color = u32::from_str_radix(&String::from_utf16(&token)?, 16)
                    .context("invalid TextRender color")?;
                self.renderer(id)?
                    .current
                    .insert("ChColor", Value::Integer((color | 0xff000000) as i64));
                continue;
            }
            if !escaped && unit == 36 {
                let expr = terminated(&units, &mut i, 59)?;
                let value = self
                    .text_callback(vm, context, "onEval", &[Value::String(expr)], budget)?
                    .unary("string")?;
                let Value::String(value) = value else {
                    unreachable!()
                };
                let mut literal = vec![];
                for ch in value {
                    charge(budget)?;
                    if matches!(ch, 37 | 35 | 91 | 36 | 38 | 92) {
                        literal.push(92);
                    }
                    literal.push(ch);
                }
                if units.len().saturating_add(literal.len()) > 4 << 20 {
                    return Err(unsupported("TextRender expanded text limit exceeded"));
                }
                units.splice(i..i, literal);
                continue;
            }
            if !escaped && unit == 91 {
                if self.renderer(id)?.vertical {
                    return Err(unsupported("TextRender vertical ruby is not implemented"));
                }
                let mut ruby = terminated(&units, &mut i, 93)?;
                let count = if let Some(comma) = ruby.iter().rposition(|u| *u == 44) {
                    let n = String::from_utf16(&ruby[comma + 1..])?
                        .parse::<usize>()
                        .context("invalid ruby character count")?;
                    ruby.truncate(comma);
                    n
                } else {
                    1
                };
                ensure!(count > 0, "ruby character count must be positive");
                if count > 100_000 {
                    return Err(unsupported("TextRender ruby character limit exceeded"));
                }
                if count > 1 && ruby.len() % count != 0 {
                    return Err(unsupported(
                        "uneven multi-character ruby spacing is not implemented",
                    ));
                }
                ruby_pending = Some((ruby, count, 0));
                continue;
            }
            if !escaped && unit == 38 {
                return Err(unsupported(
                    "TextRender graphical-character layout is not implemented",
                ));
            }
            if unit == 10 {
                self.finish_text_line(vm, id, context, budget)?;
                continue;
            }
            if unit == 13 {
                continue;
            }
            let mut text = vec![unit];
            if (0xd800..=0xdbff).contains(&unit)
                && units.get(i).is_some_and(|u| (0xdc00..=0xdfff).contains(u))
            {
                text.push(units[i]);
                i += 1;
            }
            let (format, size) = {
                let r = self.renderer(id)?;
                (
                    r.current.clone(),
                    number(&r.current, "FontSize") * r.font_scale,
                )
            };
            let width = self.text_width(vm, context, text.clone(), size, budget)?;
            let ruby = if let Some((text, count, index)) = &mut ruby_pending {
                let chunk =
                    text[*index * text.len() / *count..(*index + 1) * text.len() / *count].to_vec();
                let (rs, offset) = {
                    let r = self.renderer(id)?;
                    (
                        number(&r.current, "RubySize") * r.font_scale,
                        number(&r.current, "RubyOffset") * r.font_scale,
                    )
                };
                let rw = self.text_width(vm, context, chunk.clone(), rs, budget)?;
                *index += 1;
                Some(Ruby {
                    text: chunk,
                    size: rs,
                    width: rw,
                    x: (width - rw) / 2.0,
                    y: -(rs + offset),
                })
            } else {
                None
            };
            if ruby_pending
                .as_ref()
                .is_some_and(|(_, count, index)| count == index)
            {
                ruby_pending = None;
            }
            let mut ch = Character {
                text,
                format,
                size,
                width,
                delay,
                x: 0.0,
                y: 0.0,
                ruby,
            };
            let r = self.renderer(id)?;
            let limit = if r.vertical { r.height } else { r.width };
            if auto_indent < 0
                && r.count == start_count
                && r.pending.is_empty()
                && r.option_contains("begin", unit)
            {
                r.line_indent -= r.advance(&ch);
            }
            let following = r.option_contains("following", unit)
                || auto_indent != 0 && r.option_contains("end", unit);
            if following
                && limit > 0.0
                && r.line_indent + r.pending_width() + r.advance(&ch) > limit
                && r.pending.last().is_some_and(|c| {
                    c.text
                        .first()
                        .is_some_and(|u| r.option_contains("following", *u))
                })
            {
                return Err(unsupported(
                    "TextRender multiple hanging punctuation characters are not implemented",
                ));
            }
            if !r.pending.is_empty()
                && limit > 0.0
                && r.line_indent + r.pending_width() + r.advance(&ch) > limit
                && !following
            {
                let mut carry = vec![];
                while r.pending.len() > 1
                    && r.pending.last().is_some_and(|c| {
                        c.text
                            .first()
                            .is_some_and(|u| r.option_contains("leading", *u))
                    })
                {
                    carry.push(r.pending.pop().expect("nonempty line"));
                }
                self.finish_text_line(vm, id, context, budget)?;
                let r = self.renderer(id)?;
                for mut leading in carry.into_iter().rev() {
                    leading.delay += step;
                    delay += step;
                    leading.x = 0.0;
                    leading.y = 0.0;
                    r.pending.push(leading);
                }
                ch.delay = delay;
            }
            let r = self.renderer(id)?;
            if r.over {
                break;
            }
            r.pending.push(ch);
            if auto_indent != 0 && r.option_contains("begin", unit) {
                r.indent_stack.push(r.next_indent);
                r.next_indent = r.line_indent + r.pending_width();
            }
            if auto_indent != 0 && r.option_contains("end", unit) {
                r.next_indent = r.indent_stack.pop().unwrap_or(0.0);
            }
            r.count += 1;
            r.delay = delay + base_delay;
            delay += step;
            if r.count > 100_000 {
                return Err(unsupported("TextRender character limit exceeded"));
            }
        }
        if total > 0.0 {
            let r = self.renderer(id)?;
            let factor = if r.delay != 0.0 { total / r.delay } else { 0.0 };
            for (index, c) in r
                .lines
                .iter_mut()
                .flat_map(|l| l.characters.iter_mut())
                .chain(r.pending.iter_mut())
                .enumerate()
            {
                if index >= start_count {
                    c.delay *= factor;
                }
            }
            r.delay = total;
        }
        Ok(Value::Integer((!self.renderer(id)?.over).into()))
    }
}
fn terminated(units: &[u16], i: &mut usize, end: u16) -> Result<Vec<u16>> {
    let start = *i;
    while *i < units.len() && units[*i] != end {
        *i += 1;
    }
    ensure!(*i < units.len(), "unterminated TextRender command");
    let result = units[start..*i].to_vec();
    *i += 1;
    Ok(result)
}
