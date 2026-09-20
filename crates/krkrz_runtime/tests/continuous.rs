use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};

fn execute(session: &mut Session, source: &str) -> Value {
    let code = krkrz_tjs::compile("continuous", source).unwrap();
    session
        .vm
        .execute(&code, &mut session.services, &mut session.budget)
        .unwrap()
}

#[test]
fn original_continuous_corpus() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/continuous.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut s = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        execute(&mut s, case["source"].as_str().unwrap());
        for t in 0..4 {
            s.tick(t * 16).unwrap();
        }
        assert_eq!(
            s.evaluate(case["await"].as_str().unwrap()).unwrap(),
            Value::Integer(1)
        );
        let expected: Value = serde_json::from_value(case["expected"].clone()).unwrap();
        assert_eq!(
            s.evaluate(case["result"].as_str().unwrap()).unwrap(),
            expected,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn live_handler_order_context_and_paint() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    execute(
        &mut s,
        r#"
        var events=[];var owner=%[name:'c'];
        var c=function(t){events.add(name+t);} incontextof owner;
        function a(t){events.add('a'+t);System.removeContinuousHandler(a);System.removeContinuousHandler(b);System.addContinuousHandler(c);}
        function b(t){events.add('b'+t);}
        System.addContinuousHandler(a);System.addContinuousHandler(a);System.addContinuousHandler(b);
        var w=new Window();var l=new Layer(w,null);w.visible=1;
        l.onPaint=function(){events.add('paint');};l.callOnPaint=1;
    "#,
    );
    s.tick(12).unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("a12,c12,paint")
    );
    s.tick(30).unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("a12,c12,paint,c30")
    );
    execute(
        &mut s,
        "System.removeContinuousHandler(c);System.removeContinuousHandler(c);System.addContinuousHandler(null);System.addContinuousHandler(%[]);",
    );
    s.tick(40).unwrap();
    assert_eq!(s.evaluate("events.count").unwrap(), Value::Integer(4));
}

#[test]
fn failed_handlers_are_removed_and_budget_is_shared() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    execute(
        &mut s,
        "var n=0;function fail(){n++;throw new Exception('callback failure');}System.addContinuousHandler(fail);",
    );
    assert!(s.tick(1).unwrap_err().to_string().contains("continuous"));
    s.tick(2).unwrap();
    assert_eq!(s.evaluate("n").unwrap(), Value::Integer(1));
    execute(
        &mut s,
        "System.addContinuousHandler(function(){while(true){}});",
    );
    s.budget = 100;
    let error = s.tick(3).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    s.budget = 100;
    s.tick(4).unwrap();
}
