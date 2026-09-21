use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

#[test]
fn original_plugin_corpus() {
    let mut cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/scripts_ex.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("fixtures/array_reflection.json")).unwrap(),
    );
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    for case in cases {
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000).unwrap();
        let result = session
            .execute_storage("case.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}

#[test]
fn cyclic_structures_and_missing_plugin_operations_fail_explicitly() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    for source in [
        "var a=[];a.add(a);Scripts.clone(a);",
        "var a=[],b=[];a.add(a);b.add(b);Scripts.equalStruct(a,b);",
        "try { Scripts.getMD5HashString('x'); } catch { return 42; }",
    ] {
        std::fs::write(
            project.path().join("case.tjs"),
            format!("Plugins.link('ScriptsEx.dll');{source}"),
        )
        .unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000).unwrap();
        let error = session.execute_storage("case.tjs").unwrap_err();
        assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    }
}

#[test]
#[ignore = "requires KRKRZ_TEST_PROJECT with KAGEnvironment.tjs"]
fn installed_image_file_data_accepts_array_options_and_redraw() {
    let project = std::env::var_os("KRKRZ_TEST_PROJECT").expect("KRKRZ_TEST_PROJECT");
    let mut storage = krkrz_assets::storage::Storage::open(
        std::path::Path::new(&project),
        None,
        Default::default(),
    )
    .unwrap();
    storage.detect_cipher().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::from_storage(storage, Some(saves.path()), None, 100_000_000).unwrap();
    s.startup().unwrap();
    s.evaluate("global.reflectionProbe=(KAGEnvironment.getImageFileData incontextof global)('test.png',[],[])").unwrap();
    assert_eq!(
        s.evaluate("reflectionProbe.file").unwrap(),
        Value::string("test.png")
    );
    assert_eq!(s.evaluate("(reflectionProbe.options instanceof 'Array') && (reflectionProbe.redraw instanceof 'Array') && reflectionProbe.options.count==0 && reflectionProbe.redraw.count==0").unwrap(), Value::Integer(1));
}
