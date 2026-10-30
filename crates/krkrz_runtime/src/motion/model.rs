//! Typed PSB motion timelines. No script dispatch or host clock lives here.
use anyhow::{Context, Result, ensure};
use krkrz_assets::{
    media::Image,
    psb::{Document, Value as PsbValue},
};
use krkrz_tjs::unsupported;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{collections::BTreeMap, sync::Arc};

const MAX_IMAGE_BYTES: usize = 128 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SpriteId(usize);

pub(super) struct Sprite {
    pub image: Image,
    pub origin: [f64; 2],
}

pub(super) struct Model {
    pub label: String,
    pub objects: BTreeMap<String, BTreeMap<String, Arc<Clip>>>,
    pub sprites: Vec<Arc<Sprite>>,
    pub bytes: usize,
    pub root: Arc<krkrz_tjs::ReadOnlyData>,
    pub alive: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
pub(super) struct Pose {
    pub sprite: Option<SpriteId>,
    pub position: [f64; 2],
    pub origin: Option<[f64; 2]>,
    pub zoom: [f64; 2],
    pub flip: [bool; 2],
    pub angle: f64,
    pub opacity: f64,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            sprite: None,
            position: [0.0; 2],
            origin: None,
            zoom: [1.0; 2],
            flip: [false; 2],
            angle: 0.0,
            opacity: 255.0,
        }
    }
}

struct Key {
    time: f64,
    pose: Option<Pose>,
    interpolate: bool,
    curves: [Option<Curve>; 4],
}

pub(super) struct Node {
    pub label: String,
    parent: Option<usize>,
    keys: Vec<Key>,
    visible: bool,
}

pub(super) struct Clip {
    pub end: f64,
    pub loop_begin: Option<f64>,
    pub nodes: Vec<Node>,
    priority: Vec<(f64, Vec<usize>)>,
}

#[derive(Deserialize)]
struct Root {
    id: String,
    spec: String,
    label: String,
    source: BTreeMap<String, Source>,
    object: BTreeMap<String, Character>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(rename = "type")]
    kind: i32,
    icon: BTreeMap<String, Icon>,
}

#[derive(Deserialize)]
struct Icon {
    width: u32,
    height: u32,
    #[serde(rename = "originX", default)]
    origin_x: f64,
    #[serde(rename = "originY", default)]
    origin_y: f64,
    #[serde(default)]
    compress: Option<String>,
    #[serde(default = "one")]
    resolution: f64,
    pixel: Resource,
}

#[derive(Deserialize)]
struct Resource {
    offset: usize,
    length: usize,
}

#[derive(Deserialize)]
struct Character {
    motion: BTreeMap<String, Motion>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Motion {
    last_time: f64,
    loop_time: f64,
    layer: Vec<Layer>,
    #[serde(default)]
    variable: Vec<serde_json::Value>,
    #[serde(default)]
    parameter: Vec<serde_json::Value>,
    #[serde(default)]
    priority: Vec<Priority>,
}

#[derive(Deserialize)]
struct Priority {
    time: f64,
    content: Option<Vec<usize>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Layer {
    label: String,
    #[serde(rename = "type")]
    kind: i32,
    #[serde(default)]
    coordinate: i32,
    #[serde(default = "default_inheritance")]
    inherit_mask: u64,
    #[serde(default)]
    mesh_transform: i32,
    #[serde(default)]
    stencil_type: i32,
    #[serde(default)]
    particle: i32,
    #[serde(default)]
    ground_correction: i32,
    #[serde(default)]
    parameterize: Option<serde_json::Value>,
    #[serde(default = "visible")]
    visible: i32,
    #[serde(default)]
    frame_list: Vec<Frame>,
    #[serde(default)]
    children: Vec<Layer>,
}

#[derive(Deserialize)]
struct Frame {
    time: f64,
    #[serde(rename = "type")]
    kind: i32,
    content: Option<Content>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(default)]
struct Content {
    mask: u64,
    src: String,
    icon: Option<String>,
    coord: [f64; 3],
    ox: Option<f64>,
    oy: Option<f64>,
    zx: f64,
    zy: f64,
    angle: f64,
    opa: f64,
    fx: i32,
    fy: i32,
    bm: i32,
    sx: f64,
    sy: f64,
    color: Vec<i64>,
    ccc: Option<Curve>,
    zcc: Option<Curve>,
    acc: Option<Curve>,
    occ: Option<Curve>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            mask: 0,
            src: String::new(),
            icon: None,
            coord: [0.0; 3],
            ox: None,
            oy: None,
            zx: 1.0,
            zy: 1.0,
            angle: 0.0,
            opa: 255.0,
            fx: 0,
            fy: 0,
            bm: 0,
            sx: 0.0,
            sy: 0.0,
            color: vec![],
            ccc: None,
            zcc: None,
            acc: None,
            occ: None,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Deserialize)]
struct Curve {
    x: [f64; 4],
    y: [f64; 4],
}

impl Curve {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.x.iter().chain(&self.y).all(|n| n.is_finite()),
            "non-finite motion easing curve"
        );
        ensure!(
            self.x[0] == 0.0 && self.x[3] == 1.0 && self.y[0] == 0.0 && self.y[3] == 1.0,
            "invalid motion easing endpoints"
        );
        ensure!(
            (0.0..=1.0).contains(&self.x[1]) && (0.0..=1.0).contains(&self.x[2]),
            "motion easing is not monotone in time"
        );
        Ok(())
    }
    fn sample(&self, time: f64) -> f64 {
        let cubic = |p: &[f64; 4], t: f64| {
            let u = 1.0 - t;
            p[0] * u * u * u + 3.0 * p[1] * u * u * t + 3.0 * p[2] * u * t * t + p[3] * t * t * t
        };
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..32 {
            let middle = (low + high) * 0.5;
            if cubic(&self.x, middle) < time {
                low = middle;
            } else {
                high = middle;
            }
        }
        cubic(&self.y, (low + high) * 0.5)
    }
}

