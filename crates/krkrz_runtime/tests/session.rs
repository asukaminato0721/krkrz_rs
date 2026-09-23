use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn host_ticks_deliver_events_before_paint_and_preserve_hidden_updates() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        r#"
var events=[],w=new Window(),p=new Layer(w,null);
w.visible=true;p.onPaint=function(){events.add('paint');p.update();};
var t=new Timer(function(){events.add('timer');p.update();t.enabled=false;},'');
t.interval=10;t.enabled=true;
"#,
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10000).unwrap();
    session.startup().unwrap();
    session.tick(11).unwrap();
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("timer,paint")
    );
    session.evaluate("w.visible=false").unwrap();
    session.tick(12).unwrap();
    assert_eq!(session.evaluate("events.count").unwrap(), Value::Integer(2));
    assert_eq!(
        session.evaluate("p.callOnPaint").unwrap(),
        Value::Integer(1)
    );
    session.evaluate("w.visible=true").unwrap();
    session.tick(13).unwrap();
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("timer,paint,paint")
    );
    assert!(session.tick(12).is_err());
    session.budget = 0;
    assert!(session.tick(14).is_err());
}

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
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 100).unwrap();
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
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 100).unwrap();
    let error = format!("{:#}", session.startup().unwrap_err());
    assert!(error.contains("startup.tjs:1:1"));
    assert!(error.contains("unsupported Kirikiri native operation: Plugins.link"));
}

#[test]
fn movie_plugin_uses_builtin_video_overlay_without_replacing_objects() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        r#"
        var klass = VideoOverlay, w = new Window(), video = new VideoOverlay(w);
        video.left = 17;
        Plugins.link('plugin/KRMOVIE.dll');
        Plugins.link('plugin/KRMOVIE.dll');
        return [VideoOverlay === klass, video.left, Plugins.getList().join(',')].join('|');
    "#,
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10000).unwrap();
    assert_eq!(
        session.startup().unwrap(),
        Value::string("1|17|plugin/KRMOVIE.dll")
    );
}
#[test]
fn perspective_plugin_registers_layer_method_without_replacing_layer() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        "var klass=Layer;Plugins.link('perspective.dll');Plugins.link('perspective.dll');var w=new Window(),layer=new Layer(w,null);return [Layer===klass,typeof layer.perspectiveCopy,Plugins.getList().join(',')].join('|');",
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10_000).unwrap();
    assert_eq!(
        session.startup().unwrap(),
        Value::string("1|Object|perspective.dll")
    );
    let error = format!(
        "{:#}",
        session.evaluate("layer.perspectiveCopy()").unwrap_err()
    );
    assert!(error.contains("unsupported Layer operation: perspectiveCopy"));
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
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10).unwrap();
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
        let mut session = Session::open(dir.path(), Some(saves.path()), None, 1000).unwrap();
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
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 1000).unwrap();
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
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10_000).unwrap();
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
        var missingLayer=false;
        try {a.poolLayer;} catch(e) {missingLayer=true;}
        var primary=new Layer(a,null);
        a.setInnerSize(1280,720); a.innerWidth=960;
        a.visible=true; a.caption='Story';
        var resize=a.setInnerSize; resize(800,600);
        b.setInnerSize(320,240);
        return missingLayer && (a.poolLayer===primary) && b.caption=='Title' && a.resized==0;
    "#,
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10_000).unwrap();
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

#[test]
fn invalidation_releases_native_windows_and_cancels_queued_callbacks() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("startup.tjs"),
        r#"
        var trace='', fail=true;
        class W extends Window {
            function W(){super.Window();}
            function finalize(){trace+=this.caption; if(fail)throw 'retry';}
        }
        var a=new W(), b=new Window();
        a.caption='a';a.setInnerSize(100,200);b.setInnerSize(100,200);
        a.onResize=function(){invalidate b;trace+='r';};
        b.onResize=function(){trace+='wrong';};
        try{invalidate a;}catch(e){}
        return isvalid a;
        "#,
    )
    .unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10_000).unwrap();
    assert_eq!(session.startup().unwrap(), Value::Integer(1));
    assert_eq!(session.services.windows.len(), 2);
    session.dispatch_events().unwrap();
    assert_eq!(session.evaluate("trace").unwrap(), Value::string("ar"));
    assert_eq!(session.services.windows.len(), 1);
    session.evaluate("fail=false").unwrap();
    assert_eq!(session.evaluate("invalidate a").unwrap(), Value::Integer(1));
    assert!(session.services.windows.is_empty());
    assert_eq!(session.evaluate("trace").unwrap(), Value::string("ara"));
    session.dispatch_events().unwrap();
    assert_eq!(session.evaluate("invalidate a").unwrap(), Value::Integer(0));
}

