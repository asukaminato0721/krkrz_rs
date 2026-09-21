use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

fn execute(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), None, budget)?.execute_storage("case.tjs")
}

#[test]
fn original_copy_alpha_to_province_pixels_and_options() {
    #[derive(Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
        budget: u64,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_copy_alpha_to_province.json")).unwrap();
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
fn copy_alpha_budget_abort_cannot_be_hidden_by_script_catch() {
    let source = "Plugins.link('PackinOne.dll');var w=new Window(),d=new Layer(w,null);try{d.copyAlphaToProvince();}catch(e){return 99;}";
    let error = execute(source, 500).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert!(format!("{error:#}").contains("copyAlphaToProvince execution budget"));
}

#[test]
fn copied_alpha_drives_province_hit_testing() {
    assert_eq!(
        execute(
            r#"
            Plugins.link('PackinOne.dll');
            var w=new Window(), p=new Layer(w,null), d=new Layer(w,p);
            p.setSize(2,1); d.setSize(2,1); d.visible=true;
            d.setMaskPixel(0,0,15); d.setMaskPixel(1,0,16);
            d.copyAlphaToProvince(16); d.hitType=htProvince;
            return (p.getLayerAt(0,0)!==d) && (p.getLayerAt(1,0)===d);
            "#,
            100_000,
        )
        .unwrap(),
        Value::Integer(1),
    );
}