fn one() -> f64 {
    1.0
}
fn visible() -> i32 {
    1
}
fn default_inheritance() -> u64 {
    1532
}
fn finite(value: f64) -> Result<f64> {
    ensure!(value.is_finite(), "non-finite motion parameter");
    Ok(value)
}

impl Model {
    pub fn parse(bytes: &[u8], budget: &mut u64) -> Result<Self> {
        ensure!(bytes.len() <= MAX_IMAGE_BYTES, "motion PSB exceeds 128 MiB");
        let document = Document::parse(bytes)?;
        let root_bytes = retained_root_bytes(&document.root)?;
        let root: Root = serde_json::from_value(serde_json::to_value(&document.root)?)
            .context("invalid motion PSB structure")?;
        ensure!(root.id == "motion", "PSB is not a motion file");
        if root.spec != "krkr" {
            return Err(unsupported(format!(
                "Motion PSB platform {} is not implemented",
                root.spec
            )));
        }
        let mut sprites = Vec::new();
        let mut names = BTreeMap::new();
        let mut retained = 0usize;
        for (source_name, source) in root.source {
            if source.kind != 1 {
                return Err(unsupported(
                    "Motion external image sources are not implemented",
                ));
            }
            for (name, icon) in source.icon {
                ensure!(
                    icon.width > 0
                        && icon.height > 0
                        && u64::from(icon.width) * u64::from(icon.height) <= 16 * 1024 * 1024,
                    "invalid motion image dimensions"
                );
                ensure!(
                    icon.resolution == 1.0,
                    "Motion image resolution must be one"
                );
                let size = icon.width as usize * icon.height as usize * 4;
                *budget = budget
                    .checked_sub(size as u64 / 64 + 1)
                    .ok_or_else(|| unsupported("Motion image decode execution budget exceeded"))?;
                retained = retained
                    .checked_add(size)
                    .context("motion image memory overflow")?;
                ensure!(retained <= MAX_IMAGE_BYTES, "motion images exceed 128 MiB");
                let end = icon
                    .pixel
                    .offset
                    .checked_add(icon.pixel.length)
                    .context("motion pixel range overflow")?;
                let data = bytes
                    .get(icon.pixel.offset..end)
                    .context("motion pixel data outside PSB")?;
                let mut rgba = match icon.compress.as_deref().unwrap_or("RGBA8") {
                    "RGBA8" | "" => {
                        ensure!(
                            data.is_empty() || data.len() == size,
                            "invalid motion RGBA image size"
                        );
                        if data.is_empty() {
                            vec![0; size]
                        } else {
                            data.to_vec()
                        }
                    }
                    "RL" => decode_rl(data, size)?,
                    format => {
                        return Err(unsupported(format!(
                            "Motion texture format {format} is not implemented"
                        )));
                    }
                };
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                names.insert(format!("src/{source_name}/{name}"), SpriteId(sprites.len()));
                sprites.push(Arc::new(Sprite {
                    image: Image {
                        width: icon.width,
                        height: icon.height,
                        rgba,
                    },
                    origin: [finite(icon.origin_x)?, finite(icon.origin_y)?],
                }));
            }
        }
        let mut objects = BTreeMap::new();
        for (character, object) in root.object {
            let mut motions = BTreeMap::new();
            for (name, motion) in object.motion {
                if !motion.variable.is_empty() || !motion.parameter.is_empty() {
                    return Err(unsupported(
                        "Motion parameter-driven timelines are not implemented",
                    ));
                }
                let end = finite(motion.last_time)?;
                let begin = finite(motion.loop_time)?;
                ensure!(
                    end >= 0.0 && (begin < 0.0 || begin < end),
                    "invalid motion timeline range"
                );
                let mut nodes = Vec::new();
                for layer in motion.layer {
                    append_layer(layer, None, &names, &mut nodes, 0)?;
                }
                let mut priority = Vec::new();
                for key in motion.priority {
                    ensure!(
                        key.time.is_finite() && key.time >= 0.0,
                        "invalid motion priority time"
                    );
                    let order = key.content.unwrap_or_default();
                    ensure!(
                        order.iter().all(|i| *i < nodes.len()),
                        "motion priority index outside layer tree"
                    );
                    priority.push((key.time, order));
                }
                ensure!(
                    priority.windows(2).all(|w| w[0].0 < w[1].0),
                    "motion priority times are not ordered"
                );
                motions.insert(
                    name,
                    Arc::new(Clip {
                        end,
                        loop_begin: (begin >= 0.0).then_some(begin),
                        nodes,
                        priority,
                    }),
                );
            }
            objects.insert(character, motions);
        }
        Ok(Self {
            label: root.label,
            objects,
            sprites,
            bytes: retained + root_bytes,
            root: crate::psb_file::convert(document.root, bytes, budget)?,
            alive: Arc::new(AtomicBool::new(true)),
        })
    }
}

