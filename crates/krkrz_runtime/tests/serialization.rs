use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
    files: Vec<String>,
}

fn run(project: &Path, saves: &Path, source: &str) -> anyhow::Result<Value> {
    std::fs::write(project.join("case.tjs"), source)?;
    Session::open(project, Some(saves), false, 10_000)?.execute_storage("case.tjs")
}

#[test]
fn original_serialization_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/serialization.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        let result = run(project.path(), saves.path(), &case.source)
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
        for file in case.files {
            assert!(saves.path().join(&file).is_file(), "{}: {file}", case.name);
            assert!(
                !project.path().join(&file).exists(),
                "{}: {file}",
                case.name
            );
        }
    }
}

#[test]
fn save_reload_in_fresh_session_and_overlay_precedence() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let installed = b"[42]";
    std::fs::write(project.path().join("branch.tjs"), installed).unwrap();
    let write = r#"
var state=%["scene"=>"next", "choice"=>0, "flags"=>[1,2], "volume"=>0.1];
(Dictionary.saveStruct incontextof state)(System.dataPath+"slot/one.tjs", "z");
[7].saveStruct(System.exePath+"branch.tjs", "c");
return Scripts.evalStorage("branch.tjs")[0];
"#;
    assert_eq!(
        run(project.path(), saves.path(), write).unwrap(),
        Value::Integer(7)
    );
    assert_eq!(
        std::fs::read(project.path().join("branch.tjs")).unwrap(),
        installed
    );
    let read = r#"
var state=Scripts.evalStorage(System.dataPath+"slot/one.tjs");
return state.scene+","+state.choice+","+state.flags[1]+","+state.volume;
"#;
    assert_eq!(
        run(project.path(), saves.path(), read).unwrap(),
        Value::string("next,0,2,0.1")
    );
}

#[test]
fn invalid_serialization_does_not_replace_existing_save() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(saves.path().join("keep.txt"), b"keep").unwrap();
    for source in [
        "Plugins.link('saveStruct.dll');var a=[];a.add(a);a.saveStruct2('keep.txt');",
        "Plugins.link('saveStruct.dll');['okay',1].save2('keep.txt');",
        "[1].saveStruct('../keep.txt');",
        "[1].saveStruct('data.xp3>keep.txt');",
    ] {
        assert!(run(project.path(), saves.path(), source).is_err());
        assert_eq!(
            std::fs::read(saves.path().join("keep.txt")).unwrap(),
            b"keep"
        );
    }
    let error = run(project.path(), saves.path(),
        "Plugins.link('saveStruct.dll');var a=[];a.add(a);try {return a.toStructString();} catch {return 7;}")
        .unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_redirect_save_writes() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), saves.path().join("escape")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("keep.txt"),
        saves.path().join("file.txt"),
    )
    .unwrap();
    for target in ["escape/keep.txt", "escape/new.txt", "file.txt"] {
        let source = format!("[1].saveStruct('{target}');");
        assert!(run(project.path(), saves.path(), &source).is_err());
    }
    assert_eq!(
        std::fs::read(outside.path().join("keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!outside.path().join("new.txt").exists());
}
