use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
#[test]
fn original_video_overlay_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/video_overlay.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let mut session =
            Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        assert_eq!(
            session
                .execute_storage("case.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn bad_owners_and_unavailable_movie_backend_are_explicit_errors() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("movie.mpg"),
        b"synthetic unsupported movie",
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    assert!(session.evaluate("new VideoOverlay(null)").is_err());
    assert!(session.evaluate("new VideoOverlay(%[])").is_err());
    session
        .evaluate("Scripts.exec('global.w=new Window();global.v=new VideoOverlay(w);')")
        .unwrap();
    let missing = session.evaluate("v.open('missing.mpg')").unwrap_err();
    assert!(missing.downcast_ref::<krkrz_tjs::VmAbort>().is_none());
    let unavailable = session.evaluate("v.open('movie.mpg')").unwrap_err();
    assert!(unavailable.downcast_ref::<krkrz_tjs::VmAbort>().is_some());
    session.evaluate("invalidate v").unwrap();
    assert!(session.evaluate("v.play()").is_err());
}

#[test]
fn invalid_zoom_and_missing_layer_coordinates_do_not_panic() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    session
        .evaluate(
            "Scripts.exec('global.w=new Window();global.v=new VideoOverlay(w);v.mode=vomLayer;')",
        )
        .unwrap();
    assert!(session.evaluate("v.setPos()").is_err());
    assert!(session.evaluate("v.setPos(1)").is_err());
    assert!(session.evaluate("w.setZoom(0,0)").is_err());
    assert_eq!(session.evaluate("w.zoomNumer").unwrap(), Value::Integer(1));
    session.evaluate("w.setZoom(-2147483648,-1)").unwrap();
    assert_eq!(session.evaluate("w.zoomDenom").unwrap(), Value::Integer(1));
}

fn movie_session() -> (Session, tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("movie.mpg"),
        include_bytes!("fixtures/video_overlay.mpg"),
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000_000).unwrap();
    session.evaluate("Scripts.exec('global.w=new Window();global.l=new Layer(w,null);l.setSize(32,24);global.v=new VideoOverlay(w);v.mode=vomLayer;v.layer1=l;global.events=[];v.onStatusChanged=function(s){events.add(s);};v.onPeriod=function(r){events.add(\"period\"+r);};v.open(\"movie.mpg\");')").unwrap();
    (session, project, saves)
}

#[test]
fn overlay_frames_clip_to_window_bounds_and_respect_visibility() {
    let (mut session, _project, _saves) = movie_session();
    session.evaluate("Scripts.exec('v.close();v.mode=vomOverlay;v.open(\"movie.mpg\");l.fillRect(0,0,32,24,0xff00ff00);v.setBounds(-2,5,16,12);v.visible=true;')").unwrap();
    let window = session.evaluate("w").unwrap();
    let image = session.capture_window(&window).unwrap();
    let pixel = |x: usize, y: usize| &image.rgba[(y * 32 + x) * 4..(y * 32 + x + 1) * 4];
    assert_eq!(pixel(0, 4), &[0, 255, 0, 255]);
    assert!(pixel(0, 5)[0] > 240 && pixel(0, 5)[1] < 16);
    assert!(pixel(13, 16)[0] > 240);
    assert_eq!(pixel(14, 16), &[0, 255, 0, 255]);
    assert_eq!(pixel(13, 17), &[0, 255, 0, 255]);
    session.evaluate("v.visible=false").unwrap();
    let hidden = session.capture_window(&window).unwrap();
    assert_eq!(
        &hidden.rgba[(5 * 32) * 4..(5 * 32 + 1) * 4],
        &[0, 255, 0, 255]
    );
}

