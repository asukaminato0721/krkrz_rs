use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
    #[serde(default)]
    budget: Option<u64>,
}
fn session(source: &str, budget: u64) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, budget).unwrap();
    session.execute_storage("case.tjs").unwrap();
    (project, saves, session)
}
#[test]
fn original_layer_focus_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_focus.json")).unwrap();
    for case in cases {
        let (project, saves, mut session) = session("", case.budget.unwrap_or(100_000));
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

#[test]
fn original_modal_layer_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_modal.json")).unwrap();
    for case in cases {
        let (project, _saves, mut session) = session("", 100_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        assert_eq!(
            session
                .execute_storage("test.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name,
        );
    }
}

#[test]
fn modal_focus_callbacks_can_invalidate_the_dialog() {
    for action in [
        "a.onBeforeFocus=function(){invalidate a;};a.setMode();",
        "a.setMode();a.onSearchNextFocusable=function(){invalidate a;};a.removeMode();",
    ] {
        let (_project, _saves, mut session) = session(
            "var w=new Window(),p=new Layer(w,null),a=new Layer(w,p),b=new Layer(w,p);a.visible=b.visible=a.focusable=b.focusable=true;",
            100_000,
        );
        session
            .evaluate(&format!("Scripts.exec('{}')", action))
            .unwrap();
        assert_eq!(
            session
                .evaluate("p.nodeEnabled && b.nodeEnabled && w.focusedLayer===null")
                .unwrap(),
            Value::Integer(1)
        );
        session.collect_garbage(&[]).unwrap();
    }
}

#[test]
fn invalidation_during_focus_callbacks_does_not_leave_stale_focus() {
    for event in ["onBeforeFocus", "onFocus"] {
        let (_project, _saves, mut session) = session(
            "global.w=new Window();global.p=new Layer(w,null);global.a=new Layer(w,p);a.visible=a.focusable=true;",
            100_000,
        );
        session
            .evaluate(&format!(
                "Scripts.exec('a.{event}=function(){{invalidate a;}};a.focus();')"
            ))
            .unwrap();
        assert_eq!(
            session.evaluate("w.focusedLayer===null").unwrap(),
            Value::Integer(1)
        );
    }
}

#[test]
fn exceptions_release_focus_lock_and_reparent_callbacks_cannot_panic() {
    let (_project, _saves, mut session) = session(
        "global.w=new Window();global.p=new Layer(w,null);global.a=new Layer(w,p);global.b=new Layer(w,p);a.visible=b.visible=a.focusable=b.focusable=true;",
        100_000,
    );
    assert!(
        session
            .evaluate("Scripts.exec('a.onFocus=function(){throw 7;};a.focus();')")
            .is_err()
    );
    assert_eq!(session.evaluate("b.focus()").unwrap(), Value::Integer(1));
    session.evaluate("Scripts.exec('a.onFocus=function(){};a.focus();b.onBeforeFocus=function(){invalidate a;};')").unwrap();
    let error = session.evaluate("a.parent=p").unwrap_err();
    assert!(format!("{error:#}").contains("invalidated"), "{error:#}");
    assert_eq!(session.evaluate("b.focused").unwrap(), Value::Integer(1));
}

#[test]
fn focus_observation_properties_are_read_only() {
    let (_project, _saves, mut session) =
        session("global.w=new Window();global.a=new Layer(w,null);", 100_000);
    for name in [
        "focused",
        "nodeFocusable",
        "nodeEnabled",
        "nextFocusable",
        "prevFocusable",
    ] {
        assert!(session.evaluate(&format!("a.{name}=0")).is_err(), "{name}");
    }
}
