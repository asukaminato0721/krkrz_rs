//! Crossfade provider and Layer transition lifecycle from LayerIntf/TransIntf.
use super::*;

pub(super) struct Transition {
    pub source: usize,
    pub with_children: bool,
    pub phase: i32,
    rule: Option<Rule>,
    duration: u64,
    tick: u64,
    identity: std::rc::Rc<()>,
    start: Option<u64>,
    callback: Option<Value>,
    self_update: bool,
}
struct Rule {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    vague: i32,
}
impl Transition {
    fn phase_max(&self) -> i32 {
        255 + self.rule.as_ref().map_or(0, |r| r.vague)
    }
}
impl Services {
    pub(super) fn layer_transition_call(
        &mut self,
        vm: &mut Vm,
        id: usize,
        op: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        if op == "stopTransition" {
            self.layer_stop_transition(vm, id, true, budget)?;
            return Ok(Value::Void);
        }
        ensure!(!args.is_empty(), "Layer.beginTransition: missing arguments");
        ensure!(
            self.layers[&id].transition.is_none(),
            "Layer is already in a transition"
        );
        let name = args[0].text();
        if !matches!(name.as_str(), "crossfade" | "universal") {
            return Err(unsupported(format!("transition provider {name}")));
        }
        let with_children = args
            .get(1)
            .filter(|v| !matches!(v, Value::Void))
            .map(Value::truth)
            .transpose()?
            .unwrap_or(true);
        let source = object(args.get(2).unwrap_or(&Value::NULL))?
            .context("transition requires a source Layer")?;
        let src = self
            .layers
            .get(&source)
            .context("transition source is not a Layer")?;
        ensure!(
            src.transition.as_ref().is_none_or(|t| t.source != id),
            "mutual layer transition is invalid"
        );
        let dst = &self.layers[&id];
        ensure!(
            src.root == dst.root,
            "transition layers must belong to the same tree"
        );
        let dimensions = |l: &Layer| -> Result<(i32, i32)> {
            if with_children {
                Ok((l.width, l.height))
            } else {
                let image = l.bitmap()?;
                Ok((image.width as i32, image.height as i32))
            }
        };
        ensure!(
            dimensions(src)? == dimensions(dst)?,
            "transition layer sizes do not match"
        );
        let options = args.get(3).context("transition requires a time option")?;
        let time = vm.get_property(options, &Value::string("time"), true, false, self, budget)?;
        ensure!(
            !matches!(time, Value::Void),
            "transition requires a time option"
        );
        let duration = time.integer()?.max(2) as u64;
        let rule = if name == "universal" {
            let vague =
                vm.get_property(options, &Value::string("vague"), true, false, self, budget)?;
            let vague = if matches!(vague, Value::Void) {
                64
            } else {
                vague.integer()? as i32
            };
            ensure!(
                (0..=i32::MAX - 255).contains(&vague),
                "invalid universal transition vague"
            );
            let rule =
                vm.get_property(options, &Value::string("rule"), true, false, self, budget)?;
            ensure!(
                !matches!(rule, Value::Void),
                "universal transition requires a rule image"
            );
            let name = rule.text();
            let resolved = self
                .storage
                .resolve(&name)
                .ok()
                .or_else(|| self.suggest_graphic(&name))
                .with_context(|| format!("cannot load transition rule {name}"))?;
            let bytes = self.read_storage(&resolved)?;
            let palette = bytes.starts_with(b"BM")
                || (bytes.starts_with(b"\x89PNG") && bytes.get(25) == Some(&3));
            let (image, _) = self.read_graphic(&resolved, budget)?;
            if !palette && !bytes.starts_with(b"\x89PNG") {
                return Err(unsupported(format!(
                    "grayscale transition rule format: {resolved}"
                )));
            }
            let pixels = image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    if palette {
                        ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8
                    } else {
                        p[2]
                    }
                })
                .collect();
            Some(Rule {
                width: image.width as usize,
                height: image.height as usize,
                pixels,
                vague,
            })
        } else {
            None
        };
        let callback = vm.get_property(
            options,
            &Value::string("callback"),
            true,
            false,
            self,
            budget,
        )?;
        let callback = if matches!(callback, Value::Void) {
            None
        } else {
            object(&callback)?;
            Some(callback)
        };
        let self_update = vm
            .get_property(
                options,
                &Value::string("selfupdate"),
                true,
                false,
                self,
                budget,
            )?
            .integer()?
            != 0;
        let start = callback.as_ref().map(|_| 0);
        // Option access can execute script and invalidate either Layer.
        ensure!(
            self.layers.contains_key(&source) && self.layers.contains_key(&id),
            "transition Layer was invalidated while reading options"
        );
        ensure!(
            self.layers[&id].transition.is_none(),
            "Layer is already in a transition"
        );
        self.layers.get_mut(&id).unwrap().transition = Some(Transition {
            source,
            with_children,
            phase: 0,
            rule,
            duration,
            tick: if callback.is_some() { 0 } else { self.time_ms },
            identity: std::rc::Rc::new(()),
            start,
            callback,
            self_update,
        });
        Ok(Value::Void)
    }

    pub(super) fn layer_stop_transition(
        &mut self,
        vm: &mut Vm,
        id: usize,
        notify: bool,
        budget: &mut u64,
    ) -> Result<()> {
        let Some(transition) = self.layers.get_mut(&id).and_then(|l| l.transition.take()) else {
            return Ok(());
        };
        let source = transition.source;
        if !self.layers.contains_key(&source) {
            return Ok(());
        }
        let dest_pos = (
            self.layers[&id].left,
            self.layers[&id].top,
            self.layers[&id].visible,
        );
        let src_pos = (
            self.layers[&source].left,
            self.layers[&source].top,
            self.layers[&source].visible,
        );
        self.layer_exchange(id, source, !transition.with_children);
        let dst = self.layers.get_mut(&id).unwrap();
        (dst.left, dst.top, dst.visible) = src_pos;
        let src = self.layers.get_mut(&source).unwrap();
        (src.left, src.top, src.visible) = dest_pos;
        if notify {
            self.layer_event(
                vm,
                id,
                "onTransitionCompleted",
                &[bound(id), bound(source)],
                budget,
            )?;
        }
        Ok(())
    }

    // keep_children keeps children at their positions in the tree. Exchange
    // moves subtrees instead, including the ancestor/descendant case.
    fn layer_exchange(&mut self, a: usize, b: usize, keep_children: bool) {
        if a == b {
            return;
        }
        let swap = |id| {
            if id == a {
                b
            } else if id == b {
                a
            } else {
                id
            }
        };
        let (ap, bp) = (self.layers[&a].parent, self.layers[&b].parent);
        let ancestor_child = |this: usize, ancestor: usize| {
            let mut node = this;
            while let Some(parent) = self.layers[&node].parent {
                if parent == ancestor {
                    return Some(node);
                }
                node = parent;
            }
            None
        };
        let a_path = ancestor_child(b, a);
        let b_path = ancestor_child(a, b);
        let (ac, bc) = (
            self.layers[&a].children.clone(),
            self.layers[&b].children.clone(),
        );
        let (aa, ba) = (self.layers[&a].absolute, self.layers[&b].absolute);
        for layer in self.layers.values_mut() {
            for child in &mut layer.children {
                *child = swap(*child);
            }
            layer.children_dirty = true;
        }
        if keep_children {
            self.layers.get_mut(&a).unwrap().children = bc.into_iter().map(swap).collect();
            self.layers.get_mut(&b).unwrap().children = ac.into_iter().map(swap).collect();
        } else if let Some(path) = a_path {
            self.layers.get_mut(&a).unwrap().children =
                ac.into_iter().filter(|c| *c != path).collect();
            let mut children = bc;
            children.push(if path == b { a } else { path });
            self.layers.get_mut(&b).unwrap().children = children;
        } else if let Some(path) = b_path {
            self.layers.get_mut(&b).unwrap().children =
                bc.into_iter().filter(|c| *c != path).collect();
            let mut children = ac;
            children.push(if path == a { b } else { path });
            self.layers.get_mut(&a).unwrap().children = children;
        }
        self.layers.get_mut(&a).unwrap().parent = bp.map(swap);
        self.layers.get_mut(&b).unwrap().parent = ap.map(swap);
        self.layers.get_mut(&a).unwrap().absolute = ba;
        self.layers.get_mut(&b).unwrap().absolute = aa;
        let edges: Vec<_> = self
            .layers
            .iter()
            .flat_map(|(id, l)| l.children.iter().map(|c| (*id, *c)))
            .collect();
        for (parent, child) in edges {
            self.layers.get_mut(&child).unwrap().parent = Some(parent);
        }
        let old_root = self.layers[&a].root;
        let root = swap(old_root);
        if root != old_root {
            if let Some(manager) = self.layer_managers.remove(&old_root) {
                self.layer_managers.insert(root, manager);
            }
            for layer in self.layers.values_mut().filter(|l| l.root == old_root) {
                layer.root = root;
                layer.primary = false;
            }
            self.layers.get_mut(&root).unwrap().primary = true;
            let window = self.layers[&root].window;
            if object(&self.windows[&window].primary_layer).ok().flatten() == Some(old_root) {
                self.windows.get_mut(&window).unwrap().primary_layer = bound(root);
            }
        }
    }

    pub(super) fn layer_invalidate_transition(
        &mut self,
        vm: &mut Vm,
        id: usize,
        budget: &mut u64,
    ) -> Result<()> {
        self.layer_stop_transition(vm, id, false, budget)?;
        let destinations: Vec<_> = self
            .layers
            .iter()
            .filter(|(_, l)| l.transition.as_ref().is_some_and(|t| t.source == id))
            .map(|(id, _)| *id)
            .collect();
        for destination in destinations {
            self.layer_stop_transition(vm, destination, false, budget)?;
        }
        Ok(())
    }

    fn layer_transition_tick(&mut self, vm: &mut Vm, id: usize, budget: &mut u64) -> Result<()> {
        let Some(t) = self.layers.get(&id).and_then(|l| l.transition.as_ref()) else {
            return Ok(());
        };
        let identity = t.identity.clone();
        let callback = t.callback.clone();
        let tick = if let Some(callback) = callback {
            vm.call_function(&callback, &Value::object(0), &[], self, budget)?
                .integer()? as u64
        } else {
            self.time_ms
        };
        if let Some(t) = self.layers.get_mut(&id).and_then(|l| l.transition.as_mut())
            && std::rc::Rc::ptr_eq(&identity, &t.identity)
        {
            t.tick = tick;
        }
        Ok(())
    }

    pub(super) fn layer_before_completion(
        &mut self,
        vm: &mut Vm,
        root: usize,
        budget: &mut u64,
    ) -> Result<()> {
        let mut pending = vec![root];
        let mut visited = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            *budget = budget
                .checked_sub(1)
                .ok_or_else(|| unsupported("layer completion execution budget exceeded"))?;
            if !visited.insert(id) {
                continue;
            }
            let Some(layer) = self.layers.get_mut(&id) else {
                continue;
            };
            if std::mem::take(&mut layer.call_on_paint) {
                self.layer_event(vm, id, "onPaint", &[], budget)?;
            }
            if self
                .layers
                .get(&id)
                .is_some_and(|l| l.transition.as_ref().is_some_and(|t| t.self_update))
            {
                self.layer_transition_tick(vm, id, budget)?;
            }
            if let Some(layer) = self.layers.get_mut(&id) {
                if let Some(t) = &mut layer.transition {
                    let start = *t.start.get_or_insert(t.tick);
                    t.phase = ((t.tick.wrapping_sub(start) as u128 * t.phase_max() as u128
                        / t.duration as u128)
                        .min(t.phase_max() as u128)) as i32;
                }
                pending.extend(layer.children.iter().rev());
            }
        }
        Ok(())
    }

    pub(super) fn layer_after_completion(
        &mut self,
        vm: &mut Vm,
        root: usize,
        budget: &mut u64,
    ) -> Result<()> {
        let mut pending = vec![root];
        let mut visited = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if self.layers.get(&id).is_some_and(|l| {
                l.transition
                    .as_ref()
                    .is_some_and(|t| t.phase == t.phase_max())
            }) {
                self.layer_stop_transition(vm, id, true, budget)?;
            }
            if let Some(layer) = self.layers.get(&id) {
                pending.extend(layer.children.iter().rev());
            }
        }
        Ok(())
    }
}

