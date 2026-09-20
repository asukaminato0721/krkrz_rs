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
fn linux_host_reports_dwm_preview_unavailable() {
    assert_eq!(
        run(
            &format!("{SETUP}return System.setIconicPreview(true);"),
            10_000
        )
        .unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn system_menu_snapshot_reset_and_selection_callback() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), format!(
        "{SETUP}global.w=new Window();w.registerExEvent();global.selected=void;w.onExSystemMenuSelected=function(item){{global.selected=item.state;return 7;}};global.leaf=%[caption:'first',state:42,checked:1,group:1];global.disabled=%[caption:'disabled',state:99,enabled:0];w.exSystemMenu=[%[caption:'hidden',visible:0],%[caption:'-'],%[caption:'parent',children:[leaf,disabled]]];"
    )).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 50_000).unwrap();
    session.execute_storage("case.tjs").unwrap();
    let window = session.evaluate("w").unwrap();
    let items = session.system_menu(&window).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[0].command.is_none());
    let leaf = &items[1].children[0];
    assert_eq!(leaf.caption, "first");
    assert!(leaf.checked && leaf.radio);
    let command = leaf.command.unwrap();
    assert_eq!(command, 0xefff);
    assert!(session.select_system_menu(&window, command - 1).is_err());
    assert_eq!(
        session.select_system_menu(&window, command).unwrap(),
        Value::Integer(7)
    );
    assert_eq!(session.evaluate("selected").unwrap(), Value::Integer(42));
    session
        .evaluate("(leaf.caption='updated',leaf.state=43)")
        .unwrap();
    assert_eq!(
        session.system_menu(&window).unwrap()[1].children[0].caption,
        "first"
    );
    session.select_system_menu(&window, command).unwrap();
    assert_eq!(session.evaluate("selected").unwrap(), Value::Integer(43));
    session.evaluate("w.resetExSystemMenu()").unwrap();
    assert_eq!(
        session.system_menu(&window).unwrap()[1].children[0].caption,
        "updated"
    );
    session.evaluate("w.exSystemMenu=null").unwrap();
    assert!(session.system_menu(&window).unwrap().is_empty());
    assert!(session.select_system_menu(&window, command).is_err());
    session.evaluate("invalidate w").unwrap();
    assert!(session.system_menu(&window).is_err());
}

#[test]
fn system_menu_callbacks_share_limits_and_handle_invalidation() {
    for body in [
        "var a=%[caption:'cycle'];a.children=[a];w.exSystemMenu=[a];",
        "class C{property caption{getter(){while(1){}}}}w.exSystemMenu=[new C()];",
    ] {
        let error = run(
            &format!("{SETUP}var w=new Window();w.registerExEvent();{body}"),
            2000,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    }
    assert_eq!(run(&format!("{SETUP}global.w=new Window();w.registerExEvent();class C{{property caption{{getter(){{invalidate global.w;return 'gone';}}}}}}try{{w.exSystemMenu=[new C()];}}catch(e){{return isvalid w;}}return 99;"), 10_000).unwrap(), Value::Integer(0));
}
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
