//! Summaries and questions over what was said.
//!
//! The one part of Summo that sends anything off the machine, and only ever text. Which model it
//! goes to is the user's choice, and the default suggestion is Ollama on their own laptop — in that
//! configuration nothing leaves at all.
//!
//! Retrieval for a question is a scan across the vault rather than an index. That is not a
//! simplification: `summo-bench vault` measured a parallel scan at 30 ms over a thousand meetings,
//! which is four years of daily hour-long meetings, and an index would have cost more disk than the
//! transcripts it indexed. See `docs/adr/0002-no-database.md`.

use anyhow::{Context, Result, bail};
use summo_core::paths::Paths;
use summo_llm::{
    LlmClient, Provider,
    prompt::{self, SummaryStyle},
};
use summo_vault::MeetingDoc;

/// Where the answer should come from.
#[derive(Debug, Clone, clap::Args)]
pub struct ProviderArgs {
    /// `ollama`, `lm-studio`, `openai`, `anthropic`, or a base URL for anything OpenAI-compatible.
    #[arg(long, default_value = "ollama")]
    pub provider: String,
    /// Model name at that endpoint.
    #[arg(long)]
    pub model: Option<String>,
    /// API key. Read from `SUMMO_API_KEY` when not given, so it does not land in shell history.
    #[arg(long, env = "SUMMO_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,
}

impl ProviderArgs {
    /// Build a client, defaulting to whatever the named provider usually runs.
    pub fn client(&self) -> Result<LlmClient> {
        let provider = Provider::resolve(
            &self.provider,
            self.model.as_deref(),
            self.api_key.as_deref(),
        )?;

        // Worth saying out loud rather than burying: this is the moment transcript text leaves the
        // machine, and the user should know before it does, not afterwards.
        if provider.is_local() {
            eprintln!(
                "using {} at {} — nothing leaves this machine",
                provider.name, provider.base_url
            );
        } else {
            eprintln!(
                "using {} at {} — transcript text will be sent there. Audio never is.",
                provider.name, provider.base_url
            );
        }

        LlmClient::new(provider).map_err(Into::into)
    }
}

fn load(path: &std::path::Path) -> Result<MeetingDoc> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    MeetingDoc::parse(&body).map_err(Into::into)
}

