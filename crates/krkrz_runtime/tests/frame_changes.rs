use krkrz_runtime::{Session, WindowFrameState};
use krkrz_tjs::Value;

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session, Value) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000_000).unwrap();
    session.evaluate("Scripts.exec('global.w=new Window();global.p=new Layer(w,null);p.setSize(8,8);p.setImageSize(8,8);global.a=new Layer(w,p);a.setSize(4,4);a.setImageSize(4,4);a.fillRect(0,0,4,4,0xffff0000);a.visible=true;global.b=new Layer(w,p);b.setSize(4,4);b.setImageSize(4,4);b.fillRect(0,0,4,4,0xff0000ff);b.visible=true;')").unwrap();
    let window = session.evaluate("w").unwrap();
    (project, saves, session, window)
}

#[test]
fn unchanged_frames_skip_pixels_but_every_visual_mutation_matches_full_capture() {
    let (_project, _saves, mut session, window) = session();
    let mut state = WindowFrameState::default();
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_some()
    );
    let budget = session.budget;
    for tick in 0..20 {
        session.tick(tick * 16).unwrap();
        assert!(
            session
                .capture_window_if_changed(&window, &mut state)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        budget - session.budget < 1000,
        "idle ticks must not charge pixel composition"
    );
    for code in [
        "b.setMainPixel(0,0,0x00ff00)",
        "b.opacity=128",
        "b.left=1",
        "b.top=1",
        "b.visible=false",
        "b.visible=true",
        "b.setImageSize(6,6)",
        "b.imageLeft=-1",
        "b.imageTop=-1",
        "b.type=ltOpaque",
        "b.setSize(3,3)",
        "b.bringToBack()",
        "b.parent=a",
        "b.parent=p",
        "b.assignImages(a)",
        "b.fillRect(0,0,1,1,0xffffffff)",
        "p.setSize(9,9)",
        "invalidate b",
        "w.setZoom(2,1)",
    ] {
        session
            .evaluate(code)
            .unwrap_or_else(|e| panic!("{code}: {e:#}"));
        let image = session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .unwrap_or_else(|| panic!("missed mutation: {code}"));
        let reference = session.capture_window(&window).unwrap();
        assert_eq!(image.rgba, reference.rgba, "{code}");
        assert_eq!(
            (image.width, image.height),
            (reference.width, reference.height)
        );
        assert!(
            session
                .capture_window_if_changed(&window, &mut state)
                .unwrap()
                .is_none(),
            "{code}"
        );
    }
}

#[test]
fn paint_callbacks_run_before_change_detection() {
    let (_project, _saves, mut session, window) = session();
    let mut state = WindowFrameState::default();
    session
        .capture_window_if_changed(&window, &mut state)
        .unwrap();
    session.evaluate("Scripts.exec('global.paints=0;b.onPaint=function(){global.paints++;this.fillRect(0,0,4,4,0xff00ff00);};b.update();')").unwrap();
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_some()
    );
    assert_eq!(session.evaluate("paints").unwrap(), Value::Integer(1));
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_none()
    );
    assert_eq!(session.evaluate("paints").unwrap(), Value::Integer(1));
}

#[test]
fn transitions_advance_and_completion_is_followed_by_a_final_static_frame() {
    let (_project, _saves, mut session, window) = session();
    session.evaluate("Scripts.exec('b.visible=false;global.clock=0;a.beginTransition(\"crossfade\",true,b,%[time:100,selfupdate:true,callback:function(){return global.clock;}]);')").unwrap();
    let mut state = WindowFrameState::default();
    let mut frames = Vec::new();
    for time in [0, 50, 100] {
        session.evaluate(&format!("clock={time}")).unwrap();
        frames.push(
            session
                .capture_window_if_changed(&window, &mut state)
                .unwrap()
                .unwrap()
                .rgba,
        );
    }
    assert_ne!(frames[0], frames[1]);
    assert_ne!(frames[1], frames[2]);
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_some()
    );
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_none()
    );
}
