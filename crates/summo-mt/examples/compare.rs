//! Translate the same lines with two models and print both, side by side.
//!
//! Every claim in `docs/translation.md` about which model to use came from running this. It exists
//! so those claims can be re-checked rather than believed — a model recommendation that nobody can
//! reproduce is a preference.
//!
//! ```bash
//! cargo run -p summo-mt --features local --example compare -- \
//!   /path/to/milmmt-46-1b.gguf /path/to/milmmt-46-4b.gguf
//! ```
//!
//! The lines are Vietnamese meeting speech, not textbook sentences: dropped pronouns, English
//! product terms mid-clause, and the register a colleague actually uses. That is what the model has
//! to be good at, and it is where the differences show up.

use std::time::Instant;

use summo_mt::Local;

/// Source, target, and the line. Chosen for what each one breaks:
///
/// * `chiều nay` — a time expression small models translate as "tonight" or "this morning".
/// * `test tải` — a compound where the English term is load-testing, not downloading.
/// * `dời mốc` — the line that made MiLMMT-46-1B answer in Thai.
/// * The Japanese and Chinese lines go the other way, which is the direction a Vietnamese user
///   watching a foreign meeting needs.
const CASES: &[(&str, &str, &str)] = &[
    (
        "vi",
        "en",
        "Chiều nay mình chốt lại spec API rồi gửi cho bên khách hàng nhé.",
    ),
    (
        "vi",
        "ja",
        "Chiều nay mình chốt lại spec API rồi gửi cho bên khách hàng nhé.",
    ),
    (
        "vi",
        "zh",
        "Bên mình cần thêm hai ngày để test tải, không thì lúc go-live sẽ vỡ.",
    ),
    (
        "vi",
        "ja",
        "Ok vậy mình dời mốc ra thứ Sáu tuần sau, mình sẽ báo lại cho khách.",
    ),
    (
        "ja",
        "vi",
        "うちの中学は弁当制で持っていけない場合は、50円の学校販売のパンを買う。",
    ),
    ("zh", "vi", "开放时间早上9点至下午5点。"),
    (
        "en",
        "vi",
        "The tribal chieftain called for the boy and presented him with fifty pieces of gold.",
    ),
];

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: compare <model.gguf> [model.gguf ...]");
        std::process::exit(2);
    }

    for path in &paths {
        let model = match Local::load(path, Some(8)) {
            Ok(model) => model,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        println!("\n=== {} ===", model.name());

        let mut total = 0.0_f64;
        for (source, target, line) in CASES {
            let prompt = summo_llm::prompt::mt_text(line, Some(source), target);
            let started = Instant::now();
            let out = model.complete(&prompt).unwrap_or_else(|e| format!("<{e}>"));
            let elapsed = started.elapsed().as_secs_f64();
            total += elapsed;

            let kept = summo_llm::lang::plausible(&out, target);
            println!(
                "[{source}->{target}] {:5.0}ms {}{out}",
                elapsed * 1000.0,
                if kept {
                    ""
                } else {
                    "REFUSED (wrong language) "
                },
            );
        }
        println!("average {:.0} ms/line", total / CASES.len() as f64 * 1000.0);
    }
}
