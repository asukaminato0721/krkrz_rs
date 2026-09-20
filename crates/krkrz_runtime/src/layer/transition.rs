//! Crossfade provider and Layer transition lifecycle from LayerIntf/TransIntf.
use super::*;

pub(super) struct Transition {
    pub source: usize,
    pub with_children: bool,
    pub phase: i32,
    duration: u64,
    tick: u64,
    identity: std::rc::Rc<()>,
    start: Option<u64>,
    callback: Option<Value>,
    self_update: bool,
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
        if name != "crossfade" {
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
                    t.phase = ((t.tick.wrapping_sub(start) as u128 * 255 / t.duration as u128)
                        .min(255)) as i32;
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
            if self
                .layers
                .get(&id)
                .is_some_and(|l| l.transition.as_ref().is_some_and(|t| t.phase == 255))
            {
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
    pub(crate) fn complete_window_transitions(&mut self, window: usize) -> Result<()> {
        if !self
            .services
            .layers
            .values()
            .any(|l| l.window == window && l.transition.is_some())
        {
            return Ok(());
        }
        self.capture_window_prepared(&Value::object(window))?;
        Ok(())
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
