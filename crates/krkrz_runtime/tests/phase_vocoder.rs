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
fn original_phase_vocoder_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/phase_vocoder.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), &case.source).unwrap();
        let mut session =
            Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        assert_eq!(
            session
                .execute_storage("case.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut wave = Vec::new();
    let samples: Vec<i16> = (0..8192)
        .map(|i| ((i as f32 * std::f32::consts::TAU / 64.0).sin() * 16000.0) as i16)
        .collect();
    wave.extend(b"RIFF");
    wave.extend((36 + samples.len() as u32 * 2).to_le_bytes());
    wave.extend(b"WAVEfmt ");
    wave.extend(16u32.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(1u16.to_le_bytes());
    wave.extend(8192u32.to_le_bytes());
    wave.extend(16384u32.to_le_bytes());
    wave.extend(2u16.to_le_bytes());
    wave.extend(16u16.to_le_bytes());
    wave.extend(b"data");
    wave.extend((samples.len() as u32 * 2).to_le_bytes());
    wave.extend(samples.into_iter().flat_map(i16::to_le_bytes));
    std::fs::write(project.path().join("tone.wav"), wave).unwrap();
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=1024; Name='lip'; }",
    )
    .unwrap();
    std::fs::write(project.path().join("startup.tjs"), "global.a=new WaveSoundBuffer(null);global.b=new WaveSoundBuffer(null);global.v=new WaveSoundBuffer.PhaseVocoder();v.window=256;v.time=0.5;a.filters.add(v);a.open('tone.wav');").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    session.startup().unwrap();
    (project, saves, session)
}

#[test]
fn filters_connect_on_open_and_cannot_share_active_sources() {
    let (_project, _saves, mut session) = session();
    session.evaluate("b.filters.add(v)").unwrap();
    let error = session.evaluate("b.open('tone.wav')").unwrap_err();
    assert!(format!("{error:#}").contains("multiple sources"));
    assert_eq!(
        session.evaluate("b.status").unwrap(),
        Value::string("unload")
    );
    // Editing the array does not alter a chain that is already connected.
    session.evaluate("a.filters.clear()").unwrap();
    session.evaluate("a.play()").unwrap();
    let sound = session.evaluate("a").unwrap();
    let block = session.render_sound_source(&sound, 1024).unwrap();
    assert_eq!(block.labels[0].offset, 512);
    assert_eq!(
        session.evaluate("a.samplePosition").unwrap(),
        Value::Integer(2048)
    );
    // open disconnects the old chain; another buffer can then use the filter.
    session.evaluate("a.open('tone.wav')").unwrap();
    session.evaluate("b.open('tone.wav')").unwrap();
    assert_eq!(session.evaluate("b.status").unwrap(), Value::string("stop"));
}

#[test]
fn filtered_eof_and_label_callbacks_share_the_session_queue() {
    let (_project, _saves, mut session) = session();
    session.evaluate("Scripts.exec('global.events=[];a.onLabel=function(n){events.add(n);};a.onStatusChanged=function(s){events.add(s);};a.play();')").unwrap();
    let sound = session.evaluate("a").unwrap();
    let block = session.render_sound_source(&sound, 20000).unwrap();
    assert_eq!(block.samples.len(), 4000);
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("play")
    );
    session.dispatch_events().unwrap();
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("play,lip,stop")
    );
    // Reopening and seeking reset all FFT and overlap state.
    session.evaluate("a.stop()").unwrap();
    session.evaluate("a.play()").unwrap();
    assert_eq!(session.render_sound_source(&sound, 20000).unwrap(), block);
}

#[test]
fn invalid_filter_handles_and_parameters_are_regular_errors() {
    let (_project, _saves, mut session) = session();
    assert!(session.evaluate("v.interface=0").is_err());
    session.evaluate("v.time=0").unwrap();
    session.evaluate("a.play()").unwrap();
    let sound = session.evaluate("a").unwrap();
    assert!(session.render_sound_source(&sound, 128).is_err());
    session.evaluate("v.time=1").unwrap();
    session.evaluate("a.stop()").unwrap();
    session.evaluate("a.play()").unwrap();
    assert_eq!(
        session
            .render_sound_source(&sound, 128)
            .unwrap()
            .samples
            .len(),
        128
    );
    session
        .evaluate("b.filters.add(%[interface:999999])")
        .unwrap();
    assert!(session.evaluate("b.open('tone.wav')").is_err());
    assert_eq!(
        session.evaluate("b.status").unwrap(),
        Value::string("unload")
    );
}

#[test]
fn changing_filter_windows_cannot_exceed_the_session_memory_budget() {
    let (_project, _saves, mut session) = session();
    session.evaluate("Scripts.exec('for(var i=0;i<24;i++){var f=new WaveSoundBuffer.PhaseVocoder();f.window=64;b.filters.add(f);}b.open(\"tone.wav\");for(var i=0;i<24;i++)b.filters[i].window=32768;b.play();')").unwrap();
    let sound = session.evaluate("b").unwrap();
    let error = session.render_sound_source(&sound, 128).unwrap_err();
    assert!(format!("{error:#}").contains("memory limit"));
    assert_eq!(
        session.evaluate("b.samplePosition").unwrap(),
        Value::Integer(0)
    );
}
