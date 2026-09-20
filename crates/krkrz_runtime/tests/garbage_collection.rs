use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn session(source: &str) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("startup.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000_000).unwrap();
    session.startup().unwrap();
    (project, saves, session)
}

#[test]
fn native_arrays_parsers_and_callback_owners_keep_references_alive() {
    let (_project, _saves, mut session) = session(
        r#"
        global.w = new Window(); w.visible = true;
        function build() {
            global.p = new Layer(w, null); p.setSize(20, 10); p.visible = true;
            var child = new Layer(w, p); child.name = 'kept'; child.visible = true;
            var children = p.children;
        }
        build();
        Plugins.link('KAGParserEx.dll'); global.parser = new KAGParser();
        parser.onScenarioLoad = function() { return 'AB'; }; parser.loadScenario('synthetic');
        global.events = '';
        global.trigger = new AsyncTrigger(function() { global.events += 'async'; }, '');
        trigger.trigger(); trigger = null;
    "#,
    );
    session.collect_garbage(&[]).unwrap();
    assert_eq!(
        session.evaluate("w.primaryLayer.children[0].name").unwrap(),
        Value::string("kept")
    );
    assert_eq!(
        session.evaluate("parser.getNextTag().text").unwrap(),
        Value::string("A")
    );
    session.tick(1).unwrap();
    assert_eq!(session.evaluate("events").unwrap(), Value::string("async"));
    session.collect_garbage(&[]).unwrap();
    assert_eq!(
        session.evaluate("parser.getNextTag().text").unwrap(),
        Value::string("B")
    );
}

#[test]
fn unreferenced_native_instances_finalize_and_transient_allocations_are_reclaimed() {
    let (_project, _saves, mut session) = session(
        r#"
        global.finalized = 0;
        class T extends Timer {
            function T() { super.Timer(null); }
            function finalize() { global.finalized++; }
        }
        function allocate() { var t = new T(); t.interval=10; t.enabled=true; }
        allocate();
    "#,
    );
    let before = session.vm.live_objects();
    assert!(session.collect_garbage(&[]).unwrap() > 0);
    assert!(session.vm.live_objects() < before);
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(1));
    session.collect_garbage(&[]).unwrap();
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(1));
    let baseline = session.vm.live_objects();
    for _ in 0..5 {
        session
            .evaluate("Scripts.exec('for(var i=0;i<1000;i++){var a=[];a.add(i);}')")
            .unwrap();
        session.collect_garbage(&[]).unwrap();
        assert!(session.vm.live_objects() < baseline + 20);
    }
}

#[test]
fn explicit_external_roots_survive_and_referenced_timers_deliver() {
    let (_project, _saves, mut session) = session(
        r#"
        global.count = 0;
        function setup() {
            global.t = new Timer(function() { global.count++; }, '');
            t.interval = 10; t.enabled = true;
        }
        setup();
    "#,
    );
    let external = session.evaluate("%[answer:42]").unwrap();
    session
        .collect_garbage(std::slice::from_ref(&external))
        .unwrap();
    assert_eq!(
        session
            .vm
            .get_member(&external, &Value::string("answer"), false)
            .unwrap(),
        Value::Integer(42)
    );
    session.tick(11).unwrap();
    assert_eq!(session.evaluate("count").unwrap(), Value::Integer(1));
}

#[test]
fn window_registry_does_not_retain_abandoned_windows() {
    let (_project, _saves, mut session) = session(
        r#"
        global.finalized = 0;
        class W extends Window {
            function W() { super.Window(); }
            function finalize() { global.finalized++; }
        }
        global.window = new W();
    "#,
    );
    let window = session.evaluate("window").unwrap();
    session.evaluate("window = null").unwrap();
    session
        .collect_garbage(std::slice::from_ref(&window))
        .unwrap();
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(0));
    session.collect_garbage(&[]).unwrap();
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(1));
    assert_eq!(session.evaluate("Window.mainWindow").unwrap(), Value::NULL);
}
