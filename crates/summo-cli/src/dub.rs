//! `summo dub` — a meeting, spoken in another language, over its own recording.
//!
//! The pipeline is [`summo_engine::dub`]. This is the terminal in front of it.
//!
//! It used to be the whole implementation, and that was the problem: dubbing worked, had tests, and
//! had two published voices, and nothing in the app could reach any of it. Moving the pipeline into
//! the daemon gave the feature a second front door without giving it a second implementation, and
//! left this file as what it should have been — argument parsing and printing.

use anyhow::Result;
use summo_core::paths::Paths;
pub use summo_engine::dub::{JobState, Options};

/// Run one dub, narrating it.
///
/// Progress goes to stderr as a single rewritten line, so piping stdout somewhere useful is not
/// ruined by a hundred lines of counting. Two passes are named as such — a bar that fills, resets
/// and fills again looks broken unless it says why.
pub fn run(paths: &Paths, opts: &Options) -> Result<()> {
    let report = summo_engine::dub::run(paths, opts, &narrate)?;

    // The trailing spaces clear whatever the longest progress line left behind; `\r` moves the
    // cursor back but does not erase.
    eprintln!("\r{:<60}", "");
    println!("voice  {}", report.voice);
    println!("lines  {} of {} translated", report.lines, report.of);
    println!(
        "wrote  {} ({:.1}s at {} Hz)\nfit    {} natural, {} adjusted, {} overflowing{}",
        report.out,
        report.duration_s,
        report.rate,
        report.natural,
        report.adjusted,
        report.overflowing,
        if report.overflowing > 0 {
            format!(" — worst runs {:.1}s long", report.worst_over_s)
        } else {
            String::new()
        }
    );
    Ok(())
}

fn narrate(state: JobState) {
    use std::io::Write;
    let line = match state {
        JobState::Loading => "loading the voice".to_string(),
        JobState::Speaking {
            pass,
            spoken,
            total,
        } => format!("pass {pass} of 2 — {spoken}/{total} lines"),
        JobState::Mixing => "mixing over the recording".to_string(),
        _ => return,
    };
    eprint!("\r{line:<60}");
    std::io::stderr().flush().ok();
}
