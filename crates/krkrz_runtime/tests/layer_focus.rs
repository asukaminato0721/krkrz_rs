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
