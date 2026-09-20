use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn execute(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_text_render_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/text_render.json")).unwrap();
    for case in cases {
        let result = execute(&case.source, 100_000)
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}
#[test]
fn pending_reference_cases_fail_explicitly() {
    // These expected reference values document unfinished behavior. Rejecting
    // them is a safety boundary, not reference agreement or slice completion.
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/text_render_pending.json")).unwrap();
    for case in cases {
        let error = execute(&case.source, 100_000).unwrap_err();
        assert!(
            error.downcast_ref::<VmAbort>().is_some(),
            "{}: {error:#}",
            case.name
        );
    }
}
#[test]
fn callbacks_share_budget_and_invalidation_cannot_panic() {
    let prefix =
        r#"Plugins.link("textrender.dll");var r=new TextRenderBase();r.setRenderSize(100,200);"#;
    let error=execute(&format!("{prefix}r.onGetTextWidth=function(s,z){{while(true){{}}}};try{{r.render(\"A\",0,10,0,false);}}catch(e){{return 999;}}"),10_000).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    let value=execute(&format!("{prefix}r.onGetTextWidth=function(s,z){{invalidate r;return 10;}};try{{r.render(\"A\",0,10,0,false);}}catch(e){{return 7;}}"),10_000).unwrap();
    assert_eq!(value, Value::Integer(7));
}
#[test]
fn malformed_text_and_native_array_isolation() {
    let value = execute(
        r#"
        Plugins.link("textrender.dll");Plugins.link("saveStruct.dll");
        var r=new TextRenderBase();r.setRenderSize(100,200);
        r.onGetTextWidth=function(s,z){return s.length*10;};
        var errors=0;
        try{r.render("%funterminated",0,0,0,false);}catch(e){errors++;}
        r.clear();
        try{r.render("%abc;",0,0,0,false);}catch(e){errors++;}
        return errors;
    "#,
        10_000,
    )
    .unwrap_err();
    // Unknown commands are engine limitations, not swallowed script errors.
    assert!(value.downcast_ref::<VmAbort>().is_some());
    let value=execute(r#"
        Plugins.link("textrender.dll");Plugins.link("saveStruct.dll");
        var r=new TextRenderBase();r.setRenderSize(100,200);
        Array.probe=9;
        var chars=r.getCharacters(0,0);
        return (typeof chars.probe=="undefined") && (typeof chars.saveStruct=="Object") && (typeof chars.toStructString=="undefined");
    "#,10_000).unwrap();
    assert_eq!(value, Value::Integer(1));
    let error = execute(
        r#"Plugins.link("textrender.dll");var r=new TextRenderBase();
        r.vertical=1;r.setRenderSize(100,200);
        r.onGetTextWidth=function(s,z){return s.length*10;};
        r.render("[ruby]A",0,10,0,false);"#,
        10_000,
    )
    .unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
