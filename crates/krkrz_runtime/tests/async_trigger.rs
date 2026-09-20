use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    (project, saves, session)
}
fn execute(s: &mut Session, source: &str) -> Value {
    let program = krkrz_tjs::compile("test", source).unwrap();
    s.vm.execute(&program, &mut s.services, &mut s.budget).unwrap()
}
#[test]
fn original_trigger_corpus() {
    #[derive(Deserialize)]
    struct Case { name: String, source: String, expected: Value }
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/async_trigger.json")).unwrap();
    for case in cases {
        let (_project, _saves, mut s) = session();
        assert_eq!(execute(&mut s, &case.source), case.expected, "{}", case.name);
    }
}
#[test]
fn priority_cache_cancel_and_callback_posts() {
    let (_project, _saves, mut s) = session();
    execute(&mut s, "global.events=[];class T extends AsyncTrigger{var name;function T(n,m=0){super.AsyncTrigger(null);name=n;mode=m;}function onFire(){events.add(name);}}global.a=new T('a');global.b=new T('b',1);global.c=new T('c',2);a.trigger();b.trigger();a.trigger();c.trigger();");
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string(""));
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("b,a,c"));
    execute(&mut s, "events.clear();a.cached=false;a.trigger();a.trigger();b.trigger();b.cancel();c.trigger();c.mode=0;");
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("a,a"));
    // Cancellation must affect events already eligible for this batch.
    execute(&mut s, "events.clear();a.onFire=function(){events.add('a');c.cancel();a.trigger();};a.trigger();c.trigger();");
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("a"));
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("a,a"));
    execute(&mut s, "invalidate a;");
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("a,a"));
}
#[test]
fn exclusive_reposting_defers_normal_and_idle_callbacks() {
    let (_project, _saves, mut s) = session();
    execute(&mut s, "global.events=[];global.n=new AsyncTrigger(function(){events.add('normal');},'');global.i=new AsyncTrigger(function(){events.add('idle');},'');i.mode=2;global.x=new AsyncTrigger(function(){events.add('exclusive');if(events.count==1)x.trigger();},'');x.mode=1;n.trigger();i.trigger();x.trigger();");
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("exclusive"));
    s.dispatch_events().unwrap();
    assert_eq!(execute(&mut s, "return events.join(',');"), Value::string("exclusive,exclusive,normal,idle"));
}
#[test]
fn callbacks_consume_the_session_budget() {
    let (_project, _saves, mut s) = session();
    execute(&mut s, "global.t=new AsyncTrigger(function(){while(true){}},'');t.trigger();");
    s.budget = 1000;
    let error = s.dispatch_events().unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
