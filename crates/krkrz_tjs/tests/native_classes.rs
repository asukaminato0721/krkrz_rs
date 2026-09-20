use anyhow::{Result, bail};
use krkrz_tjs::{Host, Value, Vm, compile};
use std::collections::BTreeMap;

#[derive(Default)]
struct NativeHost {
    instances: BTreeMap<usize, i64>,
}
impl Host for NativeHost {
    fn call(&mut self, _: &mut Vm, name: &str, _: &[Value], _: &mut u64) -> Result<Value> {
        bail!("receiver required for {name}")
    }
    fn call_with_context(
        &mut self,
        _: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        _: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            bail!("object context required")
        };
        let id = reference.object.unwrap();
        if name == "Native.@initialize" {
            self.instances.insert(id, 2);
            return Ok(Value::Void);
        }
        let state = self
            .instances
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("missing instance"))?;
        match name {
            "Native.Native" | "Native.setValue" => *state = args[0].integer()?,
            "Native.getValue" | "Native.read" => return Ok(Value::Integer(*state)),
            _ => bail!("unexpected native call: {name}"),
        }
        Ok(Value::Void)
    }
}

#[test]
fn native_instances_inherit_and_bind_properties_and_methods() {
    let mut vm = Vm::default();
    let class = vm.register_native_class("Native").unwrap();
    vm.register_native("Native.Native").unwrap();
    vm.register_native("Native.read").unwrap();
    vm.register_native_property(
        &class,
        "value",
        Some("Native.getValue"),
        Some("Native.setValue"),
    )
    .unwrap();
    let source = r#"
        class Derived extends Native {
            var initial=value;
            function Derived(n) { super.Native(n); }
        }
        var a=new Derived(7), b=new Derived(9);
        var f=a.read;
        a.value+=1;
        return a.initial*10000 + f()*1000 + b.value*100 +
               *(&Native.value incontextof a)*10 + (a instanceof 'Native');
    "#;
    let mut host = NativeHost::default();
    let result = vm
        .execute(&compile("native", source).unwrap(), &mut host, &mut 10_000)
        .unwrap();
    assert_eq!(result, Value::Integer(28981));
    assert_eq!(host.instances.len(), 2);
}
