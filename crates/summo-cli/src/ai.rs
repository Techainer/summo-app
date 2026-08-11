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
        // The user's own `providers.json` counts here as much as it does in the interface: a
        // command that cannot reach an endpoint the settings screen offers is a split product.
        let catalogue = summo_core::paths::Paths::discover()
            .map(|paths| summo_llm::provider::catalogue(&paths.providers()))
            .unwrap_or_default();
        let provider = Provider::resolve_in(
            &catalogue,
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
    // Its own parent as the vault root: this takes a path to any file, which need not be inside a
    // vault at all, and the derived id only matters for a file that has no frontmatter.
    let root = path.parent().unwrap_or(std::path::Path::new("."));
    summo_vault::open(root, path).with_context(|| format!("cannot read {}", path.display()))
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

    // The listing, not a `read_dir`.
    //
    // `read_dir` is not recursive, so a meeting filed into a folder — which the interface encourages
    // with a drag-and-drop folder tree — was invisible to every question asked here. Notes were too,
    // since they live in a different directory entirely. The answer would come back confidently
    // sourced from whatever *was* reachable, which is worse than saying nothing was found.
    let vault = paths.vault();
    let meetings = paths.meetings();
    let notes = paths.notes();
    let index = summo_vault::MeetingIndex::scan_all([meetings.as_path(), notes.as_path()])?;

    let mut entries: Vec<_> = index.entries().iter().collect();
    // Newest first: recent meetings are far more often what someone is asking about.
    entries.sort_by(|a, b| b.date.cmp(&a.date));

    let mut out = Vec::new();
    for entry in entries {
        let Ok(doc) = summo_vault::open(&vault, &entry.path) else {
            continue;
        };
        let name = entry.title.clone();

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

    /// A meeting dragged into a folder — which the sidebar's folder tree exists to encourage — was
    /// invisible to every question, because the scan was a non-recursive `read_dir`. The answer
    /// still came back, sourced from whatever happened to be at the top level.
    #[test]
    fn a_meeting_inside_a_folder_is_searched() {
        let (_tmp, paths) = vault_with(&[("00:00:01", "gì đó")]);
        let folder = paths.meetings().join("Sản phẩm");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("2026-08-11-ngan-sach.md"),
            "---\nid: m2\ndate: 2026-08-11\n---\n# Ngân sách\n\n## Transcript\n**[00:00:02] me** — ngân sách quý tới tăng gấp đôi\n",
        )
        .unwrap();

        let hits = search(&paths, "ngân sách quý tới", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].text.contains("gấp đôi"));
    }

    /// Notes live in a different directory, so after the note-centric refactor they were missing
    /// from every answer — the one place a user is most likely to have written a decision down.
    #[test]
    fn a_note_is_searched_too() {
        let (_tmp, paths) = vault_with(&[("00:00:01", "gì đó")]);
        std::fs::write(
            paths.notes().join("ghi-chu.md"),
            "# Ghi chú\n\n## Transcript\n**[00:00:03] me** — chốt hạ tầng dùng Cloudflare\n",
        )
        .unwrap();

        let hits = search(&paths, "hạ tầng Cloudflare", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
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

    /// "Without a key" now includes the provider's own environment variable, not just Summo's — so
    /// this has to say what the environment is rather than assume it. On a developer's machine
    /// `OPENAI_API_KEY` is usually exported, and the test read as a failure of the error message
    /// when it was actually the fallback working.
    #[test]
    fn a_hosted_provider_without_a_key_fails_clearly() {
        // SAFETY: single-threaded within this test binary's use of these two variables; nothing
        // else reads them, and both are restored before returning.
        let restore: Vec<(&str, Option<String>)> = ["SUMMO_API_KEY", "OPENAI_API_KEY"]
            .into_iter()
            .map(|name| (name, std::env::var(name).ok()))
            .collect();
        for (name, _) in &restore {
            unsafe { std::env::remove_var(name) };
        }

        let hosted = ProviderArgs {
            provider: "openai".into(),
            model: None,
            api_key: None,
        };
        let built = hosted.client();

        for (name, value) in restore {
            if let Some(value) = value {
                unsafe { std::env::set_var(name, value) };
            }
        }

        let Err(err) = built else {
            panic!("a hosted provider without a key should not build a client")
        };
        assert!(err.to_string().contains("API key"), "got: {err}");
        assert!(
            err.to_string().contains("OPENAI_API_KEY"),
            "the message must name the variable to set: {err}"
        );
    }

    /// The reason the fallback exists: a machine that already talks to a provider should not need
    /// its credential copied into a second, Summo-specific variable.
    #[test]
    fn a_provider_s_own_environment_variable_is_accepted() {
        let restore = std::env::var("GROQ_API_KEY").ok();
        unsafe { std::env::set_var("GROQ_API_KEY", "gsk-test") };

        let hosted = ProviderArgs {
            provider: "groq".into(),
            model: None,
            api_key: None,
        };
        let built = hosted.client();

        match restore {
            Some(value) => unsafe { std::env::set_var("GROQ_API_KEY", value) },
            None => unsafe { std::env::remove_var("GROQ_API_KEY") },
        }
        assert!(built.is_ok(), "{:?}", built.err());
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
