#![cfg(unix)]

use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn local_names_convert_directories_and_files_without_storage_lookup() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("GameDir");
    let saves = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.join("AutoPath")).unwrap();
    let project = project.canonicalize().unwrap();
    let save_path = saves.path().canonicalize().unwrap();
    std::fs::write(project.join("AutoPath/Found.TJS"), "42").unwrap();
    let mut session = Session::open(&project, Some(saves.path()), None, 100_000).unwrap();
    session
        .evaluate("Storages.addAutoPath('AutoPath/')")
        .unwrap();
    for (expression, expected) in [
        ("System.exePath", format!("{}/", project.display())),
        ("'./'", format!("{}/", project.display())),
        ("'AUTOPATH/'", format!("{}/AutoPath/", project.display())),
        (
            "'autoPATH/found.tjs'",
            format!("{}/AutoPath/Found.TJS", project.display()),
        ),
        ("'Found.TJS'", format!("{}/found.tjs", project.display())),
        (
            "'A/../Missing/New.TJS'",
            format!("{}/missing/new.tjs", project.display()),
        ),
        ("'file://./'", "/".into()),
        ("System.dataPath", format!("{}/", save_path.display())),
        (
            "Storages.getFullPath(System.exePath+'AutoPath/Found.TJS')",
            format!("{}/AutoPath/Found.TJS", project.display()),
        ),
        ("'A%20B.TJS'", format!("{}/a%20b.tjs", project.display())),
    ] {
        assert_eq!(
            session
                .evaluate(&format!("Storages.getLocalName({expression})"))
                .unwrap(),
            Value::string(&expected),
            "{expression}"
        );
    }
    for expression in [
        "Storages.getLocalName()",
        "Storages.getLocalName('')",
        "Storages.getLocalName('data.xp3>script.tjs')",
        "Storages.getLocalName('http://example.invalid/a')",
        "Storages.getLocalName('a'+$0+'z')",
    ] {
        assert!(session.evaluate(expression).is_err(), "{expression}");
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::write(project.join("Case.TJS"), "1").unwrap();
        std::fs::write(project.join("case.tjs"), "2").unwrap();
        assert!(
            session
                .evaluate("Storages.getLocalName('case.tjs')")
                .is_err()
        );
    }
}
