use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn legacy_console_is_a_stable_read_only_class_without_a_native_window() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    assert_eq!(
        session
            .evaluate("Debug.console===Debug.console && Debug.console instanceof 'Class'")
            .unwrap(),
        Value::Integer(1)
    );
    assert!(session.evaluate("Debug.console=%[]").is_err());
    session.evaluate("Debug.console.visible=true").unwrap();
    assert_eq!(
        session.evaluate("Debug.console.visible").unwrap(),
        Value::Integer(0)
    );
    session.collect_garbage(&[]).unwrap();
    assert_eq!(
        session.evaluate("Debug.console.visible").unwrap(),
        Value::Integer(0)
    );
    assert_eq!(
        session.evaluate("typeof global.Console").unwrap(),
        Value::string("undefined")
    );
    assert_eq!(
        session.evaluate("invalidate new Debug.console()").unwrap(),
        Value::Integer(1)
    );
}

#[test]
fn legacy_window_ex_loads_without_game_supplied_placeholders() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    session.evaluate("Plugins.link('windowEx.dll')").unwrap();
    session.collect_garbage(&[]).unwrap();
    assert_eq!(
        session.evaluate("[Debug.console.getRect()===void,Debug.console.maximize(),Debug.console.restoreMaximize(),Debug.console.setPlacement(%[]),typeof Pad.registerExEvent].join(',')").unwrap(),
        Value::string("1,0,0,0,Object")
    );
    std::fs::write(
        project.path().join("case.tjs"),
        "var w=new Window();w.registerExEvent();w.setPos(12,34);",
    )
    .unwrap();
    session.execute_storage("case.tjs").unwrap();
    assert_eq!(
        session.evaluate("w.getWindowRect().x").unwrap(),
        Value::Integer(12)
    );
    assert!(session.evaluate("new Pad()").is_err());
}

#[test]
fn window_ex_resolves_game_defined_console_accessors() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    std::fs::write(project.path().join("case.tjs"), "var c=%[];delete Debug.console;class CustomDebug { property console { getter() { return global.c; } } } global.Debug=new CustomDebug();Plugins.link('windowEx.dll');").unwrap();
    session.execute_storage("case.tjs").unwrap();
    assert_eq!(
        session.evaluate("c.getRect()===void").unwrap(),
        Value::Integer(1)
    );
}
