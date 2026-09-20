//! Focus management follows Kirikiri LayerIntf.cpp and LayerManager.cpp.
use super::*;

fn value(id: Option<usize>) -> Value {
    id.map_or(Value::NULL, bound)
}
impl Services {
    pub(super) fn layer_node_enabled(&self, id: usize, visible: bool) -> bool {
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(layer) = self.layers.get(&id) else {
                return false;
            };
            if !layer.enabled || (visible && !layer.visible) {
                return false;
            }
            current = layer.parent;
        }
        true
    }
    fn layer_node_focusable(&self, id: usize) -> bool {
        self.layers.get(&id).is_some_and(|l| l.focusable) && self.layer_node_enabled(id, true)
    }
    fn layer_descendant(&self, id: usize, ancestor: usize) -> bool {
        let mut current = Some(id);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.layers.get(&id).and_then(|l| l.parent);
        }
        false
    }
    fn layer_nodes(&self, root: usize) -> Vec<usize> {
        let mut nodes = Vec::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if let Some(layer) = self.layers.get(&id) {
                nodes.push(id);
                pending.extend(layer.children.iter().rev());
            }
        }
        nodes
    }
    fn layer_focused(&self, root: usize) -> Option<usize> {
        self.layers
            .get(&root)
            .and_then(|l| l.focused_layer)
            .filter(|id| self.layers.contains_key(id))
    }
    pub(super) fn layer_forget_focus(&mut self, id: usize) {
        let roots: Vec<_> = self
            .layers
            .iter()
            .filter_map(|(&root, layer)| {
                layer
                    .focused_layer
                    .filter(|&f| self.layer_descendant(f, id))
                    .map(|_| root)
            })
            .collect();
        for root in roots {
            self.layers.get_mut(&root).unwrap().focused_layer = None;
        }
        for layer in self.layers.values_mut() {
            if layer.focus_work == Some(id) {
                layer.focus_work = None;
            }
        }
    }
    pub(super) fn layer_event(
        &mut self,
        vm: &mut Vm,
        id: usize,
        name: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<()> {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| unsupported("Layer event execution budget exhausted"))?;
        if !self
            .layers
            .get(&id)
            .is_some_and(|l| l.constructed && self.windows.contains_key(&l.window))
        {
            return Ok(());
        }
        let context = bound(id);
        let function =
            vm.get_property(&context, &Value::string(name), true, false, self, budget)?;
        if !matches!(function, Value::Void) {
            vm.call_function(&function, &context, args, self, budget)?;
        }
        Ok(())
    }
    fn layer_focus_candidate(&self, v: &Value) -> Result<Option<usize>> {
        if matches!(v, Value::Void) {
            return Ok(None);
        }
        let id = object(v)?;
        ensure!(
            id.is_none_or(|id| self.layers.contains_key(&id)),
            "focus target is not a Layer"
        );
        Ok(id)
    }
    fn layer_search_focus(
        &mut self,
        vm: &mut Vm,
        id: usize,
        forward: bool,
        budget: &mut u64,
    ) -> Result<Option<usize>> {
        let root = self
            .layers
            .get(&id)
            .context("focus source was invalidated")?
            .root;
        let nodes = self.layer_nodes(root);
        let found = nodes.iter().position(|&n| n == id).and_then(|start| {
            if nodes.len() < 2 {
                return None;
            }
            (1..=nodes.len())
                .map(|step| {
                    nodes[if forward {
                        (start + step) % nodes.len()
                    } else {
                        (start + nodes.len() - step) % nodes.len()
                    }]
                })
                .find(|&n| self.layer_node_focusable(n) && self.layers[&n].join_focus_chain)
        });
        self.layers.get_mut(&id).unwrap().focus_work = found;
        self.layer_event(
            vm,
            id,
            if forward {
                "onSearchNextFocusable"
            } else {
                "onSearchPrevFocusable"
            },
            &[value(found)],
            budget,
        )?;
        Ok(self.layers.get(&id).and_then(|l| l.focus_work))
    }
    pub(super) fn layer_set_focus(
        &mut self,
        vm: &mut Vm,
        root: usize,
        mut target: Option<usize>,
        forward: bool,
        budget: &mut u64,
    ) -> Result<bool> {
        let previous = self.layer_focused(root);
        if let Some(id) = target {
            if !self.layer_node_focusable(id) {
                return Ok(false);
            }
            self.layers.get_mut(&id).unwrap().focus_work = Some(id);
            self.layer_event(
                vm,
                id,
                "onBeforeFocus",
                &[bound(id), value(previous), Value::Integer(forward.into())],
                budget,
            )?;
            target = self.layers.get(&id).and_then(|l| l.focus_work);
            if target.is_some_and(|id| !self.layer_node_focusable(id)) {
                return Ok(false);
            }
        }
        let Some(manager) = self.layers.get_mut(&root) else {
            return Ok(false);
        };
        if manager.focused_layer == target {
            return Ok(false);
        }
        ensure!(
            !manager.focus_lock,
            "cannot change focus while processing focus callbacks"
        );
        manager.focus_lock = true;
        let previous = manager.focused_layer;
        manager.focused_layer = target;
        let result = (|| {
            if let Some(id) = previous {
                self.layer_event(vm, id, "onBlur", &[value(target)], budget)?;
            }
            if let Some(id) = self.layer_focused(root) {
                self.layer_event(
                    vm,
                    id,
                    "onFocus",
                    &[value(previous), Value::Integer(forward.into())],
                    budget,
                )?;
            }
            Ok(true)
        })();
        if let Some(manager) = self.layers.get_mut(&root) {
            manager.focus_lock = false;
        }
        result
    }
    pub(super) fn layer_step_focus(
        &mut self,
        vm: &mut Vm,
        root: usize,
        forward: bool,
        budget: &mut u64,
    ) -> Result<Option<usize>> {
        let target = if let Some(id) = self.layer_focused(root) {
            self.layer_search_focus(vm, id, forward, budget)?
        } else {
            self.layer_nodes(root)
                .into_iter()
                .find(|&id| self.layers[&id].join_focus_chain && self.layer_node_focusable(id))
        };
        if target.is_some() {
            self.layer_set_focus(vm, root, target, forward, budget)?;
        }
        Ok(target)
    }
    pub(super) fn layer_blur_tree(
        &mut self,
        vm: &mut Vm,
        id: usize,
        budget: &mut u64,
    ) -> Result<()> {
        let root = self
            .layers
            .get(&id)
            .context("Layer invalidated before detachment")?
            .root;
        if let Some(focused) = self.layer_focused(root)
            && self.layer_descendant(focused, id)
        {
            let next = self.layer_search_focus(vm, id, true, budget)?;
            self.layer_set_focus(vm, root, next.filter(|&next| next != focused), true, budget)?;
        }
        Ok(())
    }
    pub(super) fn layer_pointer_offset(&self, id: usize) -> [i32; 2] {
        let mut point = [0i32; 2];
        let mut id = id;
        while let Some(layer) = self.layers.get(&id) {
            let Some(parent) = layer.parent else {
                break;
            };
            point[0] = point[0].wrapping_add(layer.left);
            point[1] = point[1].wrapping_add(layer.top);
            id = parent;
        }
        point
    }
    pub(crate) fn window_focus(
        &mut self,
        vm: &mut Vm,
        id: usize,
        setting: bool,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let window = self
            .windows
            .get(&id)
            .context("context has no Window native instance")?;
        ensure!(window.constructed, "Window constructor has not run");
        let root = object(&window.primary_layer)?;
        if setting {
            let target = self.layer_focus_candidate(
                args.first().context("Window.focusedLayer: missing value")?,
            )?;
            if let Some(root) = root {
                self.layer_set_focus(vm, root, target, true, budget)?;
            }
            Ok(Value::Void)
        } else {
            Ok(value(root.and_then(|root| self.layer_focused(root))))
        }
    }
    pub(super) fn layer_focus_call(
        &mut self,
        vm: &mut Vm,
        id: usize,
        op: &str,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Option<Value>> {
        let root = self.layers[&id].root;
        let arg = |i: usize| {
            args.get(i)
                .with_context(|| format!("Layer.{op}: missing argument {i}"))
        };
        let result = match op {
            "get:cursorX" | "get:cursorY" => {
                let layer = &self.layers[&id];
                let axis = usize::from(op == "get:cursorY");
                let coordinate = if layer.constructed {
                    self.windows
                        .get(&layer.window)
                        .context("Layer Window was invalidated")?
                        .pointer[axis]
                        .wrapping_sub(self.layer_pointer_offset(id)[axis])
                } else {
                    0
                };
                Value::Integer(coordinate.into())
            }
            "set:cursorX" => {
                self.layers.get_mut(&id).unwrap().cursor_x_work = int(arg(0)?)?;
                Value::Void
            }
            "set:cursorY" | "setCursorPos" => {
                let layer = &self.layers[&id];
                if layer.constructed {
                    let offset = self.layer_pointer_offset(id);
                    let x = if op == "setCursorPos" {
                        int(arg(0)?)?
                    } else {
                        layer.cursor_x_work
                    };
                    let y = int(arg(usize::from(op == "setCursorPos"))?)?;
                    self.windows
                        .get_mut(&layer.window)
                        .context("Layer Window was invalidated")?
                        .pointer = [x.wrapping_add(offset[0]), y.wrapping_add(offset[1])];
                }
                Value::Void
            }
            "get:focusable" => Value::Integer(self.layers[&id].focusable.into()),
            "get:joinFocusChain" => Value::Integer(self.layers[&id].join_focus_chain.into()),
            "get:nodeFocusable" => Value::Integer(self.layer_node_focusable(id).into()),
            "get:nodeEnabled" => Value::Integer(self.layer_node_enabled(id, false).into()),
            "get:focused" => Value::Integer((self.layer_focused(root) == Some(id)).into()),
            "get:nextFocusable" | "get:prevFocusable" => {
                value(self.layer_search_focus(vm, id, op == "get:nextFocusable", budget)?)
            }
            "focus" => Value::Integer(
                if self.layers[&id].constructed {
                    self.layer_set_focus(
                        vm,
                        root,
                        Some(id),
                        args.first().map(Value::truth).transpose()?.unwrap_or(true),
                        budget,
                    )?
                } else {
                    false
                }
                .into(),
            ),
            "focusNext" | "focusPrev" => {
                value(self.layer_step_focus(vm, root, op == "focusNext", budget)?)
            }
            "set:joinFocusChain" => {
                self.layers.get_mut(&id).unwrap().join_focus_chain = arg(0)?.truth()?;
                Value::Void
            }
            "set:focusable" | "set:visible" | "set:enabled" => {
                let enabled = arg(0)?.truth()?;
                let layer = &self.layers[&id];
                let old = match op {
                    "set:focusable" => layer.focusable,
                    "set:visible" => layer.visible,
                    _ => layer.enabled,
                };
                if old == enabled {
                    return Ok(Some(Value::Void));
                }
                ensure!(
                    op != "set:visible" || enabled || !layer.primary,
                    "cannot hide primary Layer"
                );
                let before = if op == "set:enabled" {
                    self.layer_nodes(root)
                        .into_iter()
                        .map(|id| (id, self.layer_node_enabled(id, false)))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let previously_focusable = self.layer_node_focusable(id);
                let layer = self.layers.get_mut(&id).unwrap();
                match op {
                    "set:focusable" => layer.focusable = enabled,
                    "set:visible" => layer.visible = enabled,
                    _ => layer.enabled = enabled,
                };
                let result = (|| -> Result<()> {
                    if !enabled && let Some(focused) = self.layer_focused(root) {
                        let blur = if op == "set:focusable" {
                            previously_focusable && focused == id
                        } else {
                            self.layer_descendant(focused, id)
                        };
                        if blur {
                            let mut next = self.layer_search_focus(vm, id, true, budget)?;
                            if next == Some(focused) {
                                next = None;
                            }
                            self.layer_set_focus(vm, root, next, true, budget)?;
                        }
                    }
                    Ok(())
                })();
                for (node, old) in before {
                    if self.layers.contains_key(&node) {
                        let new = self.layer_node_enabled(node, false);
                        if old != new {
                            self.layer_event(
                                vm,
                                node,
                                if new {
                                    "onNodeEnabled"
                                } else {
                                    "onNodeDisabled"
                                },
                                &[],
                                budget,
                            )?;
                        }
                    }
                }
                result?;
                Value::Void
            }
            "releaseCapture" => {
                let window = self.layers[&id].window;
                if let Some(window) = self.windows.get_mut(&window) {
                    window.input.capture = None;
                    window.input.release_capture = true;
                }
                Value::Void
            }
            "onHitTest"
            | "onClick"
            | "onDoubleClick"
            | "onMouseDown"
            | "onMouseUp"
            | "onMouseMove"
            | "onMouseEnter"
            | "onMouseLeave"
            | "onMouseWheel"
            | "onKeyDown"
            | "onKeyUp"
            | "onKeyPress"
            | "onBeforeFocus"
            | "onPaint"
            | "onTransitionCompleted"
            | "onSearchNextFocusable"
            | "onSearchPrevFocusable"
            | "onFocus"
            | "onBlur"
            | "onNodeEnabled"
            | "onNodeDisabled" => {
                let keys: &[&str] = match op {
                    "onClick" | "onDoubleClick" => &["x", "y"],
                    "onHitTest" => &["x", "y", "hit"],
                    "onMouseDown" | "onMouseUp" => &["x", "y", "button", "shift"],
                    "onMouseMove" => &["x", "y", "shift"],
                    "onMouseWheel" => &["shift", "delta", "x", "y"],
                    "onKeyDown" | "onKeyUp" => &["key", "shift", "process"],
                    "onKeyPress" => &["key", "process"],
                    "onTransitionCompleted" => &["dest", "src"],
                    "onBeforeFocus" => &["layer", "blurred", "direction"],
                    "onFocus" => &["blurred", "direction"],
                    "onBlur" => &["focused"],
                    "onSearchNextFocusable" | "onSearchPrevFocusable" => &["layer"],
                    _ => &[],
                };
                let owner = self.layers[&id].action_owner.clone();
                let mut result = Value::Void;
                if owner != Value::NULL && op != "onHitTest" {
                    ensure!(
                        args.len() >= keys.len(),
                        "Layer.{op}: missing event arguments"
                    );
                    let action = vm.get_property(
                        &owner,
                        &Value::string("action"),
                        true,
                        false,
                        self,
                        budget,
                    )?;
                    if !matches!(action, Value::Void) {
                        let event = vm.new_dictionary()?;
                        vm.set_member(&event, &Value::string("type"), Value::string(op))?;
                        vm.set_member(&event, &Value::string("target"), bound(id))?;
                        for (key, v) in keys.iter().zip(args) {
                            vm.set_member(&event, &Value::string(key), v.clone())?;
                        }
                        result = vm.call_function(&action, &owner, &[event], self, budget)?;
                    }
                }
                if matches!(
                    op,
                    "onBeforeFocus" | "onSearchNextFocusable" | "onSearchPrevFocusable"
                ) {
                    let target = self.layer_focus_candidate(arg(0)?)?;
                    if let Some(layer) = self.layers.get_mut(&id) {
                        layer.focus_work = target;
                    }
                }
                self.layer_input_default(vm, id, op, args, budget)?;
                result
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }
}