#[test]
fn movie_frames_pcm_pause_seek_and_eof_use_session_clock() {
    let (mut session, _project, _saves) = movie_session();
    assert_eq!(
        session
            .evaluate("v.originalWidth+\",\"+v.originalHeight+\",\"+v.fps")
            .unwrap(),
        Value::string("32,24,25")
    );
    let color = session
        .evaluate("l.getMainPixel(16,12)")
        .unwrap()
        .integer()
        .unwrap();
    assert!(
        (color & 0xff0000) > 0xf00000,
        "decoded red pixel: {color:x}"
    );
    assert!((color & 0x00ffff) < 0x1000, "decoded red pixel: {color:x}");
    session.evaluate("v.play()").unwrap();
    session.tick(120).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(3));
    assert!(session.take_audio().iter().any(|v| v.abs() > 0.01));
    session.evaluate("v.pause()").unwrap();
    session.tick(240).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(3));
    assert!(session.take_audio().is_empty());
    session.evaluate("v.frame=1").unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(1));
    session.evaluate("v.play()").unwrap();
    session.tick(800).unwrap();
    assert_eq!(
        session.evaluate("events.join(\",\")").unwrap(),
        Value::string("stop,play,pause,play,stop")
    );
    session.tick(840).unwrap();
    assert!(session.take_audio().is_empty());
    session.evaluate("v.close()").unwrap();
    session.dispatch_events().unwrap();
    assert_eq!(
        session.evaluate("events[events.count-1]").unwrap(),
        Value::string("unload")
    );
}
#[test]
fn movie_loop_and_period_callbacks_follow_frame_progress() {
    let (mut session, _project, _saves) = movie_session();
    session
        .evaluate("Scripts.exec('v.setPeriodEvent(2);v.loop=true;v.play();')")
        .unwrap();
    session.tick(120).unwrap();
    assert_eq!(
        session.evaluate("v.periodEventFrame").unwrap(),
        Value::Integer(-1)
    );
    session.tick(480).unwrap();
    assert_eq!(
        session.evaluate("events.join(\",\")").unwrap(),
        Value::string("stop,play,period1,period0")
    );
    session
        .evaluate("Scripts.exec('v.loop=false;v.setSegmentLoop(1,3);v.rewind();')")
        .unwrap();
    session.tick(640).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(2));
    assert_eq!(
        session.evaluate("events[events.count-1]").unwrap(),
        Value::string("period3")
    );
}

#[test]
#[ignore = "requires KRKRZ_MOVIE naming a locally extracted game movie"]
fn installed_movie_decodes_seeks_and_finishes() {
    let path = std::env::var_os("KRKRZ_MOVIE").expect("set KRKRZ_MOVIE");
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::copy(path, project.path().join("movie.mpg")).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 50_000_000).unwrap();
    session.evaluate("Scripts.exec('global.w=new Window();global.l=new Layer(w,null);l.setSize(1280,720);global.v=new VideoOverlay(w);v.mode=vomLayer;v.layer1=l;global.last=\"\";v.onStatusChanged=function(s){last=s;};v.open(\"movie.mpg\");v.play();')").unwrap();
    session.tick(100).unwrap();
    assert!(!session.take_audio().is_empty());
    let duration = session.evaluate("v.totalTime").unwrap().integer().unwrap() as u64;
    session
        .evaluate(&format!("v.position={}", duration / 2))
        .unwrap();
    session.tick(200).unwrap();
    session
        .evaluate(&format!("v.position={}", duration.saturating_sub(50)))
        .unwrap();
    session.tick(400).unwrap();
    assert_eq!(session.evaluate("last").unwrap(), Value::string("stop"));
}

#[test]
fn frame_change_detection_keeps_overlay_video_live_and_removes_hidden_frames() {
    let (mut session, _project, _saves) = movie_session();
    session.evaluate("Scripts.exec('v.close();v.mode=vomOverlay;v.open(\"movie.mpg\");l.fillRect(0,0,32,24,0xff00ff00);v.visible=true;v.play();')").unwrap();
    let window = session.evaluate("w").unwrap();
    let mut state = krkrz_runtime::WindowFrameState::default();
    for time in [0, 120, 240] {
        session.tick(time).unwrap();
        let image = session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .unwrap();
        assert_eq!(image.rgba, session.capture_window(&window).unwrap().rgba);
    }
    session.evaluate("v.visible=false").unwrap();
    let image = session
        .capture_window_if_changed(&window, &mut state)
        .unwrap()
        .unwrap();
    assert_eq!(image.rgba, [0, 255, 0, 255].repeat(32 * 24));
    assert!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .is_none()
    );
}
