//! Translate the same lines with two models and print both, side by side.
//!
//! Every claim in `docs/translation.md` about which model to use came from running this. It exists
//! so those claims can be re-checked rather than believed — a model recommendation that nobody can
//! reproduce is a preference.
//!
//! A `.gguf` file is opened with llama.cpp; a directory is opened as an ONNX seq2seq export.
//!
//! ```bash
//! cargo run -p summo-mt --features local,onnx --example compare -- \
//!   /path/to/milmmt-46-1b.gguf /path/to/small100-int8/
//! ```
//!
//! The lines are Vietnamese meeting speech, not textbook sentences: dropped pronouns, English
//! product terms mid-clause, and the register a colleague actually uses. That is what the model has
//! to be good at, and it is where the differences show up.

use std::time::Instant;

use summo_mt::{Local, Seq2Seq};

/// Either runtime, behind one call. The point of the example is to compare models, and a model's
/// runtime is a fact about the model rather than something the reader should have to select.
enum Model {
    Gguf(Box<Local>),
    Onnx(Box<Seq2Seq>),
}

impl Model {
    fn open(path: &str) -> summo_core::Result<Self> {
        let path = std::path::Path::new(path);
        if path.is_dir() {
            let name = path
                .file_name()
                .map_or_else(|| "onnx".to_string(), |n| n.to_string_lossy().into_owned());
            Ok(Self::Onnx(Box::new(
                Seq2Seq::load(&summo_mt::seq2seq::discover(path), Some(8))?.named(name),
            )))
        } else {
            Ok(Self::Gguf(Box::new(Local::load(path, Some(8))?)))
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Gguf(m) => m.name(),
            Self::Onnx(m) => m.name(),
        }
    }

    fn translate(&self, line: &str, source: &str, target: &str) -> summo_core::Result<String> {
        match self {
            Self::Gguf(m) => {
                let prompt = summo_llm::prompt::mt_text(line, Some(source), target);
                Ok(summo_llm::prompt::parse_mt(&m.complete(&prompt)?).unwrap_or_default())
            }
            Self::Onnx(m) => m.translate(line, target),
        }
    }
}

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
        let model = match Model::open(path) {
            Ok(model) => model,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        println!("\n=== {} ===", model.name());

        let mut total = 0.0_f64;
        for (source, target, line) in CASES {
            let started = Instant::now();
            let out = model
                .translate(line, source, target)
                .unwrap_or_else(|e| format!("<{e}>"));
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
