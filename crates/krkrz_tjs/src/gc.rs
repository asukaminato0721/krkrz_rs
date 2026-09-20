//! Collection at explicit host safe points. IDs are never reused, so a stale
//! external handle fails validation instead of naming an unrelated object.
use crate::{Host, ReadOnlyData, Value, Vm, object::{Object, ObjectKind}};
use anyhow::{Result, ensure};
use std::{collections::{BTreeMap, BTreeSet}, ops::{Index, IndexMut}, sync::Arc};

pub(crate) struct ObjectArena {
    entries: BTreeMap<usize, Object>,
    next: usize,
}
impl ObjectArena {
    pub fn new(global: Object) -> Self {
        Self { entries: BTreeMap::from([(0, global)]), next: 1 }
    }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn next_id(&self) -> usize { self.next }
    pub fn push(&mut self, object: Object) {
        self.entries.insert(self.next, object);
        self.next = self.next.checked_add(1).expect("TJS handle space exhausted");
    }
    pub fn get(&self, id: usize) -> Option<&Object> { self.entries.get(&id) }
}
impl Index<usize> for ObjectArena {
    type Output = Object;
    fn index(&self, id: usize) -> &Object { &self.entries[&id] }
}
impl IndexMut<usize> for ObjectArena {
    fn index_mut(&mut self, id: usize) -> &mut Object {
        self.entries.get_mut(&id).expect("validated TJS object handle")
    }
}

fn references(value: &Value, pending: &mut Vec<usize>) {
    if let Value::Object(reference) = value {
        pending.extend(reference.object);
        pending.extend(reference.context);
    }
}
impl Vm {
    pub fn live_objects(&self) -> usize { self.objects.len() }

    pub fn should_collect_garbage(&self) -> bool { self.allocations_since_gc >= 8192 }

