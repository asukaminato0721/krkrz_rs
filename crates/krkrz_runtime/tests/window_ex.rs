use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn run(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_window_ex_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/window_ex.json")).unwrap();
    for c in cases {
        assert_eq!(
            run(&c.source, 500_000).unwrap_or_else(|e| panic!("{}: {e:#}", c.name)),
            c.expected,
            "{}",
            c.name
        );
    }
}
const SETUP: &str =
    "Plugins.link('menu.dll');global.Pad=%[];Debug.console=%[];Plugins.link('windowEx.dll');";
#[test]
fn callbacks_share_budget_and_handle_reentrant_invalidation() {
    let source = format!(
        "{SETUP}class W extends Window{{function W(){{super.Window();}}property onMove{{getter(){{invalidate this;return void;}}}}}}var w=new W();try{{w.registerExEvent();}}catch(e){{return isvalid w;}}return 99;"
    );
    assert_eq!(run(&source, 100_000).unwrap(), Value::Integer(0));
    let source = format!(
        "{SETUP}class W extends Window{{function W(){{super.Window();}}property onMove{{getter(){{while(true){{}}}}}}}}var w=new W();try{{w.registerExEvent();}}catch(e){{return 99;}}"
    );
    let error = run(&source, 2000).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
#[test]
fn native_window_operations_are_explicit_until_host_support_exists() {
    for operation in [
        "System.getDisplayMonitors()",
        "w.maximize()",
        "w.getWindowRect()",
        "w.setOverlayBitmap(null)",
    ] {
        let error = run(
            &format!("{SETUP}var w=new Window();try{{{operation};}}catch(e){{return 99;}}"),
            100_000,
        )
        .unwrap_err();
        assert!(
            error.downcast_ref::<VmAbort>().is_some(),
            "{operation}: {error:#}"
        );
    }
}