fn retained_root_bytes(value: &PsbValue) -> Result<usize> {
    let mut bytes = 128usize;
    let children = match value {
        PsbValue::String(value) => value.len().saturating_mul(2),
        PsbValue::Resource { length, .. } => *length,
        PsbValue::List(values) => values.iter().try_fold(0usize, |total, v| {
            Ok::<_, anyhow::Error>(total.saturating_add(retained_root_bytes(v)?))
        })?,
        PsbValue::Object(values) => values.iter().try_fold(0usize, |total, (key, v)| {
            Ok::<_, anyhow::Error>(
                total
                    .saturating_add(key.len().saturating_mul(2))
                    .saturating_add(retained_root_bytes(v)?),
            )
        })?,
        _ => 0,
    };
    bytes = bytes.saturating_add(children);
    ensure!(bytes <= MAX_IMAGE_BYTES, "motion root data exceeds 128 MiB");
    Ok(bytes)
}

impl Drop for Model {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

fn decode_rl(data: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(size);
    let mut input = data;
    while output.len() < size {
        let (&tag, rest) = input.split_first().context("truncated motion RL image")?;
        let count = if tag & 128 == 0 {
            usize::from(tag) + 1
        } else {
            usize::from(tag & 127) + 3
        };
        ensure!(
            count * 4 <= size - output.len(),
            "motion RL run exceeds image"
        );
        let consumed = if tag & 128 == 0 { count * 4 } else { 4 };
        let pixels = rest.get(..consumed).context("truncated motion RL pixels")?;
        if tag & 128 == 0 {
            output.extend_from_slice(pixels);
        } else {
            for _ in 0..count {
                output.extend_from_slice(pixels);
            }
        }
        input = &rest[consumed..];
    }
    ensure!(input.is_empty(), "motion RL image has trailing bytes");
    Ok(output)
}

fn append_layer(
    layer: Layer,
    parent: Option<usize>,
    sprites: &BTreeMap<String, SpriteId>,
    nodes: &mut Vec<Node>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth < 64 && nodes.len() < 10000,
        "motion layer tree exceeds limit"
    );
    if !matches!(layer.kind, 0 | 2)
        || layer.coordinate != 0
        || layer.mesh_transform != 0
        || layer.stencil_type != 0
        || layer.particle != 0
        || layer.ground_correction != 0
        || layer.parameterize.is_some()
    {
        return Err(unsupported(
            "Motion mesh, particle, camera, and stencil layers are not implemented",
        ));
    }
    if parent.is_some() && layer.inherit_mask != default_inheritance() {
        return Err(unsupported(
            "Motion custom layer inheritance is not implemented",
        ));
    }
    let mut keys = Vec::new();
    for frame in layer.frame_list {
        if frame.extra.keys().any(|key| key != "metadata") {
            return Err(unsupported(
                "Motion action, synchronization, and legacy easing frames are not implemented",
            ));
        }
        ensure!(
            frame.time.is_finite() && frame.time >= 0.0 && (0..=3).contains(&frame.kind),
            "invalid motion frame"
        );
        let mut curves = [None, None, None, None];
        let pose = if frame.kind == 0 {
            None
        } else {
            let content = frame
                .content
                .context("visible motion frame has no content")?;
            if content
                .extra
                .keys()
                .any(|key| !matches!(key.as_str(), "mask" | "metadata"))
            {
                return Err(unsupported("Motion frame contains an unimplemented effect"));
            }
            if content.mask & !0x2bfff != 0
                || (content.mask & 2 != 0 && content.coord[2] != 0.0)
                || (content.mask & 0x20000 != 0 && content.bm != 0)
                || (content.mask & 128 != 0 && content.sx != 0.0)
                || (content.mask & 256 != 0 && content.sy != 0.0)
                || (content.mask & 512 != 0
                    && content.color.iter().any(|c| *c != -1 && *c != 0xffffffff))
            {
                return Err(unsupported(
                    "Motion depth, blend effects, slant, and vertex colors are not implemented",
                ));
            }
            let source = if let Some(icon) = content.icon {
                format!("src/{}/{icon}", content.src)
            } else {
                content.src
            };
            if source.starts_with("motion/") {
                return Err(unsupported(
                    "Motion references to other timelines are not implemented",
                ));
            }
            let sprite = if source.is_empty() {
                None
            } else {
                Some(
                    *sprites
                        .get(&source)
                        .with_context(|| format!("motion sprite not found: {source}"))?,
                )
            };
            let pose = Pose {
                sprite,
                position: if content.mask & 2 != 0 {
                    [finite(content.coord[0])?, finite(content.coord[1])?]
                } else {
                    [0.0; 2]
                },
                origin: match (content.mask & 1 != 0, content.ox, content.oy) {
                    (false, _, _) | (true, None, None) => None,
                    (true, x, y) => Some([finite(x.unwrap_or(0.0))?, finite(y.unwrap_or(0.0))?]),
                },
                zoom: [
                    if content.mask & 32 != 0 {
                        finite(content.zx)?
                    } else {
                        1.0
                    },
                    if content.mask & 64 != 0 {
                        finite(content.zy)?
                    } else {
                        1.0
                    },
                ],
                flip: [
                    content.mask & 4 != 0 && content.fx != 0,
                    content.mask & 8 != 0 && content.fy != 0,
                ],
                angle: if content.mask & 16 != 0 {
                    finite(content.angle)?
                } else {
                    0.0
                },
                opacity: if content.mask & 1024 != 0 {
                    finite(content.opa)?.clamp(0.0, 255.0)
                } else {
                    255.0
                },
            };
            curves = [
                content.ccc.filter(|_| content.mask & 2048 != 0),
                content.zcc.filter(|_| content.mask & 8192 != 0),
                content.acc.filter(|_| content.mask & 4096 != 0),
                content.occ.filter(|_| content.mask & 32768 != 0),
            ];
            for curve in curves.iter().flatten() {
                curve.validate()?;
            }
            Some(pose)
        };
        keys.push(Key {
            time: frame.time,
            pose,
            interpolate: frame.kind >= 2,
            curves,
        });
    }
    ensure!(
        keys.windows(2).all(|w| w[0].time < w[1].time),
        "motion frame times are not ordered"
    );
    let index = nodes.len();
    nodes.push(Node {
        label: layer.label,
        parent,
        keys,
        visible: layer.visible != 0,
    });
    for child in layer.children {
        append_layer(child, Some(index), sprites, nodes, depth + 1)?;
    }
    Ok(())
}