impl crate::Session {
    pub(crate) fn dispatch_layer_transitions(&mut self) -> Result<()> {
        let ids: Vec<_> = self
            .services
            .layers
            .iter()
            .filter(|(_, l)| l.transition.as_ref().is_some_and(|t| !t.self_update))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.services
                .layer_transition_tick(&mut self.vm, id, &mut self.budget)?;
            let mut current = Some(id);
            let mut visible = true;
            while let Some(node) = current {
                let Some(layer) = self.services.layers.get(&node) else {
                    break;
                };
                visible &= layer.visible;
                current = layer.parent;
            }
            if !visible {
                self.services
                    .layer_stop_transition(&mut self.vm, id, true, &mut self.budget)?;
            }
        }
        Ok(())
    }
    pub(crate) fn complete_window_transitions(&mut self, window: usize) -> Result<Option<Image>> {
        if !self
            .services
            .layers
            .values()
            .any(|l| l.window == window && l.transition.is_some())
        {
            return Ok(None);
        }
        self.capture_window_prepared(&Value::object(window))
            .map(Some)
    }
}

pub(super) fn crossfade(destination: &mut Image, source: &Image, kind: i32, phase: i32) {
    if phase == 0 {
        return;
    }
    if phase == 255 {
        destination.rgba.copy_from_slice(&source.rgba);
        return;
    }
    for (d, s) in destination
        .rgba
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(source.rgba.as_chunks::<4>().0.iter())
    {
        if matches!(kind, 2 | 13) {
            let p = phase + i32::from(phase > 127);
            let a1 = d[3] as i32;
            let a2 = s[3] as i32;
            let weight =
                straight_alpha_weight(((a1 * (256 - p)) >> 8) as u8, ((a2 * p) >> 8) as u8);
            for c in 0..3 {
                d[c] = (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * weight) >> 8)) as u8;
            }
            d[3] = (a1 + (((a2 - a1) * p) >> 8)) as u8;
        } else {
            for c in 0..4 {
                d[c] = (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * phase) >> 8)) as u8;
            }
        }
    }
}

