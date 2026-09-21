use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn original_uuid_format_and_unique_identifiers() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/system_uuid.json")).unwrap();
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    for case in cases.as_array().unwrap() {
        let program = krkrz_tjs::compile("uuid", case["source"].as_str().unwrap()).unwrap();
        let result = session
            .vm
            .execute(&program, &mut session.services, &mut session.budget)
            .unwrap();
        assert_eq!(result, Value::Integer(1));
    }
}
