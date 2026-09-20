use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

#[test]
fn original_plugin_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/scripts_ex.json")).unwrap();
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    for case in cases {
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000).unwrap();
        let result = session
            .execute_storage("case.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}

#[test]
fn cyclic_structures_and_missing_plugin_operations_fail_explicitly() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    for source in [
        "var a=[];a.add(a);Scripts.clone(a);",
        "var a=[],b=[];a.add(a);b.add(b);Scripts.equalStruct(a,b);",
        "try { Scripts.getMD5HashString('x'); } catch { return 42; }",
    ] {
        std::fs::write(
            project.path().join("case.tjs"),
            format!("Plugins.link('ScriptsEx.dll');{source}"),
        )
        .unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000).unwrap();
        let error = session.execute_storage("case.tjs").unwrap_err();
        assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    }
}
