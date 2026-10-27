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

#[test]
fn window_update_delivers_pending_paint_before_returning() {
    let source = r#"
var w=new Window(),p=new Layer(w,null),c=new Layer(w,p),events=[];
p.onPaint=function(){events.add('parent');};
c.onPaint=function(){events.add('child');};
p.update();c.update();w.update();events.add('returned');
w.update();
return events.join('|')+'|'+p.callOnPaint+'|'+c.callOnPaint;
"#;
    assert_eq!(
        run(source, 100_000).unwrap(),
        Value::string("parent|child|returned|0|0")
    );
    assert_eq!(
        run("var w=new Window();w.update();return 1;", 100_000).unwrap(),
        Value::Integer(1)
    );
}

#[test]
fn closing_during_input_keeps_the_window_valid_until_the_callback_returns() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    session.evaluate("Scripts.exec('global.w=new Window();w.visible=true;var p=new Layer(w,null);w.onMouseDown=function(){w.close();global.alive=isvalid w;};')").unwrap();
    let window = session.evaluate("w").unwrap();
    session
        .input(
            &window,
            krkrz_runtime::InputEvent::PointerDown {
                x: 1,
                y: 1,
                button: 0,
                shift: 8,
            },
        )
        .unwrap();
    assert_eq!(session.evaluate("alive").unwrap(), Value::Integer(1));
    assert_eq!(session.evaluate("isvalid w").unwrap(), Value::Integer(0));
}
