//! Plugin registration is separate from operation execution. Unimplemented
//! declared methods/accessors reach the host's explicit unsupported-call error.
use anyhow::{Context, Result};
use krkrz_tjs::{Value, Vm};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Exports {
    methods: Vec<String>,
    properties: Vec<String>,
}

pub(crate) fn declare_class(vm: &mut Vm, name: &str) -> Result<()> {
    let exports: BTreeMap<String, Exports> =
        serde_json::from_str(include_str!("../data/native_exports.json"))?;
    let exports = exports
        .get(name)
        .context("missing native export declaration")?;
    let class = vm.register_native_class(name)?;
    for method in &exports.methods {
        vm.register_native(&format!("{name}.{method}"))?;
    }
    for property in &exports.properties {
        vm.register_native_property(
            &class,
            property,
            Some(&format!("{name}.get:{property}")),
            Some(&format!("{name}.set:{property}")),
        )?;
    }
    Ok(())
}

pub(crate) fn packinone(vm: &mut Vm) -> Result<()> {
    // Layer is a core class. Until its Rust renderer is bound, construction and
    // all declared operations fail explicitly at invocation.
    if !vm.globals.contains_key("Layer") {
        declare_class(vm, "Layer")?;
    }
    for class in ["Process", "TemporaryFiles"] {
        declare_class(vm, class)?;
    }
    for method in [
        "changeDirectory",
        "clearStorageCaches",
        "copyFile",
        "copyFileNoNormalize",
        "createDirectory",
        "createDirectoryNoNormalize",
        "deleteFile",
        "dirlist",
        "dirlistEx",
        "exportFile",
        "fstat",
        "getDisplayName",
        "getFileAttributes",
        "getLastModifiedFileTime",
        "getMD5HashString",
        "getTemporaryName",
        "getTime",
        "isExistentDirectory",
        "isExistentStorageNoSearchNoNormalize",
        "moveFile",
        "removeDirectory",
        "resetFileAttributes",
        "searchPath",
        "selectDirectory",
        "setCurrentDirectory",
        "setFileAttributes",
        "setLastModifiedFileTime",
        "setTime",
        "truncateFile",
    ] {
        vm.register_native(&format!("Storages.{method}"))?;
    }
    let storages = vm.globals["Storages"].clone();
    vm.register_native_property(
        &storages,
        "currentPath",
        Some("Storages.get:currentPath"),
        Some("Storages.set:currentPath"),
    )?;
    for method in [
        "addFont",
        "commandExecute",
        "confirm",
        "expandEnvString",
        "getAboutString",
        "readEnvValue",
        "urldecode",
        "urlencode",
        "waitForAppLock",
        "writeEnvValue",
        "writeRegValue",
    ] {
        vm.register_native(&format!("System.{method}"))?;
    }
    for (name, value) in [
        ("READONLY", 1),
        ("HIDDEN", 2),
        ("SYSTEM", 4),
        ("DIRECTORY", 16),
        ("ARCHIVE", 32),
        ("NORMAL", 128),
        ("TEMPORARY", 256),
    ] {
        vm.globals
            .insert(format!("FILE_ATTRIBUTE_{name}"), Value::Integer(value));
    }
    Ok(())
}
