//! Synthetic stereo vocoder benchmark and optional PCM capture for comparisons.
//! cargo run --release -p krkrz_runtime --example vocoder_profile -- 4096 1.5 0.8 /tmp/new.pcm
use anyhow::{Context, Result, ensure};
use krkrz_runtime::Session;
use std::{io::Write, time::Instant};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() <= 4,
        "usage: vocoder_profile [window time pitch new.pcm]"
    );
    let window: usize = args.first().map_or("4096", String::as_str).parse()?;
    let time: f32 = args.get(1).map_or("1.5", String::as_str).parse()?;
    let pitch: f32 = args.get(2).map_or("0.8", String::as_str).parse()?;
    ensure!(
        time.is_finite() && pitch.is_finite(),
        "parameters must be finite"
    );
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    let rate = 48_000u32;
    let samples: Vec<i16> = (0..rate * 4)
        .flat_map(|i| {
            let t = i as f64 / rate as f64;
            let phase = std::f64::consts::TAU * t;
            let envelope = if i % 24_000 < 12_000 { 1.0 } else { 0.25 };
            [
                ((phase * 233.0).sin() + (phase * 1777.0).sin() * 0.3) * 12_000.0 * envelope,
                ((phase * 571.0).sin() + (phase * 7013.0).sin() * 0.2) * 10_000.0,
            ]
            .map(|v| v as i16)
        })
        .collect();
    let size = samples.len() as u32 * 2;
    let mut wave = b"RIFF".to_vec();
    wave.extend((36 + size).to_le_bytes());
    wave.extend(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    wave.extend(rate.to_le_bytes());
    wave.extend((rate * 4).to_le_bytes());
    wave.extend(b"\x04\0\x10\0data");
    wave.extend(size.to_le_bytes());
    wave.extend(samples.into_iter().flat_map(i16::to_le_bytes));
    std::fs::write(project.path().join("tone.wav"), wave)?;
    std::fs::write(
        project.path().join("tone.wav.sli"),
        "#2.00\nLabel { Position=1024; Name='early'; }\nLabel { Position=96000; Name='middle'; }",
    )?;
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000)?;
    session.evaluate(&format!(
        "Scripts.exec('global.a=new WaveSoundBuffer(null);var v=new WaveSoundBuffer.PhaseVocoder();v.window={window};v.time={time};v.pitch={pitch};a.filters.add(v);a.open(\"tone.wav\");')"
    ))?;
    let sound = session.evaluate("a")?;
    let mut elapsed = Vec::new();
    let mut reference = None;
    // Warm once, then measure five renders, excluding file I/O and setup.
    for run in 0..6 {
        session.evaluate("a.stop()")?;
        session.evaluate("a.play()")?;
        let start = Instant::now();
        let block = session.render_sound_source(&sound, 1_000_000)?;
        let duration = start.elapsed();
        ensure!(
            block.samples.len() < 2_000_000,
            "render exceeded the benchmark frame limit"
        );
        if run > 0 {
            elapsed.push(duration);
        }
        if let Some(reference) = &reference {
            ensure!(reference == &block, "PCM or labels changed after restart");
        } else {
            reference = Some(block);
        }
    }
    elapsed.sort();
    let block = reference.context("missing rendered audio")?;
    println!(
        "window={window} time={time} pitch={pitch} frames={} median_ms={:.3} labels={:?}",
        block.samples.len() / 2,
        elapsed[2].as_secs_f64() * 1000.0,
        block.labels
    );
    if let Some(path) = args.get(3) {
        let mut file = std::fs::File::create_new(path)?;
        file.write_all(
            &block
                .samples
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
    }
    Ok(())
}