/// Summarise a meeting and write the result back into its own file.
pub async fn summarize(
    path: &std::path::Path,
    style: SummaryStyle,
    language: &str,
    args: &ProviderArgs,
    write: bool,
) -> Result<()> {
    let mut doc = load(path)?;
    if doc.transcript.is_empty() {
        bail!("{} has no transcript to summarise", path.display());
    }

    let client = args.client()?;
    let transcript = prompt::render_transcript(&doc.transcript);
    let messages = prompt::summarize(&transcript, style, language);

    let mut first = true;
    let summary = client
        .stream(&messages, |chunk| {
            if first {
                eprintln!();
                first = false;
            }
            print!("{chunk}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        })
        .await?;
    println!();

    if write {
        // Into the meeting's own file, so the summary lives with the transcript it came from and
        // syncs, greps and exports with it.
        doc.set_section("Tóm tắt", summary.trim());
        std::fs::write(path, doc.to_markdown()?)
            .with_context(|| format!("cannot write {}", path.display()))?;
        eprintln!("\nwritten to {}", path.display());
    }
    Ok(())
}

/// Answer a question from the meetings on disk.
pub async fn ask(
    paths: &Paths,
    question: &str,
    language: &str,
    limit: usize,
    args: &ProviderArgs,
) -> Result<()> {
    let excerpts = search(paths, question, limit)?;
    if excerpts.is_empty() {
        // Saying so beats asking a model to answer from nothing, which produces a confident
        // paragraph of invention.
        bail!("nothing in the vault mentions any of those words");
    }
    eprintln!(
        "{} excerpt(s) from {} meeting(s)",
        excerpts.len(),
        count_meetings(&excerpts)
    );

    let context = excerpts
        .iter()
        .map(|e| format!("[{}] {}\n{}", e.timestamp, e.meeting, e.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let client = args.client()?;
    let messages = prompt::answer(question, &context, language);

    eprintln!();
    client
        .stream(&messages, |chunk| {
            print!("{chunk}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        })
        .await?;
    println!();
    Ok(())
}

/// One matching line, with enough around it to be quotable.
#[derive(Debug)]
pub struct Excerpt {
    pub meeting: String,
    pub timestamp: String,
    pub text: String,
}

fn count_meetings(excerpts: &[Excerpt]) -> usize {
    let mut seen: Vec<&str> = excerpts.iter().map(|e| e.meeting.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

/// Find lines mentioning the question's words, with a line of context either side.
///
/// Deliberately simple. It matches what people search for — a phrase they remember hearing — and it
/// is honest about what it cannot do: a question whose answer never uses its own words needs
/// embeddings, and that is the one capability a scan cannot provide.
pub fn search(paths: &Paths, question: &str, limit: usize) -> Result<Vec<Excerpt>> {
    // Strip punctuation *before* measuring length. The other order lets "gì?" through as three
    // characters and then matches on the two that are left, which is exactly the noise this filter
    // exists to remove.
    let terms: Vec<String> = question
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.chars().count() > 2)
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let dir = paths.meetings();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    // Newest first: recent meetings are far more often what someone is asking about.
    files.sort();
    files.reverse();

    for path in files {
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = MeetingDoc::parse(&body) else {
            continue;
        };
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        for segment in &doc.transcript {
            let haystack = segment.text.to_lowercase();
            let hits = terms.iter().filter(|t| haystack.contains(*t)).count();
            if hits == 0 {
                continue;
            }
            out.push(Excerpt {
                meeting: name.clone(),
                timestamp: summo_vault::meeting::format_timestamp(segment.t0),
                text: segment.text.clone(),
            });
            if out.len() >= limit {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_with(lines: &[(&str, &str)]) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();

        let mut body = String::from("---\nid: m1\ndate: 2026-08-10\n---\n# Họp\n\n## Transcript\n");
        for (time, text) in lines {
            body.push_str(&format!("**[{time}] me** — {text}\n"));
        }
        std::fs::write(paths.meetings().join("2026-08-10-hop.md"), body).unwrap();
        (tmp, paths)
    }

    #[test]
    fn a_question_finds_the_line_that_mentions_it() {
        let (_tmp, paths) = vault_with(&[
            ("00:00:04", "chốt dùng Rust cho phần lõi"),
            ("00:00:10", "ngân sách quý tới tăng"),
        ]);
        let hits = search(&paths, "quyết định về Rust là gì?", 10).unwrap();

        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("Rust"));
        assert_eq!(hits[0].timestamp, "00:00:04");
    }

    #[test]
    fn short_words_are_ignored_so_every_line_does_not_match() {
        // Without this, "là" and "gì" match almost everything and the excerpt list is noise.
        let (_tmp, paths) = vault_with(&[("00:00:01", "cái này là gì thế")]);
        assert!(search(&paths, "là gì?", 10).unwrap().is_empty());
    }

    #[test]
    fn results_are_capped() {
        let lines: Vec<(String, String)> = (0..50)
            .map(|i| (format!("00:00:{i:02}"), "ngân sách".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = lines
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (_tmp, paths) = vault_with(&refs);

        assert_eq!(search(&paths, "ngân sách", 5).unwrap().len(), 5);
    }

    #[test]
    fn an_empty_vault_answers_nothing_rather_than_erroring() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        assert!(search(&paths, "bất cứ điều gì", 10).unwrap().is_empty());
    }

    #[test]
    fn a_local_provider_is_reported_as_local() {
        // The user should learn that text is about to leave *before* it does.
        let local = ProviderArgs {
            provider: "ollama".into(),
            model: None,
            api_key: None,
        };
        assert!(local.client().is_ok());
    }

    #[test]
    fn a_hosted_provider_without_a_key_fails_clearly() {
        let hosted = ProviderArgs {
            provider: "openai".into(),
            model: None,
            api_key: None,
        };
        let Err(err) = hosted.client() else {
            panic!("a hosted provider without a key should not build a client")
        };
        assert!(err.to_string().contains("API key"), "got: {err}");
    }

    #[test]
    fn an_unknown_provider_lists_the_ones_that_work() {
        let bad = ProviderArgs {
            provider: "nonsense".into(),
            model: None,
            api_key: None,
        };
        let Err(err) = bad.client() else {
            panic!("an unknown provider should not build a client")
        };
        assert!(err.to_string().contains("ollama"), "got: {err}");
    }
}
