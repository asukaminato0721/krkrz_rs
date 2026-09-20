use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
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
fn original_draw_device_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/draw_device.json")).unwrap();
    for case in cases {
        let (project, saves, mut session) = session("", 100_000);
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
fn device_recreation_and_replacement_invalidate_host_resources() {
    let (_project, _saves, mut session) = session("global.w=new Window();", 100_000);
    let w = session.evaluate("w").unwrap();
    let before = session.draw_device_epoch(&w).unwrap().unwrap();
    session.evaluate("w.drawDevice.recreate()").unwrap();
    let recreated = session.draw_device_epoch(&w).unwrap().unwrap();
    assert_eq!(before.0, recreated.0);
    assert_ne!(before.1, recreated.1);
    session
        .evaluate("w.drawDevice=new Window.BasicDrawDevice()")
        .unwrap();
    assert_ne!(
        recreated.0,
        session.draw_device_epoch(&w).unwrap().unwrap().0
    );
    session.evaluate("invalidate w").unwrap();
    assert!(session.draw_device_epoch(&w).is_err());
}
