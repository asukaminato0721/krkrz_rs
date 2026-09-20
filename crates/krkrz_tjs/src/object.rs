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
    pub methods: Vec<Function>,
    pub properties: Vec<Property>,
    pub fields: Vec<(String, Program)>,
}
#[derive(Clone, Debug)]
pub(crate) enum ObjectKind {
    Global,
    Namespace,
    Dictionary,
    Array(Vec<Value>),
    Function(Arc<Function>),
    Class {
        definition: Arc<Class>,
        bases: Vec<usize>,
    },
    Instance,
    Property {
        getter: Option<Value>,
        setter: Option<Value>,
    },
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
    pub classes: Vec<String>,
}
impl Object {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            kind,
            members: BTreeMap::new(),
            owner: None,
            classes: vec![],
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
