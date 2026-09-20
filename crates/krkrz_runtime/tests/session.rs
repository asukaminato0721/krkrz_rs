use krkrz_runtime::Session;
use krkrz_tjs::Value;
#[test]
fn nested_original_storage_calls_share_globals_and_budget() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("STARTUP.TJS"),
        "var x=1; Scripts.execStorage('next.tjs'); return x;",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Next.tjs"),
        "x=x+2; Debug.message('test', x);",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 100).unwrap();
    session.services.trace_enabled = true;
    assert_eq!(session.startup().unwrap(), Value::Integer(3));
    assert_eq!(session.services.messages, ["test 3"]);
    assert!(session.budget < 100);
    assert!(
        session
            .services
            .trace
            .iter()
            .any(|e| e.storage == "next.tjs")
    );
    assert!(std::fs::read_dir(saves.path()).unwrap().next().is_none());
}
#[test]
fn unsupported_call_is_an_error_with_storage_context() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "Plugins.link('required.dll');",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 100).unwrap();
    let error = format!("{:#}", session.startup().unwrap_err());
    assert!(error.contains("startup.tjs:1:1"));
    assert!(error.contains("unsupported Kirikiri native operation: Plugins.link"));
}
#[test]
fn nested_scripts_cannot_reset_execution_budget() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "Scripts.execStorage('startup.tjs');",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 10).unwrap();
    assert!(format!("{:#}", session.startup().unwrap_err()).contains("execution budget exhausted"));
    assert_eq!(session.budget, 0);
}
