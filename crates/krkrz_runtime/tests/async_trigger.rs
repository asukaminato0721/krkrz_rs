use krkrz_runtime::{InputEvent, Session};
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    (project, saves, session)
}
fn execute(s: &mut Session, source: &str) -> Value {
    let program = krkrz_tjs::compile("test", source).unwrap();
    s.vm.execute(&program, &mut s.services, &mut s.budget)
        .unwrap()
}
#[test]
fn original_trigger_corpus() {
    #[derive(Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/async_trigger.json")).unwrap();
    for case in cases {
        let (_project, _saves, mut s) = session();
        assert_eq!(
            execute(&mut s, &case.source),
            case.expected,
            "{}",
            case.name
        );
    }
}
#[test]
fn priority_cache_cancel_and_callback_posts() {
    let (_project, _saves, mut s) = session();
    execute(
        &mut s,
        "global.events=[];class T extends AsyncTrigger{var name;function T(n,m=0){super.AsyncTrigger(null);name=n;mode=m;}function onFire(){events.add(name);}}global.a=new T('a');global.b=new T('b',1);global.c=new T('c',2);a.trigger();b.trigger();a.trigger();c.trigger();",
    );
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("")
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("b,a,c")
    );
    execute(
        &mut s,
        "events.clear();a.cached=false;a.trigger();a.trigger();b.trigger();b.cancel();c.trigger();c.mode=0;",
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("a,a")
    );
    // Cancellation must affect events already eligible for this batch.
    execute(
        &mut s,
        "events.clear();a.onFire=function(){events.add('a');c.cancel();a.trigger();};a.trigger();c.trigger();",
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("a")
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("a,a")
    );
    execute(&mut s, "invalidate a;");
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("a,a")
    );
}
#[test]
fn exclusive_reposting_defers_normal_and_idle_callbacks() {
    let (_project, _saves, mut s) = session();
    execute(
        &mut s,
        "global.events=[];global.n=new AsyncTrigger(function(){events.add('normal');},'');global.i=new AsyncTrigger(function(){events.add('idle');},'');i.mode=2;global.x=new AsyncTrigger(function(){events.add('exclusive');if(events.count==1)x.trigger();},'');x.mode=1;n.trigger();i.trigger();x.trigger();",
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("exclusive")
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        execute(&mut s, "return events.join(',');"),
        Value::string("exclusive,exclusive,normal,idle")
    );
}
#[test]
fn callbacks_consume_the_session_budget() {
    let (_project, _saves, mut s) = session();
    execute(
        &mut s,
        "global.t=new AsyncTrigger(function(){while(true){}},'');t.trigger();",
    );
    s.budget = 1000;
    let error = s.dispatch_events().unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}

fn pointer_move() -> InputEvent {
    InputEvent::PointerMove {
        x: 2,
        y: 2,
        shift: 0,
    }
}

#[test]
fn transition_continuation_initializes_scene_before_hover() {
    let (_project, _saves, mut s) = session();
    execute(
        &mut s,
        r#"
        global.events=[];
        global.w=new Window();w.setInnerSize(8,8);w.visible=true;
        global.root=new Layer(w,null);root.setSize(8,8);
        global.front=new Layer(w,root);front.setSize(8,8);front.visible=true;
        global.back=new Layer(w,root);back.setSize(8,8);back.visible=false;
        front.fillRect(0,0,8,8,0xffff0000);back.fillRect(0,0,8,8,0xff0000ff);
        global.resume=new AsyncTrigger(function(){
            global.sceneTitle='ready';events.add('resume');
        },'');resume.mode=atmExclusive;
        back.onMouseMove=function(){events.add(sceneTitle);};
        front.onTransitionCompleted=function(){resume.trigger();};
        front.beginTransition('crossfade',true,back,%[time:10]);
    "#,
    );
    let window = s.evaluate("w").unwrap();
    s.tick(0).unwrap();
    s.tick(10).unwrap();
    assert_eq!(s.evaluate("events.count").unwrap(), Value::Integer(0));
    s.input(&window, pointer_move()).unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("resume,ready")
    );
}

#[test]
fn input_waits_for_exclusive_chain_but_not_normal_or_idle_events() {
    let (_project, _saves, mut s) = session();
    execute(
        &mut s,
        r#"
        global.events=[];global.w=new Window();w.visible=true;
        w.onMouseMove=function(){events.add('input');};
        global.normal=new AsyncTrigger(function(){events.add('normal');},'');
        global.idle=new AsyncTrigger(function(){events.add('idle');},'');idle.mode=atmAtIdle;
        global.resume=new AsyncTrigger(function(){
            events.add('resume');if(events.count==1)resume.trigger();
        },'');resume.mode=atmExclusive;
        normal.trigger();idle.trigger();resume.trigger();
    "#,
    );
    let window = s.evaluate("w").unwrap();
    s.input(&window, pointer_move()).unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("resume,resume,input")
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("resume,resume,input,normal,idle")
    );
    execute(&mut s, "events.clear();resume.trigger();resume.cancel();");
    s.input(&window, pointer_move()).unwrap();
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("input")
    );
}

#[test]
fn exclusive_input_barrier_handles_closed_window_and_bounds_reposting() {
    for close in [true, false] {
        let (_project, _saves, mut s) = session();
        execute(
            &mut s,
            "global.w=new Window();w.visible=true;global.resume=new AsyncTrigger(null);resume.mode=atmExclusive;",
        );
        let window = s.evaluate("w").unwrap();
        execute(
            &mut s,
            if close {
                "resume.onFire=function(){invalidate w;};resume.trigger();"
            } else {
                "resume.onFire=function(){resume.trigger();};resume.trigger();"
            },
        );
        s.budget = 1000;
        let result = s.input(&window, pointer_move());
        if close {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
        }
    }
}
