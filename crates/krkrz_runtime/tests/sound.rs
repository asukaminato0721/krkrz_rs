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
fn original_sound_control_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/sound.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let result = Session::open(project.path(), Some(saves.path()), false, 10_000)
            .unwrap()
            .execute_storage("case.tjs")
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}

#[test]
fn deterministic_fade_beats_and_invalidation() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
        var events="";
        class S extends WaveSoundBuffer {
            var label;
            function S(name){WaveSoundBuffer(null);label=name;}
            function onFadeCompleted(){events+=label+":"+volume+",";}
        }
        var a=new S("a"), b=new S("b");
        a.fade(0,180,1); b.fade(123,59,1);
    "#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000).unwrap();
    session.startup().unwrap();
    for (time, volume) in [
        (59, 100000),
        (60, 100000),
        (120, 66667),
        (180, 33334),
        (240, 0),
    ] {
        session.advance_clock(time).unwrap();
        assert_eq!(
            session.evaluate("a.volume").unwrap(),
            Value::Integer(volume)
        );
    }
    assert_eq!(session.evaluate("events").unwrap(), Value::string("a:0,"));
    // The target's delayed sub-beat fade does not complete on its own.
    session.advance_clock(10_000).unwrap();
    assert_eq!(
        session.evaluate("b.volume").unwrap(),
        Value::Integer(100000)
    );
    session.evaluate("b.stopFade()").unwrap();
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("a:0,b:123,")
    );
    session.evaluate("a.fade(99,120)").unwrap();
    session.evaluate("invalidate a").unwrap();
    session.advance_clock(20_000).unwrap();
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("a:0,b:123,")
    );
    assert!(session.advance_clock(19_999).is_err());
}

#[test]
fn sound_playback_is_explicit_until_streams_are_bound() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        "var w=new WaveSoundBuffer(null);try{w.open('missing.ogg');}catch(e){return 99;}",
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 1000).unwrap();
    let error = session.startup().unwrap_err();
    assert!(
        error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
        "{error:#}"
    );
    assert!(format!("{error:#}").contains("WaveSoundBuffer.open is not implemented"));
}
