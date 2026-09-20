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
fn original_csv_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/csv.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000).unwrap();
        let value = session
            .execute_storage("case.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}

#[test]
fn callback_reinitialization_shares_execution_budget() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let source = r#"Plugins.link("csvParser.dll");var p=new CSVParser();
p.doLine=function(a,n){p.init("forever");};
try { p.parse("forever"); } catch { return 42; }"#;
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 1000).unwrap();
    let error = session.execute_storage("case.tjs").unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
