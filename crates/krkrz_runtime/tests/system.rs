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
fn original_system_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/system.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let result = Session::open(project.path(), Some(saves.path()), None, 10_000)
            .unwrap()
            .execute_storage("case.tjs")
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}

#[test]
fn synchronous_exit_bypasses_script_handlers_and_preserves_the_status() {
    for (code, source) in [
        (0, "global.marker=0;System.exit();marker=1;"),
        (
            7,
            "global.marker=0;try{Scripts.exec('System.exit(7);');marker=1;}catch(e){marker=2;}marker=3;",
        ),
    ] {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000).unwrap();
        let error = session.execute_storage("case.tjs").unwrap_err();
        assert_eq!(error.downcast_ref::<krkrz_tjs::VmExit>().unwrap().0, code);
        assert_eq!(session.services.exit_code, Some(code));
        assert_eq!(session.evaluate("marker").unwrap(), Value::Integer(0));
    }
}

#[test]
fn missing_executable_metadata_preserves_the_default_version() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000).unwrap();
    assert_eq!(
        session.evaluate("System.versionString").unwrap(),
        Value::string("1.2.0.3")
    );
    assert_eq!(
        session
            .evaluate("System.versionInformation.substr(11,1)")
            .unwrap(),
        Value::string("Z")
    );
    assert!(session.evaluate("System.versionInformation.indexOf(System.versionString)>=0 && System.versionInformation.indexOf('krkrz_rs')>=0").unwrap().truth().unwrap());
}

#[test]
#[ignore = "requires the installed Yuri game and KRKRZ_PROJECT_DIR"]
fn installed_engine_version_passes_the_original_startup_guard() {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(
        std::path::Path::new(&project),
        Some(saves.path()),
        None,
        1_000_000_000,
    )
    .unwrap();
    assert_eq!(
        session.evaluate("System.versionString").unwrap(),
        Value::string("2.11.19.13")
    );
    // Use the original guard without starting unrelated game components.
    assert_eq!(
        session
            .evaluate(
                "Scripts.exec('if(System.versionString!=\"2.11.19.13\"){System.exit();}return 1;')"
            )
            .unwrap(),
        Value::Integer(1)
    );
    assert_eq!(session.services.exit_code, None);
}
