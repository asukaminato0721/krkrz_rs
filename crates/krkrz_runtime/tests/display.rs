use krkrz_runtime::{Session, display::Monitor};
use krkrz_tjs::Value;
const SETUP: &str = "Plugins.link('menu.dll');if(typeof global.Pad=='undefined')global.Pad=%[];if(typeof Debug.console=='undefined')Debug.console=%[];Plugins.link('windowEx.dll');";
fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    std::fs::write(project.path().join("setup.tjs"), SETUP).unwrap();
    session.execute_storage("setup.tjs").unwrap();
    (project, saves, session)
}
#[test]
fn original_monitor_query_corpus() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/display.json")).unwrap();
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
#[test]
fn monitors_select_by_overlap_distance_and_main_window_location() {
    let (_project, _saves, mut session) = session();
    session
        .services
        .set_displays(vec![
            Monitor {
                name: "left".into(),
                primary: false,
                bounds: [-800, 0, 800, 600],
                work: [-800, 20, 800, 580],
            },
            Monitor {
                name: "main".into(),
                primary: true,
                bounds: [0, 0, 1280, 720],
                work: [0, 0, 1280, 680],
            },
        ])
        .unwrap();
    assert_eq!(
        session.evaluate("System.getMonitorInfo().name").unwrap(),
        Value::string("main")
    );
    assert_eq!(
        session
            .evaluate("System.getMonitorInfo(false,-100,10,300,200).name")
            .unwrap(),
        Value::string("main")
    );
    assert_eq!(
        session
            .evaluate("System.getMonitorInfo(false,-100,10,120,200).name")
            .unwrap(),
        Value::string("left")
    );
    assert_eq!(
        session
            .evaluate("System.getMonitorInfo(true,-900,10).name")
            .unwrap(),
        Value::string("left")
    );
    assert_eq!(
        session
            .evaluate("System.getMonitorInfo(false,-900,10)")
            .unwrap(),
        Value::Void
    );
    assert_eq!(
        session
            .evaluate("System.getDisplayMonitors(-30,5,70,10)[0].intersect.w")
            .unwrap(),
        Value::Integer(30)
    );
    session
        .evaluate("Scripts.exec('global.w=new Window();w.setSize(320,240);w.setPos(-700,40);')")
        .unwrap();
    assert_eq!(session.evaluate("[System.screenWidth,System.screenHeight,System.desktopLeft,System.desktopTop,System.desktopWidth,System.desktopHeight].join(',')").unwrap(), Value::string("800,600,-800,20,800,580"));
    session.evaluate("w.fullScreen=true").unwrap();
    assert_eq!(
        session
            .evaluate("[w.left,w.top,w.width,w.height].join(',')")
            .unwrap(),
        Value::string("-800,0,800,600")
    );
    session.evaluate("w.fullScreen=false").unwrap();
    assert_eq!(
        session
            .evaluate("[w.left,w.top,w.width,w.height].join(',')")
            .unwrap(),
        Value::string("-700,40,320,240")
    );
    session.evaluate("w.setPos(100,20)").unwrap();
    assert_eq!(
        session.evaluate("System.desktopHeight").unwrap(),
        Value::Integer(680)
    );
}
#[test]
fn query_arguments_and_host_layout_are_validated() {
    let (_project, _saves, mut session) = session();
    for expression in [
        "System.getDisplayMonitors(1)",
        "System.getMonitorInfo(1)",
        "System.getMonitorInfo(1,null)",
        "System.getMonitorInfo(1,%[])",
    ] {
        assert!(session.evaluate(expression).is_err(), "{expression}");
    }
    assert!(session.services.set_displays(Vec::new()).is_err());
    assert!(
        session
            .services
            .set_displays(vec![Monitor {
                name: "invalid".into(),
                primary: true,
                bounds: [0, 0, 100, 100],
                work: [0, 0, 200, 100]
            }])
            .is_err()
    );
    assert_eq!(
        session.evaluate("System.screenWidth").unwrap(),
        Value::Integer(1280)
    );
}
#[test]
fn client_and_outer_dimensions_account_for_host_decorations() {
    let (_project, _saves, mut session) = session();
    let window = session.evaluate("global.w=new Window()").unwrap();
    let Value::Object(reference) = window else {
        panic!("expected window")
    };
    session
        .services
        .windows
        .get_mut(&reference.object.unwrap())
        .unwrap()
        .frame_insets = [4, 24, 4, 4];
    session.evaluate("w.setPos(30,40)").unwrap();
    session.evaluate("w.setSize(320,240)").unwrap();
    assert_eq!(session.evaluate("[w.width,w.height,w.innerWidth,w.innerHeight,w.getClientRect().x,w.getClientRect().y].join(',')").unwrap(), Value::string("320,240,312,212,34,64"));
    session.evaluate("w.width=640").unwrap();
    assert_eq!(
        session.evaluate("w.innerWidth").unwrap(),
        Value::Integer(632)
    );
    session.evaluate("w.fullScreen=true").unwrap();
    assert_eq!(
        session
            .evaluate("w.getClientRect().x==w.getWindowRect().x")
            .unwrap(),
        Value::Integer(1)
    );
}
