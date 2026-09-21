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
    Session::open(project, Some(saves), None, 10_000)?.execute_storage("case.tjs")
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
// KAG considers a path absolute when it contains a storage scheme colon.
var path=System.dataPath;
if(path.indexOf(":")==-1)path=System.exePath+path;
(Dictionary.saveStruct incontextof state)(path+"slot/one.tjs", "z");
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
    assert!(saves.path().join("slot/one.tjs").is_file());
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
fn default_startup_reads_and_updates_existing_game_saves() {
    let project = tempfile::tempdir().unwrap();
    let savedata = project.path().join("savedata");
    std::fs::create_dir(&savedata).unwrap();
    std::fs::write(savedata.join("data0.ksd"), b"[42]").unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        "return Scripts.evalStorage(System.dataPath+'data0.ksd')[0];",
    )
    .unwrap();
    let mut session = Session::open(project.path(), None, None, 10_000).unwrap();
    assert_eq!(session.services.save_dir, savedata.canonicalize().unwrap());
    assert_eq!(session.startup().unwrap(), Value::Integer(42));
    session
        .evaluate("[43].saveStruct(System.dataPath+'data0.ksd')")
        .unwrap();
    let mut restarted = Session::open(project.path(), None, None, 10_000).unwrap();
    assert_eq!(restarted.startup().unwrap(), Value::Integer(43));

    let isolated = tempfile::tempdir().unwrap();
    std::fs::write(isolated.path().join("data0.ksd"), b"[99]").unwrap();
    let mut overridden =
        Session::open(project.path(), Some(isolated.path()), None, 10_000).unwrap();
    assert_eq!(overridden.startup().unwrap(), Value::Integer(99));
    overridden
        .evaluate("[100].saveStruct(System.dataPath+'data0.ksd')")
        .unwrap();
    assert_eq!(restarted.startup().unwrap(), Value::Integer(43));
}

#[cfg(unix)]
#[test]
fn default_save_writes_cannot_follow_symlinks_into_game_resources() {
    let project = tempfile::tempdir().unwrap();
    let savedata = project.path().join("savedata");
    std::fs::create_dir(&savedata).unwrap();
    std::fs::write(project.path().join("protected.tjs"), b"keep").unwrap();
    std::os::unix::fs::symlink(project.path(), savedata.join("escape")).unwrap();
    let mut session = Session::open(project.path(), None, None, 10_000).unwrap();
    assert!(
        session
            .evaluate("[1].saveStruct(System.dataPath+'escape/protected.tjs')")
            .is_err()
    );
    assert_eq!(
        std::fs::read(project.path().join("protected.tjs")).unwrap(),
        b"keep"
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
        for prefix in ["'".to_owned(), "System.dataPath+'".to_owned()] {
            let source = format!("[1].saveStruct({prefix}{target}');");
            assert!(run(project.path(), saves.path(), &source).is_err());
        }
    }
    assert_eq!(
        std::fs::read(outside.path().join("keep.txt")).unwrap(),
        b"keep"
    );
    assert!(!outside.path().join("new.txt").exists());
}
