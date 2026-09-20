use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

fn execute(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}

#[test]
fn original_fill_alpha_clipping_and_ignored_arguments() {
    #[derive(Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
        budget: u64,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_fill_alpha.json")).unwrap();
    for case in cases {
        assert_eq!(
            execute(&case.source, case.budget).unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn fill_alpha_budget_abort_cannot_be_hidden_by_script_catch() {
    let source = "Plugins.link('PackinOne.dll');var w=new Window(),d=new Layer(w,null);try{d.fillAlpha();}catch(e){return 99;}";
    let error = execute(source, 500).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert!(format!("{error:#}").contains("fillAlpha execution budget"));
}
