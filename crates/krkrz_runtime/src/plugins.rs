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

/// Export names read from the installed standalone plugins. Operations use
/// the normal Layer dispatch, which reports unsupported effects explicitly.
pub(crate) fn layer_effect(vm: &mut Vm, plugin: &str) -> Result<()> {
    let methods: &[&str] = match plugin {
        "layerexalpha.dll" => &[
            "turnAlpha",
            "turnAlpha2",
            "turnAlpha3",
            "AlphaColorBlend",
            "AlphaColorBlend2",
            "AlphaColorRect",
            "turnOutline",
            "AlphaDivide",
            "AlphaAdd",
            "AlphaAdd2",
            "BlackForAlpha",
            "BlackForAlpha2",
            "AlphaBlend",
            "ColorAlphaAdd",
            "ColorVectorEffectAdd",
            "ExtraColorVectorEffectAdd",
            "WhiteToBlueBlurAdd",
            "MaskRect",
            "ExMaskRect",
            "ExMaskRect2",
            "LightForAlpha",
            "LuminanceForAlpha",
            "LevelMaskRect",
            "LevelMaskRect2",
            "MaskImageRideRect",
            "MaskImageRideRect2",
            "MaskImageRideRect3",
            "MaskImageRideRect4",
            "MaskImageRideRect5",
            "MaskImageRideRect7",
            "MaskImageRideRect8",
            "AlphaBorderInnerRect",
            "AlphaBorderImageCoverRect",
            "MaskImageEffectRect",
        ],
        "layerexcolor.dll" => &["TranslucentColorBlend"],
        "layerexraster.dll" => &[
            "copyRaster",
            "copyNotInterpolationRaster",
            "copyInterpolationRaster",
            "copyVerticalRaster",
            "copyVerticalNotInterpolationRaster",
            "copyVerticalInterpolationRaster",
        ],
        "layerexmosaic.dll" => &["AddMosaic"],
        _ => anyhow::bail!("unknown Layer effect plugin: {plugin}"),
    };
    for method in methods {
        vm.register_native(&format!("Layer.{method}"))?;
    }
    Ok(())
}

pub(crate) fn packinone(vm: &mut Vm) -> Result<()> {
    // Layer is normally registered as a core class before loading plugins.
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
