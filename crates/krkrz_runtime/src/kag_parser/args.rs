//! Typed parameter formats. Only extended parameters have a name list.
use super::*;

#[derive(Clone)]
pub(super) struct Core;
#[derive(Clone)]
pub(super) struct Extended {
    list: Value,
}

pub(super) trait ParameterFormat: Clone {
    fn new(vm: &mut Vm) -> Result<Self>;
    fn clear(&self, vm: &mut Vm) -> Result<()>;
    fn record(&self, vm: &mut Vm, name: &str) -> Result<()>;
    fn duplicate(&self, vm: &mut Vm, budget: &mut u64) -> Result<Self>;
    fn result(&self, vm: &mut Vm, dict: &Value) -> Result<()>;
    fn expand(
        vm: &mut Vm,
        source: &Args<Self>,
        target: &mut Args<Self>,
        budget: &mut u64,
    ) -> Result<()>;
    fn entries(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Vec<(String, Value)>>;
    fn store(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Value>;
    fn restore(&self, vm: &mut Vm, dict: &Value, saved: &Value, budget: &mut u64) -> Result<()>;
    fn trace(&self, out: &mut Vec<Value>);
}

impl ParameterFormat for Core {
    fn new(_: &mut Vm) -> Result<Self> {
        Ok(Self)
    }
    fn clear(&self, _: &mut Vm) -> Result<()> {
        Ok(())
    }
    fn record(&self, _: &mut Vm, _: &str) -> Result<()> {
        Ok(())
    }
    fn duplicate(&self, _: &mut Vm, _: &mut u64) -> Result<Self> {
        Ok(Self)
    }
    fn result(&self, _: &mut Vm, _: &Value) -> Result<()> {
        Ok(())
    }
    fn entries(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Vec<(String, Value)>> {
        // Array.assign uses the same native member enumeration as Dictionary.assign.
        let array = vm.new_native_array(Vec::new())?;
        let assign = vm.get_member(&array, &Value::string("assign"), false)?;
        vm.call_function(&assign, &array, std::slice::from_ref(dict), &mut (), budget)?;
        Ok(vm
            .array_items(&array)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0].text(), pair[1].clone()))
            .collect())
    }
    fn store(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Value> {
        let copy = vm.new_dictionary()?;
        vm.dictionary_assign(&copy, std::slice::from_ref(dict), false, budget)?;
        Ok(copy)
    }
    fn restore(&self, vm: &mut Vm, dict: &Value, saved: &Value, budget: &mut u64) -> Result<()> {
        // Dictionary.assign also accepts the older runtime's flat argument arrays.
        if let Ok(items) = vm.array_items(saved) {
            ensure!(
                items.len() <= 100_000 && items.len() % 2 == 0,
                "malformed KAG saved macro arguments"
            );
        }
        vm.dictionary_assign(dict, std::slice::from_ref(saved), false, budget)?;
        Ok(())
    }
    fn trace(&self, _: &mut Vec<Value>) {}
    fn expand(
        vm: &mut Vm,
        source: &Args<Self>,
        target: &mut Args<Self>,
        budget: &mut u64,
    ) -> Result<()> {
        let tag = Value::string("tagname");
        let tagname = vm.get_member(&target.dict, &tag, true)?;
        vm.dictionary_assign(
            &target.dict,
            std::slice::from_ref(&source.dict),
            false,
            budget,
        )?;
        vm.set_member(&target.dict, &tag, tagname)
    }
}

impl ParameterFormat for Extended {
    fn new(vm: &mut Vm) -> Result<Self> {
        Ok(Self {
            list: vm.new_native_array(Vec::new())?,
        })
    }
    fn clear(&self, vm: &mut Vm) -> Result<()> {
        vm.clear_array(&self.list)
    }
    fn record(&self, vm: &mut Vm, name: &str) -> Result<()> {
        let index = vm.array_items(&self.list)?.len();
        vm.set_member(
            &self.list,
            &Value::Integer(index as i64),
            Value::string(name),
        )
    }
    fn duplicate(&self, vm: &mut Vm, budget: &mut u64) -> Result<Self> {
        let items = vm.array_items(&self.list)?.to_vec();
        *budget = budget
            .checked_sub(items.len() as u64)
            .ok_or_else(|| unsupported("KAGParser argument copy budget exhausted"))?;
        Ok(Self {
            list: vm.new_native_array(items)?,
        })
    }
    fn result(&self, vm: &mut Vm, dict: &Value) -> Result<()> {
        vm.set_member(dict, &Value::string("taglist"), self.list.clone())
    }
    fn entries(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Vec<(String, Value)>> {
        let mut entries = Vec::new();
        for name in vm.array_items(&self.list)?.to_vec() {
            charge(budget)?;
            if vm.has_member(dict, &name)? {
                entries.push((name.text(), vm.get_member(dict, &name, true)?));
            }
        }
        Ok(entries)
    }
    fn store(&self, vm: &mut Vm, dict: &Value, budget: &mut u64) -> Result<Value> {
        let entries = self.entries(vm, dict, budget)?;
        vm.new_native_array(
            entries
                .into_iter()
                .flat_map(|(name, value)| [Value::string(&name), value])
                .collect(),
        )
    }
    fn restore(&self, vm: &mut Vm, dict: &Value, saved: &Value, budget: &mut u64) -> Result<()> {
        let flat = state::array(vm, saved, 100_000, budget)?;
        ensure!(flat.len() % 2 == 0, "malformed KAG saved macro arguments");
        for pair in flat.as_chunks::<2>().0 {
            let name = pair[0].text();
            vm.set_member(dict, &Value::string(&name), pair[1].clone())?;
            self.record(vm, &name)?;
        }
        Ok(())
    }
    fn trace(&self, out: &mut Vec<Value>) {
        out.push(self.list.clone());
    }
    fn expand(
        vm: &mut Vm,
        source: &Args<Self>,
        target: &mut Args<Self>,
        budget: &mut u64,
    ) -> Result<()> {
        for (name, value) in source.format.entries(vm, &source.dict, budget)? {
            if name != "tagname" {
                vm.set_member(&target.dict, &Value::string(&name), value)?;
                target.format.record(vm, &name)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Args<P: ParameterFormat> {
    dict: Value,
    format: P,
}
impl<P: ParameterFormat> Args<P> {
    fn new(vm: &mut Vm) -> Result<Self> {
        Ok(Self {
            dict: vm.new_dictionary()?,
            format: P::new(vm)?,
        })
    }
    fn duplicate(&self, vm: &mut Vm, budget: &mut u64) -> Result<Self> {
        let dict = vm.new_dictionary()?;
        vm.dictionary_assign(&dict, std::slice::from_ref(&self.dict), false, budget)?;
        Ok(Self {
            dict,
            format: self.format.duplicate(vm, budget)?,
        })
    }
}

#[derive(Clone)]
pub(super) enum Arguments {
    Core(Args<Core>),
    Extended(Args<Extended>),
}
// Both branches call a monomorphized Args<P> operation.
macro_rules! dispatch {
    ($value:expr, $args:ident => $body:expr) => {
        match $value {
            Arguments::Core($args) => $body,
            Arguments::Extended($args) => $body,
        }
    };
}
impl Arguments {
    pub(super) fn same_format(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
    pub(super) fn new(vm: &mut Vm, extended: bool) -> Result<Self> {
        if extended {
            Ok(Self::Extended(Args::new(vm)?))
        } else {
            Ok(Self::Core(Args::new(vm)?))
        }
    }
    pub(super) fn empty(&self, vm: &mut Vm) -> Result<Self> {
        Self::new(vm, matches!(self, Self::Extended(_)))
    }
    pub(super) fn dict(&self) -> &Value {
        dispatch!(self, args => &args.dict)
    }
    pub(super) fn clear(&mut self, vm: &mut Vm, budget: &mut u64) -> Result<()> {
        vm.dictionary_assign(self.dict(), &[], true, budget)?;
        dispatch!(self, args => args.format.clear(vm))
    }
    pub(super) fn add(&mut self, vm: &mut Vm, name: &str, value: Value) -> Result<()> {
        vm.set_member(self.dict(), &Value::string(name), value)?;
        dispatch!(self, args => args.format.record(vm, name))
    }
    pub(super) fn get(&self, vm: &mut Vm, name: &str) -> Result<Value> {
        vm.get_member(self.dict(), &Value::string(name), true)
    }
    pub(super) fn result(&self, vm: &mut Vm) -> Result<Value> {
        dispatch!(self, args => args.format.result(vm, &args.dict))?;
        Ok(self.dict().clone())
    }
    pub(super) fn duplicate(&self, vm: &mut Vm, budget: &mut u64) -> Result<Self> {
        match self {
            Self::Core(args) => Ok(Self::Core(args.duplicate(vm, budget)?)),
            Self::Extended(args) => Ok(Self::Extended(args.duplicate(vm, budget)?)),
        }
    }
    pub(super) fn entries(&self, vm: &mut Vm, budget: &mut u64) -> Result<Vec<(String, Value)>> {
        dispatch!(self, args => args.format.entries(vm, &args.dict, budget))
    }
    pub(super) fn expand_into(
        &self,
        vm: &mut Vm,
        target: &mut Self,
        budget: &mut u64,
    ) -> Result<()> {
        match (self, target) {
            (Self::Core(source), Self::Core(target)) => Core::expand(vm, source, target, budget),
            (Self::Extended(source), Self::Extended(target)) => {
                Extended::expand(vm, source, target, budget)
            }
            _ => Err(unsupported("KAGParser parameter formats do not match")),
        }
    }
    pub(super) fn store(&self, vm: &mut Vm, budget: &mut u64) -> Result<Value> {
        dispatch!(self, args => args.format.store(vm, &args.dict, budget))
    }
    pub(super) fn restore(&self, vm: &mut Vm, saved: &Value, budget: &mut u64) -> Result<Self> {
        let args = self.empty(vm)?;
        dispatch!(&args, value => value.format.restore(vm, &value.dict, saved, budget))?;
        Ok(args)
    }
    pub(super) fn trace(&self, out: &mut Vec<Value>) {
        out.push(self.dict().clone());
        dispatch!(self, args => args.format.trace(out));
    }
}