#[test]
fn declared_plugin_exports_do_not_hide_unimplemented_operations() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10_000).unwrap();
    session.evaluate("Plugins.link('PackinOne.dll')").unwrap();
    session.evaluate("Plugins.link('KAGParserEx.dll')").unwrap();
    session
        .evaluate("Scripts.exec('var parser=KAGParser; var csv=CSVParser;')")
        .unwrap();
    session.evaluate("Plugins.link('packinone.DLL')").unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/plugins.json")).unwrap();
    for case in cases.as_array().unwrap() {
        std::fs::write(
            dir.path().join("plugin-case.tjs"),
            case["source"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            session.execute_storage("plugin-case.tjs").unwrap(),
            serde_json::from_value::<Value>(case["expected"].clone()).unwrap()
        );
    }
    session.evaluate("Plugins.link('kagparserex.DLL')").unwrap();
    assert_eq!(
        session
            .evaluate("parser===KAGParser && csv===CSVParser && FILE_ATTRIBUTE_NORMAL==128")
            .unwrap(),
        Value::Integer(1)
    );
    assert_eq!(session.evaluate("Scripts.exec('var p=new CSVParser();p.init(\"a,b\");return p.getNextLine().join(\"/\");')").unwrap(), Value::string("a/b"));
    for call in [
        "(new Layer(new Window(),null)).beginTransition(\"wave\")",
        "Layer.light(10,20)",
        "new Process()",
        "Storages.getTime(\"x\")",
    ] {
        let error = session
            .evaluate(&format!(
                "Scripts.exec('try{{{call};}}catch{{return 42;}}')"
            ))
            .unwrap_err();
        assert!(
            error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
            "{error:#}"
        );
    }
}

#[test]
fn app_lock_excludes_other_processes_and_releases_with_session() {
    // A subprocess tests the actual OS lock, not only per-session bookkeeping.
    const CHILD: &str = "KRKRZ_LOCK_TEST_CHILD";
    if let Ok(name) = std::env::var(CHILD) {
        let dir = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let mut session = Session::open(dir.path(), Some(saves.path()), None, 100).unwrap();
        assert_eq!(
            session
                .evaluate(&format!("System.createAppLock('{name}')"))
                .unwrap(),
            Value::Integer(0)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let name = format!(
        "krkrz-lock-test-{}-{}",
        std::process::id(),
        dir.path().file_name().unwrap().to_string_lossy()
    );
    let call = format!("System.createAppLock('{name}')");
    let mut first = Session::open(dir.path(), Some(saves.path()), None, 1000).unwrap();
    assert_eq!(first.evaluate(&call).unwrap(), Value::Integer(1));
    assert_eq!(first.evaluate(&call).unwrap(), Value::Integer(0));
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app_lock_excludes_other_processes_and_releases_with_session",
        ])
        .env(CHILD, &name)
        .status()
        .unwrap();
    assert!(status.success());
    drop(first);
    let mut second = Session::open(dir.path(), Some(saves.path()), None, 1000).unwrap();
    assert_eq!(second.evaluate(&call).unwrap(), Value::Integer(1));
}

#[test]
fn executable_identity_is_independent_of_the_archive_cipher() {
    let dir = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AnotherNovel.exe"), []).unwrap();
    std::fs::write(dir.path().join("unins000.exe"), []).unwrap();
    let mut session = Session::open(dir.path(), Some(saves.path()), None, 10000).unwrap();
    assert_eq!(
        session.evaluate("System.exeName").unwrap(),
        Value::string(&dir.path().join("AnotherNovel.exe").to_string_lossy())
    );
    let storage =
        krkrz_assets::storage::Storage::open(dir.path(), None, krkrz_core::Limits::default())
            .unwrap();
    let mut session = Session::from_storage(
        storage,
        Some(saves.path()),
        Some(std::path::Path::new("AnotherNovel.exe")),
        10000,
    )
    .unwrap();
    assert_eq!(
        session.evaluate("System.exeName").unwrap(),
        Value::string(&dir.path().join("AnotherNovel.exe").to_string_lossy())
    );
}

#[test]
fn standalone_layer_image_plugin_registers_without_packinone() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::open(project.path(), Some(saves.path()), None, 10000).unwrap();
    s.evaluate("Plugins.link('layerExImage.dll')").unwrap();
    assert_eq!(
        s.evaluate("Plugins.getList().count").unwrap(),
        Value::Integer(1)
    );
    s.evaluate("Scripts.exec('global.w=new Window();global.l=new Layer(w,null);')")
        .unwrap();
    let error = s.evaluate("l.light(0,0)").unwrap_err();
    assert!(format!("{error:#}").contains("unsupported Layer operation: light"));
}
