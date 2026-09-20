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
fn original_wave_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/wave.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("tone.wav"),
            include_bytes!("fixtures/tone.wav"),
        )
        .unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let result = Session::open(project.path(), Some(saves.path()), false, 10000)
            .unwrap()
            .execute_storage("case.tjs")
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.name));
        assert_eq!(result, case.expected, "{}", case.name);
    }
}

#[test]
fn pcm_preview_loops_labels_and_eof() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("tone.wav"),
        include_bytes!("fixtures/tone.wav"),
    )
    .unwrap();
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=2; Name='mouth'; }",
    )
    .unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
        Plugins.link("getSample.dll");
        var events="";
        var w=new WaveSoundBuffer(null);
        w.onLabel=function(n){events+=n+",";};
        w.onStatusChanged=function(s){events+=s+",";};
        w.open("tone.wav"); w.sampleCount=4; w.play();
    "#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 50000).unwrap();
    session.startup().unwrap();
    let sound = session.evaluate("w").unwrap();
    let peak = (32767.0f64 / 32768.0).powi(2);
    assert_eq!(
        session.evaluate("w.sampleValue").unwrap(),
        Value::Real(peak)
    );
    assert_eq!(
        session.evaluate("w.getSample(4)").unwrap(),
        Value::Integer((8192 + 32767) / 3)
    );
    assert_eq!(
        session.evaluate("w.samplePosition").unwrap(),
        Value::Integer(0)
    );
    let first = session.render_sound_source(&sound, 4).unwrap();
    assert_eq!(first.samples, [0, 8192, -16384, 32767]);
    assert_eq!(first.labels[0].offset, 2);
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("stop,play,")
    );
    session.dispatch_events().unwrap();
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("stop,play,mouth,")
    );
    session.evaluate("w.paused=1").unwrap();
    assert!(
        session
            .render_sound_source(&sound, 10)
            .unwrap()
            .samples
            .is_empty()
    );
    assert_eq!(
        session.evaluate("w.samplePosition").unwrap(),
        Value::Integer(4)
    );
    session.evaluate("w.paused=0").unwrap();
    assert_eq!(
        session
            .render_sound_source(&sound, 5000)
            .unwrap()
            .samples
            .len(),
        4406
    );
    assert_eq!(session.evaluate("w.status").unwrap(), Value::string("play"));
    session.dispatch_events().unwrap();
    assert_eq!(session.evaluate("w.status").unwrap(), Value::string("stop"));
    session.evaluate("w.stop()").unwrap();
    session.evaluate("w.looping=1").unwrap();
    session.evaluate("w.play()").unwrap();
    assert_eq!(
        session
            .render_sound_source(&sound, 4414)
            .unwrap()
            .samples
            .len(),
        4414
    );
    assert_eq!(
        session.evaluate("w.samplePosition").unwrap(),
        Value::Integer(4)
    );
    // Invalidating the source drops pending label events.
    session.evaluate("invalidate w").unwrap();
    session.dispatch_events().unwrap();
    assert!(session.render_sound_source(&sound, 1).is_err());
}

#[test]
fn label_callback_cancels_stale_eof_and_later_labels() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("tone.wav"),
        include_bytes!("fixtures/tone.wav"),
    )
    .unwrap();
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=1; Name='one'; }\nLabel { Position=2; Name='two'; }",
    )
    .unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
        var events=""; var w=new WaveSoundBuffer(null); w.open("tone.wav");
        w.onLabel=function(n){ events+=n; w.stop(); w.play(); }; w.play();
    "#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 10000).unwrap();
    session.startup().unwrap();
    let sound = session.evaluate("w").unwrap();
    session.render_sound_source(&sound, 5000).unwrap();
    session.dispatch_events().unwrap();
    assert_eq!(session.evaluate("events").unwrap(), Value::string("one"));
    assert_eq!(session.evaluate("w.status").unwrap(), Value::string("play"));
    assert_eq!(
        session.evaluate("w.samplePosition").unwrap(),
        Value::Integer(0)
    );
}
