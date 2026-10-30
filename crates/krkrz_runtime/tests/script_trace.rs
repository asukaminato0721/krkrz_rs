use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn script_trace_reports_nested_storage_calls_and_unwinds_after_errors() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        "Scripts.execStorage('inner.tjs');",
    )
    .unwrap();
    std::fs::write(
        project.path().join("inner.tjs"),
        "global.trace=Scripts.getTraceString();global.shortTrace=Scripts.getTraceString(1);global.negativeTrace=Scripts.getTraceString(-1);",
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10000).unwrap();
    session.startup().unwrap();
    assert_eq!(
        session.evaluate("trace").unwrap(),
        Value::string("inner.tjs(1) <-- startup.tjs(1)")
    );
    assert_eq!(
        session.evaluate("shortTrace").unwrap(),
        Value::string("inner.tjs(1)")
    );
    assert!(session.evaluate("1+missing()").is_err());
    assert_eq!(
        session.evaluate("negativeTrace").unwrap(),
        Value::string("inner.tjs(1)")
    );
    assert_eq!(
        session.evaluate("Scripts.getTraceString()").unwrap(),
        Value::string("<tool>(1)")
    );
}