    fn mark_objects(&self, host: &impl Host, roots: &[Value], budget: &mut u64) -> Result<BTreeSet<usize>> {
        let mut pending = vec![0, self.native_array_class];
        for value in self.globals.values().chain(self.literal_objects.values()).chain(roots) {
            references(value, &mut pending);
        }
        for value in host.gc_roots() { references(&value, &mut pending); }
        let mut marked = BTreeSet::new();
        let mut readonly_seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !marked.insert(id) { continue; }
            *budget = budget.checked_sub(1).ok_or_else(|| crate::unsupported("TJS collection budget exceeded"))?;
            let object = self.objects.get(id).ok_or_else(|| anyhow::anyhow!("invalid TJS collection root: {id}"))?;
            pending.extend(object.owner);
            for value in object.members.values() { references(value, &mut pending); }
            match &object.kind {
                ObjectKind::Array(items) => for value in items { references(value, &mut pending); },
                ObjectKind::Property { getter, setter } => {
                    for value in getter.iter().chain(setter) { references(value, &mut pending); }
                }
                ObjectKind::VariantProperty(value) | ObjectKind::Method { receiver: value, .. } => references(value, &mut pending),
                ObjectKind::Super { bases, context } => {
                    pending.extend(bases);
                    pending.push(*context);
                }
                ObjectKind::ReadOnly(view) => {
                    let mut trees = vec![view.data.clone()];
                    while let Some(tree) = trees.pop() {
                        if !readonly_seen.insert(Arc::as_ptr(&tree) as usize) { continue; }
                        *budget = budget.checked_sub(1).ok_or_else(|| crate::unsupported("TJS collection budget exceeded"))?;
                        match tree.as_ref() {
                            ReadOnlyData::Scalar(value) => references(value, &mut pending),
                            ReadOnlyData::Array(items) => trees.extend(items.iter().cloned()),
                            ReadOnlyData::Dictionary(items) => trees.extend(items.values().cloned()),
                        }
                    }
                }
                _ => {}
            }
            for value in host.gc_trace(id) { references(&value, &mut pending); }
        }
        Ok(marked)
    }

    /// Collect unreachable objects while no script frame is executing.
    ///
    /// The caller must include every externally retained Value in `roots`.
    /// The Host must report its strong roots and native outgoing references.
    /// An omitted handle becomes invalid; IDs are never reused. Script/native
    /// finalizers execute before reclamation and can resurrect invalidated
    /// handles, which remain allocated when reachable after finalization.
    pub fn collect_garbage(&mut self, host: &mut impl Host, roots: &[Value], budget: &mut u64) -> Result<usize> {
        ensure!(self.depth == 0 && !self.collecting, "TJS collection requires an idle interpreter");
        self.collecting = true;
        let result = self.collect_idle(host, roots, budget);
        self.collecting = false;
        result
    }

    fn collect_idle(&mut self, host: &mut impl Host, roots: &[Value], budget: &mut u64) -> Result<usize> {
        let marked = self.mark_objects(host, roots, budget)?;
        let dead: Vec<_> = self.objects.entries.keys().filter(|id| !marked.contains(id)).copied().collect();
        // Keep the whole unreachable graph allocated throughout finalization:
        // a finalizer may call another object which is also unreachable.
        for &id in &dead {
            let object = &self.objects[id];
            if object.valid && (matches!(object.kind, ObjectKind::Instance) || !object.native_finalizers.is_empty()) {
                self.invalidate(&Value::object(id), host, budget)?;
            }
        }
        let marked = self.mark_objects(host, roots, budget)?;
        let mut reclaimed = 0;
        for id in dead {
            if !marked.contains(&id) {
                self.objects.entries.remove(&id);
                reclaimed += 1;
            }
        }
        self.allocations_since_gc = 0;
        Ok(reclaimed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, compile_expression};
    fn eval(vm: &mut Vm, source: &str) -> Value {
        vm.execute(&compile_expression("gc", source).unwrap(), &mut (), &mut 10_000_000).unwrap()
    }
    #[test]
    fn reclaims_cycles_preserves_explicit_roots_and_never_reuses_ids() -> Result<()> {
        let mut vm = Vm::default();
        let baseline = vm.live_objects();
        let retained = eval(&mut vm, "[1,2]");
        let stale = eval(&mut vm, "%[x:1]");
        vm.execute(&compile("gc", "for(var i=0;i<2000;i++){var a=[];a.add(a);}")?, &mut (), &mut 1_000_000)?;
        assert!(vm.collect_garbage(&mut (), std::slice::from_ref(&retained), &mut 1_000_000)? >= 2000);
        assert_eq!(vm.get_member(&retained, &Value::Integer(1), false)?, Value::Integer(2));
        assert!(vm.get_member(&stale, &Value::string("x"), false).is_err());
        let fresh = eval(&mut vm, "[]");
        assert_ne!(stale, fresh);
        vm.collect_garbage(&mut (), &[], &mut 1_000_000)?;
        assert_eq!(vm.live_objects(), baseline);
        Ok(())
    }
    #[test]
    fn collection_calls_finalizers_and_traces_native_edges() -> Result<()> {
        struct Native { root: Value, edge: Value }
        impl Host for Native {
            fn call(&mut self, _: &mut Vm, _: &str, _: &[Value], _: &mut u64) -> Result<Value> { unreachable!() }
            fn gc_roots(&self) -> Vec<Value> { vec![self.root.clone()] }
            fn gc_trace(&self, id: usize) -> Vec<Value> {
                if self.root == Value::object(id) { vec![self.edge.clone()] } else { vec![] }
            }
        }
        let mut vm = Vm::default();
        vm.execute(&compile("gc", "var done=0;class C{function finalize(){global.done++;}} function f(){new C();} f();")?, &mut (), &mut 1_000_000)?;
        let mut host = Native { root: vm.allocate(ObjectKind::Namespace)?, edge: eval(&mut vm, "[123]") };
        vm.collect_garbage(&mut host, &[], &mut 1_000_000)?;
        assert_eq!(vm.globals["done"], Value::Integer(1));
        assert_eq!(vm.get_member(&host.edge, &Value::Integer(0), false)?, Value::Integer(123));
        vm.collect_garbage(&mut host, &[], &mut 1_000_000)?;
        assert_eq!(vm.globals["done"], Value::Integer(1));
        Ok(())
    }

    #[test]
    fn traces_bound_contexts_and_readonly_scalar_objects() -> Result<()> {
        use std::sync::atomic::AtomicBool;
        let mut vm = Vm::default();
        let bound = eval(&mut vm, "(function(){return this.answer;}) incontextof %[answer:42]");
        let leaf = eval(&mut vm, "[99]");
        let tree = vm.new_readonly_data(
            Arc::new(ReadOnlyData::Array(vec![Arc::new(ReadOnlyData::Scalar(leaf))])),
            Arc::new(AtomicBool::new(true)),
        )?;
        vm.collect_garbage(&mut (), &[bound.clone(), tree.clone()], &mut 1_000_000)?;
        assert_eq!(vm.invoke(&bound, &Value::NULL, &[], &mut (), &mut 1000)?, Value::Integer(42));
        let leaf = vm.get_member(&tree, &Value::Integer(0), false)?;
        assert_eq!(vm.get_member(&leaf, &Value::Integer(0), false)?, Value::Integer(99));
        Ok(())
    }

    #[test]
    fn retained_finalized_handles_survive_and_finalizers_cannot_reenter_collection() -> Result<()> {
        struct Reenter;
        impl Host for Reenter {
            fn call(&mut self, vm: &mut Vm, _: &str, _: &[Value], budget: &mut u64) -> Result<Value> {
                assert!(vm.collect_garbage(self, &[], budget).is_err());
                Ok(Value::Void)
            }
        }
        let mut vm = Vm::default();
        vm.register_native("reenter")?;
        vm.execute(&compile("gc", "var retained=null;class C{function finalize(){reenter();global.retained=this;}} function f(){new C();}f();")?, &mut Reenter, &mut 1_000_000)?;
        vm.collect_garbage(&mut Reenter, &[], &mut 1_000_000)?;
        let retained = vm.globals["retained"].clone();
        assert_eq!(vm.is_valid(&retained)?, Value::Integer(0));
        vm.collect_garbage(&mut Reenter, &[], &mut 1_000_000)?;
        assert_eq!(vm.is_valid(&retained)?, Value::Integer(0));
        Ok(())
    }
}
