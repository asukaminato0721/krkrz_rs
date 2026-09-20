use crate::{Program, Value};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub parameters: Vec<String>,
    pub rest: Option<String>,
    pub program: Program,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub getter: Option<Function>,
    pub setter: Option<Function>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Class {
    pub name: String,
    pub bases: Vec<Program>,
    pub methods: Vec<Function>,
    pub properties: Vec<Property>,
    pub initializer: Option<Program>,
}
#[derive(Clone, Debug)]
pub(crate) enum ObjectKind {
    Global,
    Namespace,
    Dictionary,
    ReadOnly(crate::readonly::ReadOnlyView),
    Array(Vec<Value>),
    Function(Arc<Function>),
    Class {
        definition: Arc<Class>,
        native_initializer: Option<String>,
    },
    Instance,
    RegExp {
        compiled: Arc<fancy_regex::Regex>,
        global: bool,
    },
    Property {
        getter: Option<Value>,
        setter: Option<Value>,
    },
    VariantProperty(Value),
    Super {
        bases: Vec<usize>,
        context: usize,
    },
    Native(String),
    Method {
        receiver: Value,
        name: String,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Object {
    pub kind: ObjectKind,
    pub members: BTreeMap<Vec<u16>, Value>,
    pub owner: Option<usize>,
    pub date: Option<i64>,
    pub classes: Vec<String>,
    pub member_flags: BTreeMap<Vec<u16>, u32>,
    // Native static methods receive the script caller's this, not their namespace.
    pub native_static: bool,
    pub call_missing: bool,
    pub processing_missing: bool,
    pub member_layout: crate::member_layout::MemberLayout,
    pub hash_generation: u64,
    pub valid: bool,
    pub finalizing: bool,
    pub native_finalizers: Vec<String>,
}
impl Object {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            kind,
            members: BTreeMap::new(),
            owner: None,
            date: None,
            classes: vec![],
            member_flags: BTreeMap::new(),
            native_static: false,
            call_missing: false,
            processing_missing: false,
            member_layout: Default::default(),
            hash_generation: 0,
            valid: true,
            finalizing: false,
            native_finalizers: vec![],
        }
    }
}

/// Engine limitations and resource limits must not disappear inside a script catch.
#[derive(Debug)]
pub struct VmAbort(pub String);
impl std::fmt::Display for VmAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for VmAbort {}
pub fn unsupported(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(VmAbort(message.into()))
}

#[derive(Debug)]
pub(crate) struct Thrown {
    pub value: Value,
    pub message: String,
}
impl std::fmt::Display for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Thrown {}
