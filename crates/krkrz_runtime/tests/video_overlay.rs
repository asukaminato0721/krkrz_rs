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
