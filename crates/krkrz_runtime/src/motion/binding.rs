use super::{
    model::{Clip, Model, Pose},
    raster,
};
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{ObjectRef, Value, Vm, unsupported};
use lru::LruCache;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ManagerId(usize);
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PlayerId(usize);

enum Manager {
    Allocated,
    Ready(Resources),
}
struct Resources {
    owner: Value,
    capacity: usize,
    loaded: BTreeMap<String, Arc<Model>>,
    order: Vec<String>,
    cached: LruCache<String, Arc<Model>>,
}

impl Resources {
    fn cache(&mut self, name: String, model: Arc<Model>) {
        self.cached.put(name, model);
        while self.cached.len() > 64
            || self.cached.iter().map(|(_, m)| m.bytes).sum::<usize>() > self.capacity
        {
            self.cached.pop_lru();
        }
    }
}

struct Playing {
    model: Arc<Model>,
    clip: Arc<Clip>,
    tick: f64,
    playing: bool,
    first_progress: bool,
}

struct Player {
    manager: Value,
    character: String,
    motion: String,
    current: Option<Playing>,
    pose: Pose,
    speed: f64,
}

#[derive(Default)]
pub(crate) struct State {
    managers: BTreeMap<ManagerId, Manager>,
    players: BTreeMap<PlayerId, Option<Player>>,
}

impl State {
    pub(crate) fn gc_trace(&self, id: usize, edges: &mut Vec<Value>) {
        if let Some(Manager::Ready(resources)) = self.managers.get(&ManagerId(id)) {
            edges.push(resources.owner.clone());
        }
        if let Some(Some(player)) = self.players.get(&PlayerId(id)) {
            edges.push(player.manager.clone());
        }
    }
    fn resources(&self, id: ManagerId) -> Result<&Resources> {
        match self.managers.get(&id) {
            Some(Manager::Ready(resources)) => Ok(resources),
            _ => anyhow::bail!("Motion.ResourceManager is not initialized"),
        }
    }
    fn resources_mut(&mut self, id: ManagerId) -> Result<&mut Resources> {
        match self.managers.get_mut(&id) {
            Some(Manager::Ready(resources)) => Ok(resources),
            _ => anyhow::bail!("Motion.ResourceManager is not initialized"),
        }
    }
    fn player(&self, id: PlayerId) -> Result<&Player> {
        self.players
            .get(&id)
            .and_then(Option::as_ref)
            .context("Motion.Player is not initialized")
    }
    fn player_mut(&mut self, id: PlayerId) -> Result<&mut Player> {
        self.players
            .get_mut(&id)
            .and_then(Option::as_mut)
            .context("Motion.Player is not initialized")
    }
    fn retained_bytes(&self) -> usize {
        let mut seen = BTreeSet::new();
        let mut bytes = 0;
        let mut add = |model: &Arc<Model>| {
            if seen.insert(Arc::as_ptr(model)) {
                bytes += model.bytes;
            }
        };
        for manager in self.managers.values() {
            if let Manager::Ready(resources) = manager {
                for model in resources.loaded.values() {
                    add(model);
                }
                for (_, model) in &resources.cached {
                    add(model);
                }
            }
        }
        for player in self.players.values().flatten() {
            if let Some(current) = &player.current {
                add(&current.model);
            }
        }
        bytes
    }
}

fn object(value: &Value) -> Result<usize> {
    match value {
        Value::Object(ObjectRef {
            object: Some(id), ..
        }) => Ok(*id),
        _ => anyhow::bail!("Motion requires a non-null object"),
    }
}
fn number(value: &Value) -> Result<f64> {
    let value = value.real()?;
    ensure!(value.is_finite(), "Motion requires a finite number");
    Ok(value)
}

