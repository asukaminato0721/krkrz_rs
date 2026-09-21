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
    Session::open(project.path(), Some(saves.path()), None, budget)?.execute_storage("case.tjs")
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
fn unfinished_movie_operations_cannot_be_hidden_by_script_catch() {
    {
        let operation = "a.setNextMovieFile('empty.amv')";
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

#[test]
fn decoded_frame_updates_layer_pixels_geometry_and_transport() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("dc.amv"),
        include_bytes!("../../krkrz_assets/tests/fixtures/amv/dc.amv"),
    )
    .unwrap();
    std::fs::write(project.path().join("case.tjs"),
        "Plugins.link('AlphaMovie.dll');var w=new Window();var primary=new Layer(w,null);var l=new Layer(w,primary);var a=new AlphaMovie();a.open(System.exePath+'dc.amv');a.setPosition(7,-3);a.play();var n=a.showNextImage(l);var result=[n,l.left,l.top,l.width,l.height,l.imageWidth,l.imageHeight,l.getMainPixel(0,0),l.getMaskPixel(0,0),l.getMainPixel(8,8),l.getMaskPixel(8,8)];a.stop();result.add(a.showNextImage(l));return result.join('|');").unwrap();
    let result = Session::open(project.path(), Some(saves.path()), None, 100_000)
        .unwrap()
        .execute_storage("case.tjs")
        .unwrap();
    assert_eq!(
        result,
        Value::string("0|7|-3|16|16|16|16|8880517|153|9867412|180|0")
    );
}

#[test]
fn packet_sequences_loop_before_padding_and_seek_to_requested_frame() {
    let source = "Plugins.link('AlphaMovie.dll');var w=new Window();var l=new Layer(w,null);var a=new AlphaMovie();a.open(System.exePath+'empty.amv');a.play();var r=[];for(var i=0;i<6;i++)r.add(a.showNextImage(l));a.frame=2;r.add(a.showNextImage(l));a.frame=0;r.add(a.showNextImage(l));a.loop=false;a.frame=2;r.add(a.showNextImage(l));r.add(a.showNextImage(l));return r.join('|');";
    assert_eq!(
        execute(source, 100_000).unwrap(),
        Value::string("0|0|1|2|0|0|2|0|2|2")
    );
}
