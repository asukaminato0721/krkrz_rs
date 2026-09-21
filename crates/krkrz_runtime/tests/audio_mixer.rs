use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn setup(
    rate: u32,
    frames: usize,
    source: &str,
) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut wave = Vec::new();
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36u32 + frames as u32 * 2).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    wave.extend_from_slice(&rate.to_le_bytes());
    wave.extend_from_slice(&(rate * 2).to_le_bytes());
    wave.extend_from_slice(b"\x02\0\x10\0data");
    wave.extend_from_slice(&(frames as u32 * 2).to_le_bytes());
    for i in 0..frames {
        wave.extend_from_slice(&(((i % 32) as i16 - 16) * 1024).to_le_bytes());
    }
    std::fs::write(project.path().join("tone.wav"), wave).unwrap();
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=50; Name='cue'; }",
    )
    .unwrap();
    std::fs::write(project.path().join("startup.tjs"), format!("var events='';var s=new WaveSoundBuffer(null);s.onLabel=function(n){{global.events+=n+',';}};s.onStatusChanged=function(n){{global.events+=n+',';}};s.open('tone.wav');s.play();{source}")).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    session.startup().unwrap();
    session.evaluate("events='' ").unwrap();
    (project, saves, session)
}

#[test]
fn tick_outputs_pcm_and_dispatches_labels_eof() {
    let (_p, _s, mut session) = setup(48000, 96, "s.volume=50000;s.pan=100000;");
    session.tick(1).unwrap();
    let audio = session.take_audio();
    assert_eq!(audio.len(), 96);
    for (i, frame) in audio.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(frame[0], 0.0);
        assert_eq!(
            frame[1],
            (((i % 32) as f32 - 16.0) * 1024.0 / 32768.0) * 0.5
        );
    }
    assert_eq!(session.evaluate("events").unwrap(), Value::string(""));
    session.tick(2).unwrap();
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("cue,stop,")
    );
    session.tick(3).unwrap();
    assert!(session.take_audio().is_empty());
}

#[test]
fn resampling_is_independent_of_tick_partition_and_seek_discards_lookahead() {
    let (_p1, _s1, mut a) = setup(44100, 4410, "");
    let (_p2, _s2, mut b) = setup(44100, 4410, "");
    a.tick(20).unwrap();
    let expected = a.take_audio();
    let mut actual = Vec::new();
    for time in [1, 4, 9, 20] {
        b.tick(time).unwrap();
        actual.extend(b.take_audio());
    }
    assert_eq!(actual, expected);
    assert_eq!(a.evaluate("events").unwrap(), b.evaluate("events").unwrap());
    b.evaluate("s.samplePosition=0").unwrap();
    b.tick(40).unwrap();
    assert_eq!(b.take_audio(), expected);
    b.evaluate("s.paused=true").unwrap();
    let before = b.evaluate("s.samplePosition").unwrap();
    b.tick(50).unwrap();
    assert_eq!(b.evaluate("s.samplePosition").unwrap(), before);
    assert!(b.take_audio().is_empty());
}

#[test]
fn library_resampling_matches_source_clock_across_rates_and_live_rate_changes() {
    // The scalar expression below is the pre-migration interpolation contract.
    // Compare a complete signal, including the first sample and rate boundaries.
    let (_p, _s, mut session) = setup(48000, 40_000, "");
    let mut position = 0u64;
    let mut time = 0;
    for frequency in [1, 8000, 44100, 96000, 48000, 768000, 22050, 1] {
        session
            .evaluate(&format!("s.frequency={frequency}"))
            .unwrap();
        time += 4;
        session.tick(time).unwrap();
        let audio = session.take_audio();
        assert_eq!(audio.len(), 4 * 48 * 2);
        for frame in audio.as_chunks::<2>().0 {
            let at = position / 48_000;
            let fraction = (position % 48_000) as f32 / 48_000.;
            let sample = |at: u64| ((at % 32) as f32 - 16.) / 32.;
            let expected = sample(at) + (sample(at + 1) - sample(at)) * fraction;
            assert!(
                (frame[0] - expected).abs() < 0.000002,
                "rate {frequency}, position {position}: {} != {expected}",
                frame[0]
            );
            assert_eq!(frame[0], frame[1]);
            position += frequency;
        }
    }
}

#[test]
fn downsampling_does_not_skip_labels_or_delay_eof_between_output_samples() {
    let (project, _s, mut session) = setup(768000, 768, "");
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=767; Name='last'; }",
    )
    .unwrap();
    session
        .evaluate("Scripts.exec('s.open(\"tone.wav\");s.play();events=\"\";')")
        .unwrap();
    session.tick(1).unwrap();
    assert_eq!(session.take_audio().len(), 96);
    assert_eq!(
        session.evaluate("events").unwrap(),
        Value::string("last,stop,")
    );
    session.tick(2).unwrap();
    assert!(session.take_audio().is_empty());
}

#[test]
fn looping_and_fractional_rates_are_stable_across_tick_partitions() {
    for rate in [1, 8000, 32000, 44100, 48000, 96000, 768000] {
        let (_p1, _s1, mut a) = setup(rate, 64, "s.looping=true;");
        let (_p2, _s2, mut b) = setup(rate, 64, "s.looping=true;");
        a.tick(20).unwrap();
        let expected = a.take_audio();
        let mut actual = Vec::new();
        for time in [1, 2, 3, 5, 9, 11, 20] {
            b.tick(time).unwrap();
            actual.extend(b.take_audio());
        }
        assert_eq!(actual.len(), expected.len());
        let error = actual
            .iter()
            .zip(expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0., f32::max);
        assert!(error < 0.000002, "rate {rate}: partition error {error}");
        assert_eq!(
            a.evaluate("events").unwrap(),
            b.evaluate("events").unwrap(),
            "rate {rate}"
        );
        assert_eq!(a.evaluate("s.status").unwrap(), Value::string("play"));
    }
}