impl Services {
    pub(crate) fn motion_call(
        &mut self,
        vm: &mut Vm,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let (class, operation) = operation
            .split_once('.')
            .context("invalid Motion native operation")?;
        let id = object(context)?;
        let arg = |i| args.get(i).context("missing Motion argument");
        if class == "ResourceManager" {
            let id = ManagerId(id);
            return match operation {
                "@initialize" => {
                    self.motion.managers.insert(id, Manager::Allocated);
                    Ok(Value::Void)
                }
                "@invalidate" => {
                    self.motion.managers.remove(&id);
                    Ok(Value::Void)
                }
                "ResourceManager" => {
                    ensure!(
                        self.motion.managers.contains_key(&id),
                        "context has no Motion.ResourceManager native instance"
                    );
                    let capacity = usize::try_from(arg(1)?.integer()?)
                        .context("Motion cache size must be nonnegative and fit the host")?;
                    self.motion.managers.insert(
                        id,
                        Manager::Ready(Resources {
                            owner: arg(0)?.clone(),
                            capacity,
                            loaded: BTreeMap::new(),
                            order: Vec::new(),
                            cached: LruCache::unbounded(),
                        }),
                    );
                    Ok(Value::Void)
                }
                "finalize" => Ok(Value::Void),
                "load" => {
                    let name = arg(0)?.text();
                    let path = self.storage.resolve(&name)?;
                    ensure!(
                        self.motion.resources(id)?.loaded.len() < 64
                            || self.motion.resources(id)?.loaded.contains_key(&path),
                        "too many loaded motion files"
                    );
                    let existing = self.motion.resources(id)?.loaded.get(&path).cloned();
                    let model =
                        if let Some(model) = existing {
                            model
                        } else if let Some(model) =
                            self.motion.resources(id)?.cached.peek(&path).cloned()
                        {
                            model
                        } else {
                            let bytes = self.read_storage(&name)?;
                            *budget = budget.checked_sub(bytes.len() as u64 / 64 + 1).ok_or_else(
                                || unsupported("Motion PSB load execution budget exceeded"),
                            )?;
                            let model = Arc::new(Model::parse(&bytes, budget)?);
                            ensure!(
                                model.bytes
                                    <= (256usize << 20)
                                        .saturating_sub(self.motion.retained_bytes()),
                                "session motion resources exceed 256 MiB"
                            );
                            model
                        };
                    let root = vm.new_readonly_data(model.root.clone(), model.alive.clone())?;
                    let resources = self.motion.resources_mut(id)?;
                    resources.cached.pop(&path);
                    resources.order.retain(|old| old != &path);
                    resources.order.push(path.clone());
                    resources.loaded.insert(path, model);
                    Ok(root)
                }
                "unload" => {
                    let path = self.storage.resolve(&arg(0)?.text())?;
                    let resources = self.motion.resources_mut(id)?;
                    resources.order.retain(|old| old != &path);
                    if let Some(model) = resources.loaded.remove(&path) {
                        resources.cache(path, model);
                    }
                    Ok(Value::Void)
                }
                "unloadAll" => {
                    let resources = self.motion.resources_mut(id)?;
                    resources.order.clear();
                    for (path, model) in std::mem::take(&mut resources.loaded) {
                        resources.cache(path, model);
                    }
                    Ok(Value::Void)
                }
                "clearCache" => {
                    self.motion.resources_mut(id)?.cached.clear();
                    Ok(Value::Void)
                }
                _ => Err(unsupported(format!(
                    "Motion.ResourceManager.{operation} is not implemented"
                ))),
            };
        }
        ensure!(class == "Player", "unknown Motion native class");
        let id = PlayerId(id);
        match operation {
            "@initialize" => {
                self.motion.players.insert(id, None);
                return Ok(Value::Void);
            }
            "@invalidate" => {
                self.motion.players.remove(&id);
                return Ok(Value::Void);
            }
            "Player" => {
                ensure!(
                    self.motion.players.contains_key(&id),
                    "context has no Motion.Player native instance"
                );
                self.motion.resources(ManagerId(object(arg(0)?)?))?;
                self.motion.players.insert(
                    id,
                    Some(Player {
                        manager: arg(0)?.clone(),
                        character: String::new(),
                        motion: String::new(),
                        current: None,
                        pose: Pose::default(),
                        speed: 1.0,
                    }),
                );
                return Ok(Value::Void);
            }
            "finalize" => return Ok(Value::Void),
            "play" | "set:motion" => {
                let name = arg(0)?.text();
                let flags = if operation == "play" {
                    args.get(1).map(Value::integer).transpose()?.unwrap_or(0)
                } else {
                    0
                };
                if !matches!(flags, 0 | 1) {
                    return Err(unsupported(format!(
                        "Motion play flags {flags} are not implemented"
                    )));
                }
                let player = self.motion.player(id)?;
                if flags == 0
                    && player.motion == name
                    && player.current.as_ref().is_some_and(|p| p.playing)
                {
                    return Ok(Value::Void);
                }
                let character = &player.character;
                let resources = self.motion.resources(ManagerId(object(&player.manager)?))?;
                let selected = resources
                    .order
                    .iter()
                    .rev()
                    .filter_map(|path| resources.loaded.get(path))
                    .find_map(|model| {
                        let character = character
                            .strip_prefix(&format!("{}/", model.label))
                            .unwrap_or(character);
                        model
                            .objects
                            .get(character)
                            .and_then(|motions| motions.get(&name))
                            .map(|clip| (model.clone(), clip.clone()))
                    });
                let (model, clip) =
                    selected.with_context(|| format!("motion not found: {character}/{name}"))?;
                let player = self.motion.player_mut(id)?;
                player.motion = name;
                player.current = Some(Playing {
                    model,
                    clip,
                    tick: 0.0,
                    playing: true,
                    first_progress: true,
                });
                return Ok(Value::Void);
            }
            "draw" => {
                let player = self.motion.player(id)?;
                let target = object(arg(0)?)?;
                let (clip, face, hold) = self
                    .layers
                    .get(&target)
                    .context("Motion draw target is not a Layer")?
                    .motion_surface()?;
                let nodes = player.current.as_ref().map_or(0, |p| p.clip.nodes.len());
                *budget = budget
                    .checked_sub(nodes as u64 * 64)
                    .ok_or_else(|| unsupported("Motion timeline execution budget exceeded"))?;
                let commands = player.current.as_ref().map_or_else(Vec::new, |p| {
                    p.clip.commands(&p.model, p.tick * 0.06, player.pose)
                });
                let prepared = raster::prepare(&commands, clip, budget)?;
                if prepared.is_empty() {
                    return Ok(Value::Void);
                }
                let available = crate::layer::image_available(&self.layers, target);
                let image = self
                    .layers
                    .get_mut(&target)
                    .unwrap()
                    .plugin_image_mut(available)?;
                raster::render(image, &prepared, face, hold);
                return Ok(Value::Void);
            }
            "clear" => {
                self.motion.player(id)?;
                let target = arg(0)?.clone();
                let layer = self
                    .layers
                    .get(&object(&target)?)
                    .context("Motion clear target is not a Layer")?;
                let (clip, _, _) = layer.motion_surface()?;
                let color = args.get(1).cloned().unwrap_or(Value::Integer(0));
                let available = crate::layer::image_available(&self.layers, object(&target)?);
                if self
                    .layers
                    .get_mut(&object(&target)?)
                    .unwrap()
                    .clear_motion_alpha(color.integer()? as u32, available, budget)?
                {
                    return Ok(Value::Void);
                }
                let args = [
                    Value::Integer(clip[0].into()),
                    Value::Integer(clip[1].into()),
                    Value::Integer((clip[2] - clip[0]).into()),
                    Value::Integer((clip[3] - clip[1]).into()),
                    color,
                ];
                return self.layer_call(vm, "fillRect", &target, &args, budget);
            }
            "getLayerNames" => {
                let player = self.motion.player(id)?;
                let query = arg(0)?.text();
                let names = player.current.as_ref().map_or_else(Vec::new, |p| {
                    p.clip
                        .nodes
                        .iter()
                        .filter(|node| node.label.starts_with(&query))
                        .map(|node| Value::string(&node.label))
                        .collect()
                });
                return vm.new_native_array(names);
            }
            _ => {}
        }
        let player = self.motion.player_mut(id)?;
        let current = player.current.as_ref();
        let result = match operation {
            "get:resourceManager" => player.manager.clone(),
            "get:chara" | "get:stealthChara" => Value::string(&player.character),
            "get:motion" | "get:stealthMotion" => Value::string(&player.motion),
            "set:chara" | "set:stealthChara" => {
                player.character = arg(0)?.text();
                player.motion.clear();
                player.current = None;
                Value::Void
            }
            "get:tickCount" => Value::Real(current.map_or(0.0, |p| p.tick)),
            "get:frameTickCount" => Value::Real(current.map_or(0.0, |p| p.tick * 0.06)),
            "get:frameLastTime" => Value::Real(current.map_or(0.0, |p| p.clip.end)),
            "get:frameLoopTime" => {
                Value::Real(current.map_or(0.0, |p| p.clip.loop_begin.unwrap_or(-1.0)))
            }
            "get:playing" | "get:allplaying" => {
                Value::Integer(current.is_some_and(|p| p.playing).into())
            }
            "get:hasCamera" | "get:syncWaiting" => Value::Integer(0),
            "get:speed" => Value::Real(player.speed),
            "set:speed" => {
                let speed = number(arg(0)?)?;
                ensure!(speed >= 0.0, "Motion reverse playback is not implemented");
                player.speed = speed;
                Value::Void
            }
            "get:x" | "get:left" => Value::Real(player.pose.position[0]),
            "get:y" | "get:top" => Value::Real(player.pose.position[1]),
            "set:x" | "set:left" => {
                player.pose.position[0] = number(arg(0)?)?;
                Value::Void
            }
            "set:y" | "set:top" => {
                player.pose.position[1] = number(arg(0)?)?;
                Value::Void
            }
            "get:zoomX" => Value::Real(player.pose.zoom[0]),
            "get:zoomY" => Value::Real(player.pose.zoom[1]),
            "set:zoomX" => {
                player.pose.zoom[0] = number(arg(0)?)?;
                Value::Void
            }
            "set:zoomY" => {
                player.pose.zoom[1] = number(arg(0)?)?;
                Value::Void
            }
            "get:angleDeg" => Value::Real(player.pose.angle),
            "get:angleRad" => Value::Real(player.pose.angle.to_radians()),
            "set:angleDeg" => {
                player.pose.angle = number(arg(0)?)?;
                Value::Void
            }
            "set:angleRad" => {
                player.pose.angle = number(arg(0)?)?.to_degrees();
                Value::Void
            }
            "get:opacity" => Value::Integer(player.pose.opacity as i64),
            "set:opacity" => {
                player.pose.opacity = number(arg(0)?)?.clamp(0.0, 255.0);
                Value::Void
            }
            "setCoord" => {
                player.pose.position = [number(arg(0)?)?, number(arg(1)?)?];
                Value::Void
            }
            "setZoom" => {
                player.pose.zoom = [number(arg(0)?)?, number(arg(1)?)?];
                Value::Void
            }
            "stop" => {
                if let Some(current) = &mut player.current {
                    current.playing = false;
                }
                Value::Void
            }
            "progress" => {
                let delta = number(arg(0)?)?;
                ensure!(delta >= 0.0, "Motion progress interval must be nonnegative");
                if let Some(current) = &mut player.current
                    && current.playing
                {
                    if current.first_progress {
                        current.first_progress = false;
                    } else {
                        let tick = current.tick + delta * player.speed;
                        ensure!(tick.is_finite(), "Motion playback time overflow");
                        current.tick = tick;
                        let frame = tick * 0.06;
                        if frame >= current.clip.end {
                            if let Some(begin) = current.clip.loop_begin {
                                current.tick = (begin
                                    + (frame - begin).rem_euclid(current.clip.end - begin))
                                    / 0.06;
                            } else {
                                current.playing = false;
                            }
                        }
                    }
                }
                Value::Void
            }
            "set:tickCount" | "set:frameTickCount" => {
                let tick = number(arg(0)?)?
                    / if operation == "set:frameTickCount" {
                        0.06
                    } else {
                        1.0
                    };
                ensure!(
                    tick >= 0.0 && tick.is_finite(),
                    "invalid Motion seek position"
                );
                if let Some(current) = &mut player.current {
                    current.tick = tick;
                    current.first_progress = false;
                }
                Value::Void
            }
            _ => {
                return Err(unsupported(format!(
                    "Motion.Player.{operation} is not implemented"
                )));
            }
        };
        Ok(result)
    }
}
