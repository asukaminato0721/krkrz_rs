use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn original_fullscreen_control_corpus() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/fullscreen.json")).unwrap();
    for case in cases {
        let (project, _saves, mut session) = session();
        std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
        assert_eq!(
            session.execute_storage("case.tjs").unwrap(),
            case.expected,
            "{}",
            case.name
        );
    }
}

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    session.evaluate("Scripts.exec('global.a=new Window();global.b=new Window();a.setInnerSize(640,480);a.setPos(30,40);a.setZoom(3,2);b.setInnerSize(300,200);')").unwrap();
    (project, saves, session)
}
fn id(session: &mut Session, name: &str) -> usize {
    let Value::Object(reference) = session.evaluate(name).unwrap() else {
        panic!("expected Window")
    };
    reference.object.unwrap()
}

#[test]
fn fullscreen_fits_content_and_restores_window_geometry() {
    let (_project, _saves, mut session) = session();
    let id = id(&mut session, "a");
    assert_eq!(
        session.services.windows[&id].draw_rect(640, 480),
        [0, 0, 960, 720]
    );
    session.evaluate("a.fullScreen=true").unwrap();
    assert_eq!(
        session.services.windows[&id].draw_rect(640, 480),
        [160, 0, 960, 720]
    );
    assert_eq!(
        session
            .evaluate("[a.fullScreen,a.left,a.top,a.innerWidth,a.innerHeight,a.visible].join(',')")
            .unwrap(),
        Value::string("1,0,0,1280,720,1")
    );
    session.evaluate("a.setZoom(2,3)").unwrap();
    assert_eq!(
        session.services.windows[&id].draw_rect(640, 480),
        [160, 0, 960, 720]
    );
    // Logical zoom can change in full screen; it becomes effective on exit.
    session.evaluate("a.fullScreen=false").unwrap();
    assert_eq!(
        session.services.windows[&id].draw_rect(640, 480),
        [0, 0, 427, 320]
    );
    assert_eq!(
        session
            .evaluate("[a.fullScreen,a.left,a.top,a.innerWidth,a.innerHeight,a.visible].join(',')")
            .unwrap(),
        Value::string("0,30,40,640,480,1")
    );
}

#[test]
fn entering_another_window_restores_the_previous_fullscreen_window() {
    let (_project, _saves, mut session) = session();
    session.evaluate("a.fullScreen=true").unwrap();
    session.evaluate("a.fullScreen=true").unwrap();
    session.evaluate("b.fullScreen=true").unwrap();
    assert_eq!(
        session
            .evaluate(
                "[a.fullScreen,b.fullScreen,a.left,a.top,a.innerWidth,a.innerHeight].join(',')"
            )
            .unwrap(),
        Value::string("0,1,30,40,640,480")
    );
    session.evaluate("invalidate b").unwrap();
    session.evaluate("a.fullScreen=true").unwrap();
    session.evaluate("a.fullScreen=false").unwrap();
    assert_eq!(
        session.evaluate("a.innerHeight").unwrap(),
        Value::Integer(480)
    );
}

#[test]
fn invalid_display_does_not_displace_an_existing_fullscreen_window() {
    let (_project, _saves, mut session) = session();
    session.evaluate("a.fullScreen=true").unwrap();
    session.services.screen_size = (0, 720);
    assert!(session.evaluate("b.fullScreen=true").is_err());
    assert_eq!(session.evaluate("a.fullScreen").unwrap(), Value::Integer(1));
    assert_eq!(session.evaluate("b.fullScreen").unwrap(), Value::Integer(0));
}
