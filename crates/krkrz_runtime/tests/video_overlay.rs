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
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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
    movie_session_with(include_bytes!("fixtures/video_overlay.mpg"))
}

fn movie_session_with(bytes: &[u8]) -> (Session, tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("movie.mpg"), bytes).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000_000).unwrap();
    session.evaluate("Scripts.exec('global.w=new Window();global.l=new Layer(w,null);l.setSize(32,24);global.v=new VideoOverlay(w);v.mode=vomLayer;v.layer1=l;global.events=[];v.onStatusChanged=function(s){events.add(s);};v.onPeriod=function(r){events.add(\"period\"+r);};v.open(\"movie.mpg\");')").unwrap();
    (session, project, saves)
}

#[test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
fn wmv_frames_audio_pause_rewind_loop_and_eof_follow_session_clock() {
    // Keep the misleading .mpg filename: detection must use ASF bytes.
    let (mut session, _project, _saves) =
        movie_session_with(include_bytes!("fixtures/video_overlay.wmv"));
    let window = session.evaluate("w").unwrap();
    let first = session.capture_window(&window).unwrap();
    assert_eq!(
        session
            .evaluate("v.originalWidth+','+v.originalHeight+','+v.fps")
            .unwrap(),
        Value::string("32,24,25")
    );
    session.evaluate("v.play()").unwrap();
    session.tick(160).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(4));
    assert!(session.take_audio().iter().any(|v| v.abs() > 0.01));
    assert_ne!(first.rgba, session.capture_window(&window).unwrap().rgba);
    session.evaluate("v.pause()").unwrap();
    session.tick(240).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(4));
    assert!(session.take_audio().is_empty());
    session.evaluate("v.frame=0").unwrap();
    assert_eq!(first.rgba, session.capture_window(&window).unwrap().rgba);
    session
        .evaluate("Scripts.exec('v.loop=true;v.play();')")
        .unwrap();
    let duration = session.evaluate("v.totalTime").unwrap().integer().unwrap() as u64;
    session.tick(240 + duration + 80).unwrap();
    assert_eq!(session.evaluate("v.frame").unwrap(), Value::Integer(2));
    assert_eq!(
        session.evaluate("events[events.count-1]").unwrap(),
        Value::string("period0")
    );
    session.evaluate("v.loop=false").unwrap();
    session.tick(240 + 2 * duration + 160).unwrap();
    assert_eq!(
        session.evaluate("events[events.count-1]").unwrap(),
        Value::string("stop")
    );
    session.evaluate("v.close()").unwrap();
}

#[test]
fn missing_wmv_tools_report_the_required_decoder() {
    const CHILD: &str = "KRKRZ_TEST_MISSING_WMV_TOOLS";
    if std::env::var_os(CHILD).is_some() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("movie.wmv"),
            include_bytes!("fixtures/video_overlay.wmv"),
        )
        .unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        session
            .evaluate("Scripts.exec('global.w=new Window();global.v=new VideoOverlay(w);')")
            .unwrap();
        let error = session.evaluate("v.open('movie.wmv')").unwrap_err();
        assert!(
            format!("{error:#}").contains("ffprobe must be installed on PATH"),
            "{error:#}"
        );
        return;
    }
    let empty = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "missing_wmv_tools_report_the_required_decoder"])
            .env(CHILD, "1")
            .env("PATH", empty.path())
            .status()
            .unwrap()
            .success()
    );
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
fn layer_movie_expands_placeholder_layers_to_the_decoded_frame_size() {
    let (mut session, _project, _saves) = movie_session();
    session.evaluate("Scripts.exec('v.close();v.layer1=null;l.setSize(64,48);l.fillRect(0,0,64,48,0xff00ff00);global.a=new Layer(w,l);global.b=new Layer(w,l);a.setSize(8,8);b.setSize(4,4);a.setPos(4,3);b.setPos(28,24);a.visible=b.visible=true;v.layer1=a;v.layer2=b;v.open(\"movie.mpg\");v.play();')").unwrap();
    session.tick(120).unwrap();
    assert_eq!(session.evaluate("[a.width,a.height,a.imageWidth,a.imageHeight,b.width,b.height,a.left,a.top,b.left,b.top].join(',')").unwrap(), Value::string("32,24,32,24,32,24,4,3,28,24"));
    let window = session.evaluate("w").unwrap();
    let image = session.capture_window(&window).unwrap();
    let pixel = |x: usize, y: usize| &image.rgba[(y * 64 + x) * 4..(y * 64 + x + 1) * 4];
    // Both movies extend beyond their placeholder bounds and keep their position.
    for (x, y) in [(20, 15), (59, 47)] {
        assert!(pixel(x, y)[0] > 240 && pixel(x, y)[1] < 16);
    }
    assert_eq!(pixel(3, 3), &[0, 255, 0, 255]);
    assert_eq!(pixel(60, 47), &[0, 255, 0, 255]);
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
    let mut session = Session::open(project.path(), Some(saves.path()), None, 50_000_000).unwrap();
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

#[test]
fn movie_playback_without_external_programs() {
    const CHILD: &str = "KRKRZ_TEST_BUILTIN_MOVIE";
    if std::env::var_os(CHILD).is_some() {
        movie_frames_pcm_pause_seek_and_eof_use_session_clock();
        movie_loop_and_period_callbacks_follow_frame_progress();
        overlay_frames_clip_to_window_bounds_and_respect_visibility();
        return;
    }
    let empty_path = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "movie_playback_without_external_programs",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("PATH", empty_path.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "built-in playback must work with an empty PATH"
    );
}
