use krkrz_runtime::Session;
use krkrz_tjs::Value;
#[test]
fn nested_original_storage_calls_share_globals_and_budget() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("STARTUP.TJS"),
        "var x=1; Scripts.execStorage('next.tjs'); return x;",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Next.tjs"),
        "x=x+2; Debug.message('test', x);",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 100).unwrap();
    session.services.trace_enabled = true;
    assert_eq!(session.startup().unwrap(), Value::Integer(3));
    assert_eq!(session.services.messages, ["test 3"]);
    assert!(session.budget < 100);
    assert!(
        session
            .services
            .trace
            .iter()
            .any(|e| e.storage == "next.tjs")
    );
    assert!(std::fs::read_dir(saves.path()).unwrap().next().is_none());
}
#[test]
fn unsupported_call_is_an_error_with_storage_context() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "Plugins.link('required.dll');",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 100).unwrap();
    let error = format!("{:#}", session.startup().unwrap_err());
    assert!(error.contains("startup.tjs:1:1"));
    assert!(error.contains("unsupported Kirikiri native operation: Plugins.link"));
}
#[test]
fn nested_scripts_cannot_reset_execution_budget() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "Scripts.execStorage('startup.tjs');",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 10).unwrap();
    assert!(format!("{:#}", session.startup().unwrap_err()).contains("execution budget exhausted"));
    assert_eq!(session.budget, 0);
}

#[test]
fn script_catch_cannot_hide_missing_plugins_or_compiled_script_support() {
    for call in [
        "Plugins.link('required.dll')",
        "Scripts.execStorage('binary.tjs')",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("startup.tjs"),
            format!("try{{{call};}}catch{{return 42;}}"),
        )
        .unwrap();
        std::fs::write(dir.path().join("binary.tjs"), b"TJS2").unwrap();
        let mut session = Session::open(dir.path(), Some(saves.path()), false, 1000).unwrap();
        let error = session.startup().unwrap_err();
        assert!(
            error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
            "{error:#}"
        );
    }
}

#[test]
fn preprocessor_state_persists_across_script_calls_but_is_not_runtime_globals() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "Scripts.exec('@set(SLICE=7)'); Scripts.execStorage('next.tjs'); return x;",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("next.tjs"),
        "@if(SLICE==7 && kirikiriz) var x=7; @endif @if(!SLICE) var x=99; @endif",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 1000).unwrap();
    assert_eq!(session.startup().unwrap(), Value::Integer(7));
    assert!(!session.vm.globals.contains_key("SLICE"));
}

#[test]
fn scripts_use_explicit_context_and_preserve_source_locations() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        r#"
        var d=%['x'=>2];
        Scripts.exec('var y=x+3;', 'dynamic.tjs', 20, d);
        Scripts.execStorage('next.tjs', , d);
        return Scripts.eval('x+y', , , d) + Scripts.evalStorage('expr.tjs', , d);
    "#,
    )
    .unwrap();
    std::fs::write(dir.path().join("next.tjs"), "x+=1;").unwrap();
    std::fs::write(dir.path().join("expr.tjs"), "x*2").unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 10_000).unwrap();
    session.services.trace_enabled = true;
    assert_eq!(session.startup().unwrap(), Value::Integer(14));
    assert!(!session.vm.globals.contains_key("y"));
    assert!(
        session
            .services
            .trace
            .iter()
            .any(|at| at.storage == "dynamic.tjs" && at.line == 21)
    );
}

#[test]
fn window_state_and_deferred_resize_callbacks_share_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        r#"
        System.title='Title';
        class PoolWindow extends Window {
            var resized=0, actions=0;
            function PoolWindow() {super.Window(...);}
            property poolLayer {getter(){return *(&global.Window.primaryLayer incontextof this);}}
            function onResize() {resized++; super.onResize();}
            function action(e) {if(e.type=='onResize' && e.target===this) actions++;}
        }
        var a=new PoolWindow(), b=new Window();
        a.setInnerSize(1280,720); a.innerWidth=960;
        a.visible=true; a.caption='Story';
        var resize=a.setInnerSize; resize(800,600);
        b.setInnerSize(320,240);
        return (a.poolLayer===null) && b.caption=='Title' && a.resized==0;
    "#,
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), false, 10_000).unwrap();
    assert_eq!(session.startup().unwrap(), Value::Integer(1));
    assert_eq!(session.services.windows.len(), 2);
    let story = session
        .services
        .windows
        .values()
        .find(|w| w.caption == "Story")
        .unwrap();
    assert!(story.visible);
    assert_eq!((story.inner_width, story.inner_height), (800, 600));
    session.advance_clock(10).unwrap();
    assert_eq!(session.evaluate("a.resized").unwrap(), Value::Integer(1));
    assert_eq!(session.evaluate("a.actions").unwrap(), Value::Integer(1));
    session.advance_clock(20).unwrap();
    assert_eq!(session.evaluate("a.resized").unwrap(), Value::Integer(1));
    assert_eq!(
        session.evaluate("b.innerWidth").unwrap(),
        Value::Integer(320)
    );
}
