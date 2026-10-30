//! Native M2 Motion resource management and script-driven 2D playback.
mod binding;
mod model;
mod raster;
pub(crate) use binding::State;

use anyhow::Result;
use krkrz_tjs::{Value, Vm};

pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let motion = vm.register_native_class("Motion")?;
    vm.register_native_static_value(&motion, "PlayFlagForce", Value::Integer(1))?;
    for (name, methods) in [
        (
            "ResourceManager",
            &[
                "ResourceManager",
                "finalize",
                "load",
                "unload",
                "unloadAll",
                "clearCache",
            ][..],
        ),
        (
            "Player",
            &[
                "Player",
                "finalize",
                "setCoord",
                "setZoom",
                "stop",
                "play",
                "clear",
                "progress",
                "frameProgress",
                "draw",
                "getLayerNames",
            ][..],
        ),
    ] {
        let prefix = format!("Motion.{name}");
        let class = vm.new_native_class(name, &format!("{prefix}.@initialize"))?;
        for method in methods {
            vm.register_native_method(&class, method, &format!("{prefix}.{method}"))?;
        }
        if name == "Player" {
            for property in [
                "resourceManager",
                "chara",
                "motion",
                "stealthChara",
                "stealthMotion",
                "tickCount",
                "frameTickCount",
                "frameLastTime",
                "frameLoopTime",
                "playing",
                "allplaying",
                "hasCamera",
                "syncWaiting",
                "speed",
                "x",
                "y",
                "left",
                "top",
                "zoomX",
                "zoomY",
                "angleDeg",
                "angleRad",
                "opacity",
            ] {
                let readonly = matches!(
                    property,
                    "resourceManager"
                        | "frameLastTime"
                        | "frameLoopTime"
                        | "playing"
                        | "allplaying"
                        | "hasCamera"
                        | "syncWaiting"
                );
                vm.register_native_property(
                    &class,
                    property,
                    Some(&format!("{prefix}.get:{property}")),
                    (!readonly)
                        .then(|| format!("{prefix}.set:{property}"))
                        .as_deref(),
                )?;
            }
        }
        vm.register_native_static_value(&motion, name, class)?;
    }
    Ok(())
}
