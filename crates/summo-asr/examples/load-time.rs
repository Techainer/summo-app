//! What the three seconds before a meeting starts are actually spent on.
//!
//! `e2e/start-latency.mjs` narrowed "pressing record is slow" down to one number: building the
//! decoder, 2.9 s, with the voice detector at 137 ms and everything else in the noise. Warming
//! moves that cost off the press for anybody who pauses before pressing, and does nothing for the
//! person who presses immediately — for them the only remaining question is whether the build
//! itself can be made smaller.
//!
//! It is not disk. `warm.rs` already records that the second construction in a process costs the
//! same as the first, so the page cache is warm and the time is inside ONNX Runtime. What is left
//! to ask is how much of it scales with the thread count sherpa is given, which is the one knob
//! this code actually turns: `recommended_threads()` is `cores.clamp(1, 8)`, so a laptop and a
//! 64-core machine both ask for 8.
//!
//! ```text
//! cargo run -p summo-asr --features sherpa --example load-time -- <model-dir>
//! ```
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: load-time <model-dir>");
        std::process::exit(2);
    });

    // Once before the measurements, so the first result is not also paying for whatever a process
    // does the first time it touches ONNX Runtime.
    let _ = summo_asr::sherpa::ZipformerDecoder::from_dir(&dir, 1);

    println!("threads   load");
    for threads in [1, 2, 4, 8, 16] {
        let mut best = f64::MAX;
        for _ in 0..3 {
            let began = Instant::now();
            match summo_asr::sherpa::ZipformerDecoder::from_dir(&dir, threads) {
                Ok(decoder) => {
                    let ms = began.elapsed().as_secs_f64() * 1000.0;
                    best = best.min(ms);
                    // Dropped explicitly, so the next construction does not overlap this one's
                    // teardown and report a number that belongs to both.
                    drop(decoder);
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        println!("{threads:>7}   {best:>6.0} ms");
    }
}