impl Node {
    fn pose(&self, time: f64, model: &Model) -> Option<Pose> {
        let index = self
            .keys
            .partition_point(|key| key.time <= time)
            .checked_sub(1)?;
        let a = &self.keys[index];
        let mut pose = a.pose?;
        let origin = |pose: &Pose| {
            pose.origin.unwrap_or_else(|| {
                pose.sprite
                    .map_or([0.0; 2], |sprite| model.sprites[sprite.0].origin)
            })
        };
        pose.origin = Some(origin(&pose));
        if a.interpolate
            && let Some(b) = self.keys.get(index + 1)
            && let Some(next) = b.pose
        {
            let ratio = ((time - a.time) / (b.time - a.time)).clamp(0.0, 1.0);
            let r: Vec<_> = a
                .curves
                .iter()
                .map(|curve| curve.as_ref().map_or(ratio, |curve| curve.sample(ratio)))
                .collect();
            for axis in 0..2 {
                pose.position[axis] += (next.position[axis] - pose.position[axis]) * r[0];
                pose.zoom[axis] += (next.zoom[axis] - pose.zoom[axis]) * r[1];
            }
            pose.origin = Some(std::array::from_fn(|axis| {
                origin(&pose)[axis] + (origin(&next)[axis] - origin(&pose)[axis]) * ratio
            }));
            pose.angle += (next.angle - pose.angle) * r[2];
            pose.opacity += (next.opacity - pose.opacity) * r[3];
        }
        Some(pose)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Matrix(pub [f64; 6]);

impl Matrix {
    pub fn pose(pose: &Pose) -> Self {
        let (sin, cos) = pose.angle.to_radians().sin_cos();
        let zoom: [f64; 2] =
            std::array::from_fn(|axis| pose.zoom[axis] * if pose.flip[axis] { -1.0 } else { 1.0 });
        Self([
            cos * zoom[0],
            sin * zoom[0],
            -sin * zoom[1],
            cos * zoom[1],
            pose.position[0],
            pose.position[1],
        ])
    }
    pub fn append(self, other: Self) -> Self {
        let [a, b, c, d, x, y] = self.0;
        let [e, f, g, h, u, v] = other.0;
        Self([
            a * e + c * f,
            b * e + d * f,
            a * g + c * h,
            b * g + d * h,
            a * u + c * v + x,
            b * u + d * v + y,
        ])
    }
    pub fn point(self, x: f64, y: f64) -> [f64; 2] {
        let [a, b, c, d, u, v] = self.0;
        [a * x + c * y + u, b * x + d * y + v]
    }
}

pub(super) struct Draw {
    pub sprite: Arc<Sprite>,
    pub matrix: Matrix,
    pub opacity: f64,
}

impl Clip {
    pub fn commands(&self, model: &Model, time: f64, base: Pose) -> Vec<Draw> {
        let mut world: Vec<Option<(Matrix, f64)>> = Vec::with_capacity(self.nodes.len());
        let mut commands = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let parent = node
                .parent
                .map_or(Some((Matrix::pose(&base), base.opacity)), |p| world[p]);
            let local = node.pose(time, model);
            let composed = parent.zip(local).map(|((matrix, opacity), pose)| {
                (
                    matrix.append(Matrix::pose(&pose)),
                    opacity * pose.opacity / 255.0,
                )
            });
            world.push(composed);
            if node.visible
                && let Some((matrix, opacity)) = composed
                && let Some(pose) = local
                && let Some(sprite) = pose.sprite
            {
                let sprite = model.sprites[sprite.0].clone();
                let origin = pose.origin.unwrap_or(sprite.origin);
                let origin = Matrix([1.0, 0.0, 0.0, 1.0, -origin[0], -origin[1]]);
                commands.push((
                    index,
                    Draw {
                        sprite,
                        matrix: matrix.append(origin),
                        opacity,
                    },
                ));
            }
        }
        if let Some((_, order)) = self.priority.iter().rev().find(|(at, _)| *at <= time) {
            let mut ranks = vec![usize::MAX; self.nodes.len()];
            for (rank, index) in order.iter().enumerate() {
                ranks[*index] = rank;
            }
            commands.retain(|(index, _)| ranks[*index] != usize::MAX);
            commands.sort_by_key(|(index, _)| ranks[*index]);
        }
        commands.into_iter().map(|(_, command)| command).collect()
    }
}
