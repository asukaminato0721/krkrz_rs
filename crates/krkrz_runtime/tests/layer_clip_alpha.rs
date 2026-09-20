use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
    budget: u64,
}
fn execute(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_clip_alpha_rounding_modes_clipping_and_overlap() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_clip_alpha.json")).unwrap();
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
fn clipping_budget_abort_cannot_be_hidden_by_script_catch() {
    let source = "Plugins.link('PackinOne.dll');var w=new Window(),d=new Layer(w,null),s=new Layer(w,d);try{d.clipAlphaRect(0,0,s,0,0,32,32);}catch(e){return 99;}";
    let error = execute(source, 500).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert!(
        format!("{error:#}").contains("clipAlphaRect execution budget"),
        "{error:#}"
    );
}
#[test]
fn invalid_sources_report_script_errors() {
    let source = "Plugins.link('PackinOne.dll');var w=new Window(),d=new Layer(w,null),s=new Layer(w,d),r=[];s.hasImage=false;try{d.clipAlphaRect(0,0,null,0,0,1,1);}catch(e){r.add(1);}try{d.clipAlphaRect(0,0,s,0,0,1,1);}catch(e){r.add(2);}return r.join(',');";
    assert_eq!(execute(source, 10000).unwrap(), Value::string("1,2"));
}
