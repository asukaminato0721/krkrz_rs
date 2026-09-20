use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn execute(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(
        project.path().join("empty.amv"),
        include_bytes!("fixtures/empty.amv"),
    )?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_alpha_movie_metadata_and_transport_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/alpha_movie.json")).unwrap();
    for c in cases {
        assert_eq!(
            execute(&c.source, 100_000).unwrap_or_else(|e| panic!("{}: {e:#}", c.name)),
            c.expected,
            "{}",
            c.name
        );
    }
}
#[test]
fn unfinished_decoder_cannot_be_hidden_by_script_catch() {
    for operation in [
        "a.showNextImage(null)",
        "a.frame=1",
        "a.setNextMovieFile('empty.amv')",
        "a.setPosition(1,2)",
    ] {
        let source = format!(
            "Plugins.link('AlphaMovie.dll');var a=new AlphaMovie();a.open(System.exePath+'empty.amv');try{{{operation};}}catch(e){{return 999;}}"
        );
        let error = execute(&source, 100_000).unwrap_err();
        assert!(
            error.downcast_ref::<VmAbort>().is_some(),
            "{operation}: {error:#}"
        );
    }
}
#[test]
fn open_obeys_session_budget() {
    let source = "Plugins.link('AlphaMovie.dll');var a=new AlphaMovie();try{a.open(System.exePath+'empty.amv');}catch(e){return 999;}";
    let error = execute(source, 45).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert!(
        format!("{error:#}").contains("AlphaMovie open execution budget"),
        "{error:#}"
    );
}
