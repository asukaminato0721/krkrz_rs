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
fn original_layer_draw_and_reached_language_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_draw.json")).unwrap();
    for case in cases {
        let value =
            execute(&case.source, 100_000).unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}
#[test]
fn conversion_callbacks_obey_budget_and_invalidated_receivers_fail() {
    let source = r#"Plugins.link("layerExDraw.dll");var p=new GdiPlus.PointF(0,0);
    class C { property x { getter(){while(true){}} } }
    try{p.Equals(new C());}catch(e){return 999;}"#;
    let error = execute(source, 1000).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    let source = r#"Plugins.link("layerExDraw.dll");var p=new GdiPlus.PointF(0,0);
    class C { property x { getter(){invalidate p;return 1;} } }
    try{p.Equals(new C());}catch(e){return 7;}return 999;"#;
    assert_eq!(execute(source, 10_000).unwrap(), Value::Integer(7));
    // The original null-point conversion timed out; do not reproduce its unsafe access.
    assert_eq!(execute(r#"Plugins.link("layerExDraw.dll");var p=new GdiPlus.PointF(0,0);try{p.Equals(null);}catch(e){return 1;}return 0;"#,10_000).unwrap(),Value::Integer(1));
}
#[test]
fn drawing_and_unimplemented_resources_fail_explicitly() {
    for source in [
        "new GdiPlus.Font('unused',24,0);",
        "new GdiPlus.Image();",
        "new GdiPlus.Path();",
        "new GdiPlus.Appearance();",
        "Layer.drawPath(null);",
    ] {
        let error = execute(
            &format!("Plugins.link('layerExDraw.dll');try{{{source}}}catch(e){{return 1;}}"),
            10_000,
        )
        .unwrap_err();
        assert!(
            error.downcast_ref::<VmAbort>().is_some(),
            "{source}: {error:#}"
        );
    }
}
