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
fn original_psb_file_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/psb_file.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("list.psb"),
            include_bytes!("fixtures/list.psb"),
        )
        .unwrap();
        std::fs::write(
            project.path().join("object.psb"),
            include_bytes!("fixtures/object.psb"),
        )
        .unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let result = Session::open(project.path(), Some(saves.path()), false, 100_000)
            .unwrap()
            .execute_storage("case.tjs")
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}
#[test]
fn invalidation_rejects_stale_views_without_crashing() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("list.psb"),
        include_bytes!("fixtures/list.psb"),
    )
    .unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
        Plugins.link("psbfile.dll");
        var p=new PSBFile("list.psb"), r=p.root, child=r[7];
        invalidate p;
        var errors=0;
        try{var x=r[4];}catch(e){errors++;}
        try{var x=child[0];}catch(e){errors++;}
        return errors;
    "#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10000).unwrap();
    assert_eq!(session.startup().unwrap(), Value::Integer(2));
}
