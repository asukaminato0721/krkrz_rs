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
    Session::open(project.path(), Some(saves.path()), None, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_window_lifetime_and_context_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/window_core.json")).unwrap();
    for case in cases {
        assert_eq!(
            run(&case.source, 100_000).unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}
#[test]
fn window_invalidation_does_not_swallow_execution_limits() {
    let error = run(
        "class C{function finalize(){while(1){}}}var w=new Window();w.add(new C());invalidate w;",
        1000,
    )
    .unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
