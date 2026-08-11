//! Say a line and write it to a WAV.
//!
//! An example rather than a test: it needs a voice on disk, and a test that silently passes when
//! the model is absent proves nothing. This is how the synthesiser is exercised for real.
//!
//! ```bash
//! cargo run -p summo-tts --features sherpa --example speak -- \
//!     /path/to/vits-piper-en_US-amy-low "Hello there" /tmp/out.wav
//! ```
fn main() -> summo_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: speak <voice-dir> [text] [out.wav]");
    let text = args.next().unwrap_or_else(|| "Hello from Summo".into());
    let out = args.next().unwrap_or_else(|| "/tmp/speak.wav".into());

    let started = std::time::Instant::now();
    let mut tts = summo_tts::vits::Vits::load(std::path::Path::new(&dir), 4)?;
    println!("loaded in {} ms", started.elapsed().as_millis());

    for speed in [1.0_f32, 1.3] {
        let at = std::time::Instant::now();
        let speech = tts.say_at(&text, speed)?;
        let wall = at.elapsed().as_secs_f64();
        let path = if speed == 1.0 {
            out.clone()
        } else {
            out.replace(".wav", "-fast.wav")
        };
        summo_tts::dub::write_wav(std::path::Path::new(&path), &speech.samples, speech.rate)?;

        println!(
            "speed {speed:.1} → {:.2}s of audio at {} Hz in {wall:.2}s (RTF {:.3}) → {path}",
            speech.duration_s(),
            speech.rate,
            wall / speech.duration_s().max(f64::EPSILON),
        );
    }
    Ok(())
}
