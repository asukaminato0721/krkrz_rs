use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn session(source: &str, budget: u64) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, budget).unwrap();
    session.execute_storage("case.tjs").unwrap();
    (project, saves, session)
}
#[test]
fn original_timer_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/timer.json")).unwrap();
    for case in cases {
        let (project, saves, mut session) = session("", 100_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        assert_eq!(
            session
                .execute_storage("test.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
        drop(saves);
    }
}
const COUNT: &str =
    "global.n=0;global.t=new Timer(function(e){global.n++;},'');t.interval=10;t.enabled=true;";
#[test]
fn fixed_point_deadlines_capacity_and_late_wakeups() {
    let (_project, _saves, mut session) = session(COUNT, 100_000);
    session.advance_clock(10).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(0));
    session.advance_clock(11).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(1));
    session.advance_clock(100).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(7)); // queue capacity six
    session.advance_clock(1000).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(8)); // >40 intervals collapses to one
    session.evaluate("t.enabled=false").unwrap();
    session.advance_clock(2000).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(8));
    session
        .evaluate("(t.interval=0.5,t.enabled=true,t.capacity=0)")
        .unwrap();
    session.advance_clock(2001).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(10));
    session.evaluate("(t.interval=0,t.enabled=true)").unwrap();
    session.advance_clock(3000).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(10));
}
#[test]
fn queued_timer_callbacks_cancel_immediately_and_share_async_priority() {
    let (_project, _saves, mut session) = session(
        "global.log=[];global.t=new Timer(function(){global.log.add('timer');global.t.enabled=false;},'');t.interval=1;t.enabled=true;global.a=new AsyncTrigger(function(){global.log.add('async');},'');a.mode=1;a.trigger();",
        100_000,
    );
    session.advance_clock(10).unwrap();
    assert_eq!(
        session.evaluate("log.join(',')").unwrap(),
        Value::string("async,timer")
    );
    session
        .evaluate("(t.enabled=true,a.mode=0,a.trigger())")
        .unwrap();
    session.advance_clock(20).unwrap();
    assert_eq!(
        session.evaluate("log.join(',')").unwrap(),
        Value::string("async,timer,async,timer")
    );
    session
        .evaluate("(t.enabled=true,a.mode=1,a.onFire=function(){invalidate global.t;},a.trigger())")
        .unwrap();
    session.advance_clock(30).unwrap();
    assert_eq!(session.evaluate("log.count").unwrap(), Value::Integer(4));
}
#[test]
fn changing_interval_cancels_due_callbacks_and_resets_deadline() {
    let (_project, _saves, mut session) = session(
        "global.n=0;global.t=new Timer(function(){global.n++;global.t.interval=20;},'');t.interval=1;t.enabled=true;",
        10_000,
    );
    session.advance_clock(10).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(1));
    session.advance_clock(30).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(1));
    session.advance_clock(31).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(2));
}
#[test]
fn lax_capacity_and_zero_capacity_have_reference_meaning() {
    let (_project, _saves, mut session) = session(COUNT, 10_000);
    session
        .services
        .arguments
        .insert("-laxtimer".into(), "yes".into());
    session.advance_clock(100).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(1));
    session.services.arguments.remove("-laxtimer");
    session.evaluate("t.capacity=0").unwrap();
    session.advance_clock(200).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(11));
    session.evaluate("t.capacity=-1").unwrap();
    session.advance_clock(300).unwrap();
    assert_eq!(session.evaluate("n").unwrap(), Value::Integer(11));
}
#[test]
fn timer_callback_execution_uses_session_budget() {
    let (_project, _saves, mut session) = session(
        "var t=new Timer(function(){while(1){}},'');t.interval=1;t.enabled=true;",
        1000,
    );
    let error = session.advance_clock(2).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