impl Transition {
    pub(super) fn blend(
        &self,
        destination: &mut Image,
        source: &Image,
        kind: i32,
        origin: [i64; 2],
    ) {
        let Some(rule) = &self.rule else {
            crossfade(destination, source, kind, self.phase);
            return;
        };
        if self.phase == 0 {
            return;
        }
        if self.phase == self.phase_max() {
            destination.rgba.copy_from_slice(&source.rgba);
            return;
        }
        let width = destination.width as usize;
        let rule_at = |x: usize, y: usize| {
            let x = (origin[0] + x as i64).rem_euclid(rule.width as i64) as usize;
            let y = (origin[1] + y as i64).rem_euclid(rule.height as i64) as usize;
            rule.pixels[y * rule.width + x] as i32
        };
        for (i, (d, s)) in destination
            .rgba
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(source.rgba.as_chunks::<4>().0.iter())
            .enumerate()
        {
            let (x, y) = (i % width, i / width);
            let level = rule_at(x, y);
            let low = self.phase - rule.vague;
            let opaque = !matches!(kind, 2 | 12 | 13);
            if rule.vague < 512 {
                // The original MMX opaque path switches pairs together; the
                // odd tail always blends. Alpha paths switch each pixel.
                let partner = if opaque {
                    (x / 2 * 2 + 1 < width).then(|| rule_at(x ^ 1, y))
                } else {
                    Some(level)
                };
                if let Some(partner) = partner {
                    if level >= self.phase && partner >= self.phase {
                        continue;
                    }
                    if level < low && partner < low {
                        *d = *s;
                        continue;
                    }
                }
            }
            let weight = if level < low {
                255
            } else if level >= self.phase {
                0
            } else {
                255 - ((level as i64 - low as i64) * 255 / rule.vague as i64) as i32
            };
            if matches!(kind, 2 | 13) {
                let a1 = (d[3] as i32 * (256 - weight)) >> 8;
                let a2 = (s[3] as i32 * weight) >> 8;
                let color = straight_alpha_weight(a1 as u8, a2 as u8);
                for c in 0..3 {
                    d[c] = (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * color) >> 8)) as u8;
                }
                d[3] = if rule.vague < 512 {
                    (255 - (255 - a1) * (255 - a2) / 255) as u8
                } else {
                    (d[3] as i32 + (((s[3] as i32 - d[3] as i32) * weight) >> 8)) as u8
                };
            } else if opaque {
                let inverse = 255 - weight;
                for c in 0..4 {
                    d[c] = (s[c] as i32 + ((((d[c] as i32 - s[c] as i32) * inverse) >> 9) * 2))
                        .clamp(0, 255) as u8;
                }
            } else {
                for c in 0..4 {
                    d[c] = (d[c] as i32 + (((s[c] as i32 - d[c] as i32) * weight) >> 8)) as u8;
                }
            }
        }
    }
}

impl Transition {
    pub(super) fn gc_trace(&self, out: &mut Vec<Value>) {
        out.push(Value::object(self.source));
        out.extend(self.callback.iter().cloned());
    }
}
