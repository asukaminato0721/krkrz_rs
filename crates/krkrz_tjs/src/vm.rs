use crate::object::{Object, ObjectKind, Thrown};
use crate::{Class, Function, ObjectRef, Property, Value, VmAbort, unsupported};
use anyhow::{Context, Result, bail, ensure};
use krkrz_core::SourceLocation;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Argument {
    Value(usize),
    Spread(usize),
    Forward,
    ForwardFrom(usize),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Parameter {
        name: String,
        index: usize,
        rest: bool,
    },
    ConstantObject {
        out: usize,
        identity: u64,
        literal: crate::Literal,
    },
    DeleteName {
        out: usize,
        name: String,
    },
    DeleteMember {
        out: usize,
        object: usize,
        key: usize,
    },
    Constant {
        out: usize,
        value: Value,
    },
    Load {
        out: usize,
        name: String,
        optional: bool,
        raw: bool,
    },
    Store {
        name: String,
        input: usize,
        raw: bool,
    },
    Declare {
        name: String,
        input: usize,
    },
    Move {
        out: usize,
        input: usize,
    },
    Unary {
        out: usize,
        op: String,
        input: usize,
    },
    Binary {
        out: usize,
        op: String,
        left: usize,
        right: usize,
    },
    Get {
        out: usize,
        object: usize,
        key: usize,
        optional: bool,
        raw: bool,
    },
    TypeOfMember {
        out: usize,
        object: usize,
        key: usize,
    },
    Set {
        object: usize,
        key: usize,
        input: usize,
        raw: bool,
    },
    Array {
        out: usize,
        items: Vec<usize>,
    },
    Dictionary {
        out: usize,
        items: Vec<(usize, usize)>,
    },
    Function {
        out: usize,
        function: Box<Function>,
    },
    Call {
        out: usize,
        callee: usize,
        context: Option<usize>,
        args: Vec<Argument>,
        result_needed: bool,
    },
    Construct {
        out: usize,
        callee: usize,
        args: Vec<Argument>,
    },
    Class {
        out: usize,
        definition: Box<Class>,
    },
    Property {
        out: usize,
        definition: Box<Property>,
    },
    ReadProperty {
        out: usize,
        input: usize,
    },
    WriteProperty {
        property: usize,
        input: usize,
    },
    Eval {
        out: usize,
        input: usize,
        result_needed: bool,
    },
    Jump {
        target: usize,
    },
    JumpUnless {
        input: usize,
        target: usize,
    },
    EnterScope,
    LeaveScope,
    EnterWith {
        input: usize,
    },
    LeaveWith,
    With {
        out: usize,
    },
    Try {
        target: usize,
        exception: usize,
    },
    EndTry,
    Unwind {
        scopes: usize,
        handlers: usize,
        withs: usize,
    },
    Throw {
        input: usize,
    },
    Return {
        input: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instruction {
    pub location: SourceLocation,
    pub op: Op,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Program {
    pub storage: String,
    pub registers: usize,
    pub code: Vec<Instruction>,
}
impl Program {
    pub fn validate(&self) -> Result<()> {
        self.validate_depth(0)
    }
    fn validate_depth(&self, depth: usize) -> Result<()> {
        ensure!(
            depth < 128 && self.registers <= 1_000_000 && self.code.len() <= 1_000_000,
            "TJS program exceeds limit"
        );
        for i in &self.code {
            let mut regs = vec![];
            match &i.op {
                Op::Parameter { index, .. } => {
                    ensure!(*index <= 1024, "invalid TJS parameter index")
                }
                Op::Constant { out, value } => {
                    ensure!(
                        !matches!(value, Value::Object(o) if o.object.is_some() || o.context.is_some()),
                        "live object handle in TJS constant pool"
                    );
                    regs.push(*out);
                }
                Op::Load { out, .. } | Op::DeleteName { out, .. } | Op::With { out } => {
                    regs.push(*out)
                }
                Op::Store { input, .. }
                | Op::Declare { input, .. }
                | Op::Return { input }
                | Op::Throw { input }
                | Op::EnterWith { input } => regs.push(*input),
                Op::ReadProperty { out, input }
                | Op::Move { out, input }
                | Op::Unary { out, input, .. }
                | Op::Eval { out, input, .. } => regs.extend([*out, *input]),
                Op::Binary {
                    out, left, right, ..
                } => regs.extend([*out, *left, *right]),
                Op::Get {
                    out, object, key, ..
                }
                | Op::TypeOfMember { out, object, key }
                | Op::DeleteMember { out, object, key } => regs.extend([*out, *object, *key]),
                Op::Set {
                    object, key, input, ..
                } => regs.extend([*object, *key, *input]),
                Op::ConstantObject { out, literal, .. } => {
                    regs.push(*out);
                    literal.validate(0)?;
                }
                Op::Array { out, items } => {
                    regs.push(*out);
                    regs.extend(items);
                }
                Op::Dictionary { out, items } => {
                    regs.push(*out);
                    for (key, value) in items {
                        regs.extend([*key, *value]);
                    }
                }
                Op::Function { out, function } => {
                    regs.push(*out);
                    ensure!(function.parameters.len() <= 1024, "too many TJS parameters");
                    function.program.validate_depth(depth + 1)?;
                }
                Op::WriteProperty { property, input } => regs.extend([*property, *input]),
                Op::Property { out, definition } => {
                    regs.push(*out);
                    for f in [&definition.getter, &definition.setter]
                        .into_iter()
                        .flatten()
                    {
                        f.program.validate_depth(depth + 1)?;
                    }
                }
                Op::Class { out, definition } => {
                    regs.push(*out);
                    for base in &definition.bases {
                        base.validate_depth(depth + 1)?;
                    }
                    for f in &definition.methods {
                        f.program.validate_depth(depth + 1)?;
                    }
                    if let Some(p) = &definition.initializer {
                        p.validate_depth(depth + 1)?;
                    }
                    for p in &definition.properties {
                        for f in [&p.getter, &p.setter].into_iter().flatten() {
                            f.program.validate_depth(depth + 1)?;
                        }
                    }
                }
                Op::Construct { out, callee, args } => {
                    regs.extend([*out, *callee]);
                    for arg in args {
                        if let Argument::Value(r) | Argument::Spread(r) = arg {
                            regs.push(*r);
                        }
                    }
                }
                Op::Call {
                    out,
                    callee,
                    context,
                    args,
                    ..
                } => {
                    ensure!(args.len() <= 1024, "too many TJS arguments");
                    regs.extend([*out, *callee]);
                    regs.extend(context);
                    for arg in args {
                        if let Argument::Value(r) | Argument::Spread(r) = arg {
                            regs.push(*r);
                        }
                    }
                }
                Op::JumpUnless { input, target } => {
                    regs.push(*input);
                    ensure!(*target <= self.code.len(), "invalid TJS branch target");
                }
                Op::Jump { target } => {
                    ensure!(*target <= self.code.len(), "invalid TJS branch target")
                }
                Op::Try { target, exception } => {
                    regs.push(*exception);
                    ensure!(*target < self.code.len(), "invalid TJS handler target");
                }
                _ => (),
            }
            ensure!(
                regs.into_iter().all(|r| r < self.registers),
                "invalid TJS register operand"
            );
        }
        Ok(())
    }
}
pub trait Host {
    fn call(&mut self, vm: &mut Vm, name: &str, args: &[Value], budget: &mut u64) -> Result<Value>;
    /// Called with the resolved receiver, including an explicitly bound context.
    fn call_with_context(
        &mut self,
        vm: &mut Vm,
        name: &str,
        _context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        self.call(vm, name, args, budget)
    }
    /// Native calls can observe whether the VM supplied a result destination.
    #[allow(clippy::too_many_arguments)]
    fn call_with_result(
        &mut self,
        vm: &mut Vm,
        name: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
        _result_needed: bool,
    ) -> Result<Value> {
        self.call_with_context(vm, name, context, args, budget)
    }
    fn trace(&mut self, _instruction: &Instruction) -> Result<()> {
        Ok(())
    }
}
impl Host for () {
    fn call(&mut self, _: &mut Vm, name: &str, _: &[Value], _: &mut u64) -> Result<Value> {
        Err(unsupported(format!("unsupported native call: {name}")))
    }
}
pub struct Vm {
    pub preprocessor: crate::Preprocessor,
    pub globals: BTreeMap<String, Value>,
    pub executed: u64,
    pub(crate) objects: Vec<Object>,
    pub(crate) hash_generation: u64,
    literal_objects: BTreeMap<u64, Value>,
    native_array_class: usize,
    depth: usize,
}
impl Default for Vm {
    fn default() -> Self {
        let mut vm = Self {
            preprocessor: crate::Preprocessor::default(),
            globals: BTreeMap::new(),
            executed: 0,
            objects: vec![Object::new(ObjectKind::Global)],
            hash_generation: 0,
            literal_objects: BTreeMap::new(),
            native_array_class: 0,
            depth: 0,
        };
        for name in ["Array", "Dictionary", "Exception", "RegExp"] {
            vm.register_native(name).expect("initial TJS heap");
        }
        vm.register_serialization(false)
            .expect("initial serialization members");
        let array = vm.globals["Array"].clone();
        vm.register_native_method(&array, "assign", "Array.assign")
            .expect("initial Array.assign member");
        let array_id = vm.object_id(&array).expect("Array class");
        vm.objects[array_id].member_flags.insert("assign".encode_utf16().collect(), crate::scripts_ex::HIDDEN);
        let dictionary = vm.globals["Dictionary"].clone();
        for method in ["assign", "clear"] {
            vm.register_native_static_method(&dictionary, method, &format!("Dictionary.{method}")).expect("initial Dictionary members");
            let function = vm.get_member(&dictionary, &Value::string(method), false).expect("Dictionary method");
            let id = vm.object_id(&function).expect("Dictionary method object");
            vm.objects[id].native_static = true;
        }
        let id = vm.object_id(&array).expect("Array class");
        vm.native_array_class = vm.objects.len();
        vm.objects.push(vm.objects[id].clone());
        vm
    }
}
struct Handler {
    target: usize,
    exception: usize,
    scopes: usize,
    withs: usize,
}
pub(crate) struct Frame {
    scopes: Vec<BTreeMap<String, Value>>,
    handlers: Vec<Handler>,
    withs: Vec<Value>,
    pub(crate) context: usize,
    owner: Option<usize>,
    arguments: Vec<Value>,
}
impl Frame {
    pub(crate) fn global() -> Self {
        Self {
            scopes: vec![],
            handlers: vec![],
            withs: vec![],
            context: 0,
            owner: None,
            arguments: vec![],
        }
    }
}
impl Vm {
    pub(crate) fn allocate(&mut self, kind: ObjectKind) -> Result<Value> {
        if self.objects.len() >= 100_000 {
            return Err(unsupported("TJS object allocation limit exceeded"));
        }
        let id = self.objects.len();
        let bound = matches!(
            kind,
            ObjectKind::Array(_)
                | ObjectKind::Dictionary
                | ObjectKind::ReadOnly(_)
                | ObjectKind::Instance
                | ObjectKind::RegExp { .. }
        );
        self.objects.push(Object::new(kind));
        self.objects[id].hash_generation = self.hash_generation;
        let mut value = Value::object(id);
        if bound && let Value::Object(reference) = &mut value {
            reference.context = Some(id);
        }
        if matches!(self.objects[id].kind, ObjectKind::Array(_)) {
            let class = self
                .globals
                .get("Array")
                .cloned()
                .context("Array class is missing")?;
            let class_id = self.object_id(&class)?;
            for (key, mut member) in self.objects[class_id].members.clone() {
                let flags = self.objects[class_id]
                    .member_flags
                    .get(&key)
                    .copied()
                    .unwrap_or(0);
                if flags & crate::scripts_ex::STATIC != 0 {
                    continue;
                }
                if let Value::Object(reference) = &mut member {
                    reference.context = Some(id);
                }
                self.set_member(&value, &Value::String(key.clone()), member)?;
                self.objects[id].member_flags.insert(key, flags);
            }
        }
        Ok(value)
    }
    pub(crate) fn object_id(&self, value: &Value) -> Result<usize> {
        let id = self.object_handle(value)?;
        ensure!(self.objects[id].valid, "TJS object is invalid");
        Ok(id)
    }
    // Validity checks themselves must accept handles to invalidated objects.
    pub(crate) fn object_handle(&self, value: &Value) -> Result<usize> {
        let Value::Object(ObjectRef {
            object: Some(id), ..
        }) = value
        else {
            bail!("TJS value is not a non-null object");
        };
        ensure!(*id < self.objects.len(), "invalid TJS object handle");
        Ok(*id)
    }
    pub fn register_namespace(&mut self, name: &str) -> Result<Value> {
        if let Some(value) = self.globals.get(name) {
            return Ok(value.clone());
        }
        let value = self.allocate(ObjectKind::Namespace)?;
        self.globals.insert(name.into(), value.clone());
        Ok(value)
    }
    pub fn new_array(&mut self, items: Vec<Value>) -> Result<Value> {
        self.allocate(ObjectKind::Array(items))
    }
    /// TVPCreateArrayObject uses the engine's private builtin class, not the
    /// script-visible Array class that plugins can extend.
    pub fn new_native_array(&mut self, items: Vec<Value>) -> Result<Value> {
        let class = self
            .globals
            .insert("Array".into(), Value::object(self.native_array_class));
        let result = self.new_array(items);
        if let Some(class) = class {
            self.globals.insert("Array".into(), class);
        } else {
            self.globals.remove("Array");
        }
        result
    }
    pub fn new_dictionary(&mut self) -> Result<Value> {
        self.allocate(ObjectKind::Dictionary)
    }
    pub fn register_native(&mut self, name: &str) -> Result<()> {
        let value = self.allocate(ObjectKind::Native(name.into()))?;
        if let Some((namespace, member)) = name.split_once('.') {
            ensure!(
                !member.contains('.'),
                "native registration supports one namespace component"
            );
            let receiver = self.register_namespace(namespace)?;
            self.set_member(&receiver, &Value::string(member), value)?;
        } else {
            self.globals.insert(name.into(), value);
        }
        Ok(())
    }
    /// Native classes participate in ordinary TJS inheritance and property dispatch.
    /// The host receives `NAME.@initialize` before the derived class's field initializers.
    pub fn register_native_class(&mut self, name: &str) -> Result<Value> {
        ensure!(
            !self.globals.contains_key(name),
            "class is already registered: {name}"
        );
        let value = self.new_native_class(name, &format!("{name}.@initialize"))?;
        self.globals.insert(name.into(), value.clone());
        Ok(value)
    }
    /// Create a native class without adding a global name, for nested plugin classes.
    pub fn new_native_class(&mut self, name: &str, initializer: &str) -> Result<Value> {
        ensure!(
            initializer.ends_with(".@initialize"),
            "invalid native initializer name"
        );
        let value = self.define_class(&crate::Class {
            name: name.into(),
            bases: vec![],
            methods: vec![],
            properties: vec![],
            initializer: None,
        })?;
        let id = self.object_id(&value)?;
        if let ObjectKind::Class {
            native_initializer, ..
        } = &mut self.objects[id].kind
        {
            *native_initializer = Some(initializer.into());
        }
        Ok(value)
    }
    /// Install a class member that is not copied into constructed instances.
    pub fn register_native_static_value(
        &mut self,
        receiver: &Value,
        member: &str,
        value: Value,
    ) -> Result<()> {
        self.set_member(receiver, &Value::string(member), value)?;
        let id = self.object_id(receiver)?;
        self.objects[id]
            .member_flags
            .insert(member.encode_utf16().collect(), crate::scripts_ex::STATIC);
        Ok(())
    }
    /// Attach an explicitly named host operation to an object or nested class.
    pub fn register_native_method(
        &mut self,
        receiver: &Value,
        member: &str,
        operation: &str,
    ) -> Result<()> {
        let value = self.allocate(ObjectKind::Native(operation.into()))?;
        self.set_member(receiver, &Value::string(member), value)
    }
    /// Install an accessor descriptor. Host operation names are explicit to avoid
    /// confusing a method and a property with the same script-visible name.
    pub fn register_native_property(
        &mut self,
        receiver: &Value,
        name: &str,
        getter: Option<&str>,
        setter: Option<&str>,
    ) -> Result<()> {
        let mut accessor = |name: Option<&str>| -> Result<Option<Value>> {
            name.map(|name| self.allocate(ObjectKind::Native(name.into())))
                .transpose()
        };
        let getter = accessor(getter)?;
        let setter = accessor(setter)?;
        let property = self.allocate(ObjectKind::Property { getter, setter })?;
        self.set_member(receiver, &Value::string(name), property)
    }
    /// Register a class method that is not copied into instances.
    pub fn register_native_static_method(
        &mut self,
        receiver: &Value,
        name: &str,
        operation: &str,
    ) -> Result<()> {
        self.register_native_method(receiver, name, operation)?;
        let id = self.object_id(receiver)?;
        self.objects[id]
            .member_flags
            .insert(name.encode_utf16().collect(), crate::scripts_ex::STATIC);
        Ok(())
    }
    /// Register a class property that is not copied into instances.
    pub fn register_native_static_property(
        &mut self,
        receiver: &Value,
        name: &str,
        getter: Option<&str>,
        setter: Option<&str>,
    ) -> Result<()> {
        self.register_native_property(receiver, name, getter, setter)?;
        let id = self.object_id(receiver)?;
        self.objects[id]
            .member_flags
            .insert(name.encode_utf16().collect(), crate::scripts_ex::STATIC);
        Ok(())
    }
    /// Execute a top-level script in the supplied object context (Scripts.exec).
    pub fn execute_in_context(
        &mut self,
        program: &Program,
        context: &Value,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        let mut frame = Frame::global();
        frame.context = self.object_id(context)?;
        self.run(program, host, budget, frame)
    }
    /// Invoke a script callback from a native service using the same budget.
    pub fn call_function(
        &mut self,
        function: &Value,
        context: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        self.invoke(function, context, args, host, budget)
    }
    pub fn execute(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        self.run(program, host, budget, Frame::global())
    }
    pub(crate) fn run(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        budget: &mut u64,
        frame: Frame,
    ) -> Result<Value> {
        program
            .validate()
            .map_err(|e| unsupported(format!("invalid TJS program: {e}")))?;
        if self.depth >= 128 {
            return Err(unsupported("TJS call stack depth exceeded"));
        }
        self.depth += 1;
        let result = self.run_inner(program, host, budget, frame);
        self.depth -= 1;
        result
    }
    fn load_name(
        &mut self,
        frame: &Frame,
        name: &str,
        optional: bool,
        raw: bool,
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        if name == "global" {
            return Ok(Value::object(0));
        }
        if name == "this" {
            return Ok(Value::Object(ObjectRef {
                object: Some(frame.context),
                context: Some(frame.context),
            }));
        }
        if name == "super" {
            let owner = frame.owner.context("super outside class method")?;
            let ObjectKind::Class { definition, .. } = &self.objects[owner].kind else {
                bail!("invalid super owner")
            };
            let base = definition
                .bases
                .first()
                .context("class has no super expression")?
                .clone();
            let base = self.run(&base, host, budget, Frame::global())?;
            let base = self.object_id(&base)?;
            return self.allocate(ObjectKind::Super {
                bases: vec![base],
                context: frame.context,
            });
        }
        for scope in frame.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Ok(value.clone());
            }
        }
        if frame.context != 0 {
            self.object_id(&Value::object(frame.context))?;
            let key: Vec<u16> = name.encode_utf16().collect();
            if self.objects[frame.context].members.contains_key(&key) {
                return self.get_property(
                    &Value::object(frame.context),
                    &Value::string(name),
                    optional,
                    raw,
                    host,
                    budget,
                );
            }
        }
        if self.globals.contains_key(name) {
            return self.get_property(
                &Value::object(0),
                &Value::string(name),
                optional,
                raw,
                host,
                budget,
            );
        }
        if optional {
            Ok(Value::Void)
        } else {
            bail!("member not found: {name}")
        }
    }
    fn store_name(
        &mut self,
        frame: &mut Frame,
        name: &str,
        value: Value,
        declare: bool,
    ) -> Result<()> {
        if declare {
            if let Some(scope) = frame.scopes.last_mut() {
                scope.insert(name.into(), value);
            } else {
                self.set_member(&Value::object(frame.context), &Value::string(name), value)?;
            }
            return Ok(());
        }
        for scope in frame.scopes.iter_mut().rev() {
            if let Some(member) = scope.get_mut(name) {
                *member = value;
                return Ok(());
            }
        }
        if frame.context != 0 {
            let key: Vec<u16> = name.encode_utf16().collect();
            if let Some(member) = self.objects[frame.context].members.get_mut(&key) {
                *member = value;
                return Ok(());
            }
        }
        self.globals.insert(name.into(), value);
        Ok(())
    }
    fn run_inner(
        &mut self,
        program: &Program,
        host: &mut impl Host,
        budget: &mut u64,
        mut frame: Frame,
    ) -> Result<Value> {
        let mut registers = vec![Value::Void; program.registers];
        let mut ip = 0;
        while let Some(instruction) = program.code.get(ip) {
            let at = &instruction.location;
            if *budget == 0 {
                return Err(unsupported(format!(
                    "{}:{}:{}: execution budget exhausted after {} instructions",
                    at.storage, at.line, at.column, self.executed
                )));
            }
            *budget -= 1;
            self.executed += 1;
            host.trace(instruction)
                .map_err(|e| unsupported(format!("trace failed: {e:#}")))?;
            let executing = ip;
            ip += 1;
            let result = (|| -> Result<Option<Value>> {
                match &instruction.op {
                    Op::DeleteName { out, name } => {
                        if frame.scopes.iter().any(|s| s.contains_key(name)) {
                            return Err(unsupported(
                                "deleting a lexical local requires compile-time binding removal",
                            ));
                        }
                        let context = if self
                            .has_member(&Value::object(frame.context), &Value::string(name))?
                        {
                            frame.context
                        } else {
                            0
                        };
                        registers[*out] =
                            self.delete_member(&Value::object(context), &Value::string(name))?;
                    }
                    Op::DeleteMember { out, object, key } => {
                        registers[*out] =
                            self.delete_member(&registers[*object], &registers[*key])?
                    }
                    Op::Constant { out, value } => registers[*out] = value.clone(),
                    Op::Parameter { name, index, rest } => {
                        ensure!(
                            !frame.scopes.is_empty(),
                            "parameter binding outside function"
                        );
                        let value = if *rest {
                            self.new_array(
                                frame.arguments.get(*index..).unwrap_or_default().to_vec(),
                            )?
                        } else {
                            frame.arguments.get(*index).cloned().unwrap_or(Value::Void)
                        };
                        self.store_name(&mut frame, name, value, true)?;
                    }
                    Op::Load {
                        out,
                        name,
                        optional,
                        raw,
                    } => {
                        registers[*out] =
                            self.load_name(&frame, name, *optional, *raw, host, budget)?
                    }
                    Op::Declare { name, input } => {
                        self.store_name(&mut frame, name, registers[*input].clone(), true)?
                    }
                    Op::Store { name, input, raw } => {
                        let local = frame.scopes.iter().any(|s| s.contains_key(name));
                        if local {
                            self.store_name(&mut frame, name, registers[*input].clone(), false)?;
                        } else {
                            self.object_id(&Value::object(frame.context))?;
                            let context = if self.objects[frame.context]
                                .members
                                .contains_key(&name.encode_utf16().collect::<Vec<_>>())
                            {
                                frame.context
                            } else {
                                0
                            };
                            self.set_property(
                                &Value::object(context),
                                &Value::string(name),
                                registers[*input].clone(),
                                *raw,
                                host,
                                budget,
                            )?;
                        }
                    }
                    Op::Move { out, input } => registers[*out] = registers[*input].clone(),
                    Op::Unary { out, op, input } => {
                        registers[*out] = match op.as_str() {
                            "isvalid" => self.is_valid(&registers[*input])?,
                            "invalidate" => self.invalidate(&registers[*input], host, budget)?,
                            _ => registers[*input].unary(op)?,
                        }
                    }
                    Op::Binary {
                        out,
                        op,
                        left,
                        right,
                    } => {
                        registers[*out] = if op == "incontextof" {
                            let Value::Object(mut object) = registers[*left] else {
                                bail!("incontextof requires an object");
                            };
                            let Value::Object(context) = registers[*right] else {
                                bail!("incontextof context must be an object");
                            };
                            if context.object.is_some() {
                                self.object_handle(&registers[*right])?;
                            }
                            object.context = context.object;
                            Value::Object(object)
                        } else if op == "instanceof" {
                            self.instance_of(&registers[*left], &registers[*right].text())?
                        } else {
                            registers[*left].binary(op, &registers[*right])?
                        };
                    }
                    Op::Get {
                        out,
                        object,
                        key,
                        optional,
                        raw,
                    } => {
                        registers[*out] = self.get_property(
                            &registers[*object],
                            &registers[*key],
                            *optional,
                            *raw,
                            host,
                            budget,
                        )?
                    }
                    Op::TypeOfMember { out, object, key } => {
                        let value = self.get_property(
                            &registers[*object],
                            &registers[*key],
                            true,
                            false,
                            host,
                            budget,
                        )?;
                        registers[*out] = if matches!(value, Value::Void)
                            && !self.has_member(&registers[*object], &registers[*key])?
                        {
                            Value::string("undefined")
                        } else {
                            value.unary("typeof")?
                        };
                    }
                    Op::Set {
                        object,
                        key,
                        input,
                        raw,
                    } => self.set_property(
                        &registers[*object],
                        &registers[*key],
                        registers[*input].clone(),
                        *raw,
                        host,
                        budget,
                    )?,
                    Op::ConstantObject {
                        out,
                        identity,
                        literal,
                    } => {
                        let value = if let Some(value) = self.literal_objects.get(identity) {
                            value.clone()
                        } else {
                            let value = self.materialize_literal(literal, budget)?;
                            self.literal_objects.insert(*identity, value.clone());
                            value
                        };
                        registers[*out] = value;
                    }
                    Op::Array { out, items } => {
                        registers[*out] = self.allocate(ObjectKind::Array(
                            items.iter().map(|i| registers[*i].clone()).collect(),
                        ))?
                    }
                    Op::Dictionary { out, items } => {
                        let value = self.allocate(ObjectKind::Dictionary)?;
                        for (key, input) in items {
                            self.set_member(&value, &registers[*key], registers[*input].clone())?;
                        }
                        registers[*out] = value;
                    }
                    Op::Function { out, function } => {
                        registers[*out] =
                            self.allocate(ObjectKind::Function(Arc::new(*function.clone())))?
                    }
                    Op::Call {
                        out,
                        callee,
                        context,
                        args,
                        result_needed,
                    } => {
                        let static_call = self
                            .object_id(&registers[*callee])
                            .ok()
                            .is_some_and(|id| self.objects[id].native_static);
                        let context = context
                            .as_ref()
                            .filter(|_| !static_call)
                            .map(|r| registers[*r].clone())
                            .unwrap_or(Value::object(frame.context));
                        let args = self.expand_args(args, &registers, &frame.arguments)?;
                        registers[*out] = self.invoke_result(
                            &registers[*callee],
                            &context,
                            &args,
                            host,
                            budget,
                            *result_needed,
                        )?;
                    }
                    Op::Construct { out, callee, args } => {
                        let args = self.expand_args(args, &registers, &frame.arguments)?;
                        registers[*out] =
                            self.construct(&registers[*callee], &args, host, budget)?;
                    }
                    Op::Class { out, definition } => {
                        registers[*out] = self.define_class(definition)?;
                    }
                    Op::Property { out, definition } => {
                        registers[*out] = self.define_property(definition, None)?
                    }
                    Op::ReadProperty { out, input } => {
                        registers[*out] = self.read_property(
                            &registers[*input],
                            &Value::object(frame.context),
                            host,
                            budget,
                        )?
                    }
                    Op::WriteProperty { property, input } => {
                        self.write_property(
                            &registers[*property],
                            &Value::object(frame.context),
                            registers[*input].clone(),
                            host,
                            budget,
                        )?;
                    }
                    Op::Eval {
                        out,
                        input,
                        result_needed,
                    } => {
                        let p = crate::compile_with_preprocessor(
                            "<eval operator>",
                            &registers[*input].text(),
                            *result_needed,
                            &mut self.preprocessor,
                        )?;
                        // TJS eval sees the current object context, not local lexical variables.
                        let mut eval = Frame::global();
                        eval.context = frame.context;
                        registers[*out] = self.run(&p, host, budget, eval)?;
                    }
                    Op::Jump { target } => ip = *target,
                    Op::JumpUnless { input, target } => {
                        if !registers[*input].truth()? {
                            ip = *target;
                        }
                    }
                    Op::EnterScope => frame.scopes.push(BTreeMap::new()),
                    Op::LeaveScope => {
                        frame.scopes.pop().context("TJS scope stack underflow")?;
                    }
                    Op::EnterWith { input } => {
                        self.object_id(&registers[*input])?;
                        frame.withs.push(registers[*input].clone());
                    }
                    Op::LeaveWith => {
                        frame.withs.pop().context("TJS with stack underflow")?;
                    }
                    Op::With { out } => {
                        registers[*out] = frame
                            .withs
                            .last()
                            .context("dot member outside with")?
                            .clone()
                    }
                    Op::Try { target, exception } => frame.handlers.push(Handler {
                        target: *target,
                        exception: *exception,
                        scopes: frame.scopes.len(),
                        withs: frame.withs.len(),
                    }),
                    Op::EndTry => {
                        frame
                            .handlers
                            .pop()
                            .context("TJS handler stack underflow")?;
                    }
                    Op::Unwind {
                        scopes,
                        handlers,
                        withs,
                    } => {
                        ensure!(
                            *scopes <= frame.scopes.len()
                                && *handlers <= frame.handlers.len()
                                && *withs <= frame.withs.len(),
                            "invalid TJS unwind depth"
                        );
                        frame.scopes.truncate(*scopes);
                        frame.handlers.truncate(*handlers);
                        frame.withs.truncate(*withs);
                    }
                    Op::Throw { input } => {
                        return Err(anyhow::Error::new(Thrown {
                            value: registers[*input].clone(),
                            message: format!("TJS throw: {}", registers[*input].text()),
                        }));
                    }
                    Op::Return { input } => return Ok(Some(registers[*input].clone())),
                }
                Ok(None)
            })();
            match result {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => (),
                Err(error) => {
                    let error = error.context(format!(
                        "{}:{}:{} at VM instruction {executing}",
                        at.storage, at.line, at.column
                    ));
                    if error.downcast_ref::<VmAbort>().is_some() {
                        return Err(error);
                    }
                    if let Some(handler) = frame.handlers.pop() {
                        registers[handler.exception] =
                            if let Some(thrown) = error.downcast_ref::<Thrown>() {
                                thrown.value.clone()
                            } else {
                                let exception =
                                    self.exception_object(&error.root_cause().to_string())?;
                                self.set_member(
                                    &exception,
                                    &Value::string("trace"),
                                    Value::string(&format!("{error:#}")),
                                )?;
                                exception
                            };
                        frame.scopes.truncate(handler.scopes);
                        frame.withs.truncate(handler.withs);
                        ip = handler.target;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        Ok(Value::Void)
    }
    fn expand_args(
        &self,
        args: &[Argument],
        registers: &[Value],
        forwarded: &[Value],
    ) -> Result<Vec<Value>> {
        let mut values = vec![];
        for arg in args {
            let expanded: &[Value] = match arg {
                Argument::Value(r) => std::slice::from_ref(&registers[*r]),
                Argument::Forward => forwarded,
                Argument::ForwardFrom(start) => forwarded.get(*start..).unwrap_or_default(),
                Argument::Spread(r) => {
                    let id = self.object_id(&registers[*r])?;
                    let ObjectKind::Array(items) = &self.objects[id].kind else {
                        bail!("argument expansion requires Array");
                    };
                    items
                }
            };
            if values.len() + expanded.len() > 1024 {
                return Err(unsupported("too many expanded TJS arguments"));
            }
            values.extend_from_slice(expanded);
        }
        Ok(values)
    }
    pub(crate) fn invoke(
        &mut self,
        callee: &Value,
        context: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
    ) -> Result<Value> {
        self.invoke_result(callee, context, args, host, budget, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn invoke_result(
        &mut self,
        callee: &Value,
        context: &Value,
        args: &[Value],
        host: &mut impl Host,
        budget: &mut u64,
        result_needed: bool,
    ) -> Result<Value> {
        let id = self.object_id(callee)?;
        let Value::Object(reference) = callee else {
            unreachable!()
        };
        let kind = self.objects[id].kind.clone();
        match kind {
            ObjectKind::Native(name) => match name.as_str() {
                "Array.assign" => {
                    let context = reference.context.map(Value::object).unwrap_or_else(|| context.clone());
                    self.array_assign(&context, args, budget)
                }
                "Dictionary.assign" | "Dictionary.clear" => {
                    let context = reference.context.map(Value::object).unwrap_or_else(|| context.clone());
                    self.dictionary_assign(&context, args, name == "Dictionary.clear", budget)
                }
                "RegExp" => self.regexp_new(args),
                "Dictionary" => self.allocate(ObjectKind::Dictionary),
                "Array" => self.allocate(ObjectKind::Array(vec![])),
                "Exception" => {
                    self.exception_object(&args.first().map(Value::text).unwrap_or_default())
                }
                _ if name.starts_with("ScriptsEx.") => {
                    let context = reference
                        .context
                        .map(Value::object)
                        .unwrap_or_else(|| context.clone());
                    self.scripts_ex(&name[10..], &context, args, host, budget, result_needed)
                }
                _ if name.starts_with("Serialization.") => {
                    let context = reference
                        .context
                        .map(Value::object)
                        .unwrap_or_else(|| context.clone());
                    self.serialization_call(
                        &name[14..],
                        &context,
                        args,
                        host,
                        budget,
                        result_needed,
                    )
                }
                _ => {
                    let context = reference
                        .context
                        .map(Value::object)
                        .unwrap_or_else(|| context.clone());
                    self.object_id(&context)?;
                    host.call_with_result(self, &name, &context, args, budget, result_needed)
                }
            },
            ObjectKind::Function(function) => {
                let mut frame = Frame::global();
                frame.context = reference.context.unwrap_or(self.object_handle(context)?);
                frame.owner = self.objects[id].owner;
                frame.arguments = args.to_vec();
                ensure!(
                    frame.context < self.objects.len(),
                    "invalid TJS bound context"
                );
                frame.scopes.push(BTreeMap::new());
                self.run(&function.program, host, budget, frame)
            }
            ObjectKind::Method { receiver, name } => {
                self.method(&receiver, &name, args, host, budget, result_needed)
            }
            _ => bail!("TJS object is not callable"),
        }
    }
    pub(crate) fn exception_object(&mut self, message: &str) -> Result<Value> {
        let value = self.allocate(ObjectKind::Dictionary)?;
        let id = self.object_id(&value)?;
        self.objects[id].classes.push("Exception".into());
        self.set_member(&value, &Value::string("message"), Value::string(message))?;
        self.set_member(&value, &Value::string("trace"), Value::string(message))?;
        Ok(value)
    }
}
