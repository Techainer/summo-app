//! Writing the summary once the meeting is over.
//!
//! Everything this needs already existed and nothing called it: `summo_llm::prompt::summarize`
//! builds the request, `summo_vault::template` decides the shape, and `MeetingDoc::set_section`
//! writes the result back into the Markdown file. This is the wiring.
//!
//! Three constraints shape it.
//!
//! **It never runs during the meeting.** Summarising a transcript that is still growing produces a
//! summary of the first half, and the request competes with decoding for the same CPU. It is
//! triggered on stop.
//!
//! **It never blocks anything.** The transcript is already on disk and already correct before this
//! starts. A failed or slow summary costs a section, not a meeting — so failure is reported and
//! swallowed rather than propagated.
//!
//! **It is the only part of the pipeline that may leave the machine.** Speech recognition and
//! diarization are local by construction; the summary goes wherever the user pointed their LLM
//! setting, which may be Ollama on the same box or a hosted API. That asymmetry is the product, so
//! the code says it out loud rather than treating the LLM as just another step.

use std::path::Path;

use summo_core::{Error, MeetingId, Result, paths::Paths};
use summo_llm::{LlmClient, prompt};
use summo_vault::{
    meeting::MeetingDoc,
    template::{Template, Templates},
};

/// What a summary run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summarized {
    pub meeting: MeetingId,
    /// Template used, so the interface can say which shape was applied and offer to change it.
    pub template: String,
    /// Headings written into the document.
    pub sections: Vec<String>,
}

/// Less material than this is not worth a request.
///
/// A meeting that produced two lines is a false start or a test, and asking a model to summarise it
/// yields a paraphrase of the two lines plus an apology.
///
/// Counted across the transcript *and* the notes, because either one can carry the meeting. A
/// twenty-minute call somebody typed a page of notes during, on a laptop whose microphone caught
/// four words, was refused here with "the transcript is too short to summarise (31 characters)" —
/// while the thing worth summarising sat in the same document, unread. The notes are an input to
/// the summary; they are an input to the decision to make one.
pub(crate) const MIN_CHARACTERS: usize = 400;

/// How much there is to work with, in characters.
///
/// Shared with [`crate::draft`], which had its own copy of the `400` and its own transcript-only
/// count — so the same meeting could be refused a draft and granted a summary, or the reverse, and
/// the two answers came from two literals nobody would think to change together.
pub(crate) fn material(transcript: &str, notes: Option<&str>) -> usize {
    transcript.chars().count() + notes.map_or(0, |n| n.trim().chars().count())
}

/// Summarise one meeting and write the result into its Markdown file.
///
/// `template_id` overrides the automatic choice, which is what the interface passes when the user
/// picks a different shape and asks for a rewrite.
pub async fn run(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    template_id: Option<&str>,
) -> Result<Summarized> {
    let path = find_meeting_file(&paths.vault(), meeting)?;
    let mut doc = summo_vault::open(&paths.vault(), &path)?;

    let transcript = prompt::render_transcript(&doc.transcript);
    // What the user typed while it was happening, if they typed anything. Read from the document
    // rather than passed in: whoever asks for a summary — the button, the draft, an agent — gets
    // the same notes, because they are part of the meeting rather than part of the request.
    let notes = doc.section(summo_vault::meeting::NOTES_HEADING);
    let have = material(&transcript, notes);
    if have < MIN_CHARACTERS {
        return Err(Error::Other(format!(
            "there is too little here to summarise ({have} characters of transcript and notes)"
        )));
    }

    let templates = Templates::load_or_seed(&paths.templates())?;
    let template = choose(&templates, template_id, &doc)?;

    // A template may pin a language; otherwise the user's setting decides, and only if neither is
    // set does the model follow the transcript.
    let settings = summo_core::settings::Settings::load(&paths.settings()).ok();
    let configured = settings
        .as_ref()
        .map(|s| s.llm.language.as_str())
        .unwrap_or("");
    let language = [template.language.as_str(), configured]
        .into_iter()
        .find(|l| !l.is_empty())
        .unwrap_or("the language of the transcript");
    let messages = prompt::summarize_with(&transcript, notes, &template.instructions(), language);
    let response = client.complete(&messages).await?;

    let written = apply(&mut doc, &response, template);
    if written.is_empty() {
        return Err(Error::Other(
            "the model returned nothing that looks like the requested sections".into(),
        ));
    }

    summo_vault::write::write_atomically(&path, doc.to_markdown()?.as_bytes())?;

    Ok(Summarized {
        meeting: meeting.clone(),
        template: template.id.clone(),
        sections: written,
    })
}

/// Summarise in the background, reporting the outcome to the log rather than to a caller.
///
/// Detached on purpose. The meeting is already saved and correct by the time this starts, and the
/// window may well be closed — a user who stops a recording and shuts the lid should still come
/// back to a summary. Nothing downstream waits on it, so every failure is logged and swallowed.
/// Honours `settings.llm.summarize_on_stop`: a user who turned it off gets nothing, silently.
pub fn spawn(paths: Paths, meeting: MeetingId) {
    tokio::spawn(async move {
        let settings = match summo_core::settings::Settings::load(&paths.settings()) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(%meeting, error = %e, "no summary: cannot read settings");
                return;
            }
        };
        if !settings.llm.summarize_on_stop {
            return;
        }

        let provider = match resolve_provider(&settings) {
            Ok(provider) => provider,
            Err(e) => {
                // Not an error worth shouting about: plenty of users never configure an LLM, and
                // the product works without one.
                tracing::info!(%meeting, reason = %e, "no summary: no language model configured");
                return;
            }
        };
        let client = match LlmClient::new(provider) {
            Ok(client) => client,
            Err(e) => {
                tracing::warn!(%meeting, error = %e, "no summary: cannot build the client");
                return;
            }
        };
        match run(&paths, &client, &meeting, None).await {
            Ok(done) => tracing::info!(
                %meeting,
                template = %done.template,
                sections = done.sections.len(),
                "summary written"
            ),
            Err(e) => tracing::warn!(%meeting, error = %e, "summary failed"),
        }
    });
}

/// The provider from settings, with the key from the environment.
///
/// The key is deliberately not in `settings.json`; see `summo_core::settings::Llm`.
fn resolve_provider(settings: &summo_core::settings::Settings) -> Result<summo_llm::Provider> {
    let key = std::env::var("SUMMO_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty());
    summo_llm::Provider::resolve(
        &settings.llm.provider,
        settings.llm.model.as_deref(),
        key.as_deref(),
    )
}

fn choose<'a>(
    templates: &'a Templates,
    requested: Option<&str>,
    doc: &MeetingDoc,
) -> Result<&'a Template> {
    if let Some(id) = requested {
        return templates
            .get(id)
            .ok_or_else(|| Error::Other(format!("no template with id {id}")));
    }
    templates
        .best_for(&doc.title, &doc.frontmatter.tags)
        .ok_or_else(|| Error::Other("no summary templates are installed".into()))
}

/// Split the model's Markdown into sections and write each one into the document.
///
/// Only headings the template asked for are written. A model that invents an extra section is
/// ignored rather than allowed to add structure the user did not ask for, and one that omits a
/// section leaves that heading absent rather than empty.
fn apply(doc: &mut MeetingDoc, response: &str, template: &Template) -> Vec<String> {
    let produced = split_sections(response);
    let mut written = Vec::new();

    for section in &template.sections {
        let Some(body) = produced.get(section.heading.as_str()) else {
            continue;
        };
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        doc.set_section(&section.heading, body);
        written.push(section.heading.clone());
    }
    written
}

/// `## Heading` … into a map. Tolerates the model wrapping its answer in a code fence.
fn split_sections(markdown: &str) -> std::collections::BTreeMap<&str, String> {
    let mut out: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    let mut heading: Option<&str> = None;
    let mut buffer = String::new();

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(previous) = heading.take() {
                out.insert(previous, buffer.trim().to_string());
            }
            heading = Some(rest.trim());
            buffer.clear();
        } else if heading.is_some() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    if let Some(previous) = heading {
        out.insert(previous, buffer.trim().to_string());
    }
    out
}

/// Locate a meeting's file by id, anywhere under the vault.
///
/// Meetings move between folders, so the path is not derivable from the id — it has to be looked
/// up. The index already scans heads only, so this is cheap.
/// Locate a meeting's Markdown file by id.
///
/// Shared rather than duplicated: translation and export need the same lookup, and two scans that
/// disagree about which file is which meeting is the kind of bug that only shows up once somebody
/// renames a note.
pub fn find_meeting_file(vault: &Path, meeting: &MeetingId) -> Result<std::path::PathBuf> {
    let index = summo_vault::index::MeetingIndex::of_vault(vault)?;
    index
        .entries()
        .iter()
        .find(|entry| &entry.id == meeting)
        .map(|entry| vault.join(&entry.path))
        .ok_or_else(|| Error::Other(format!("no meeting with id {meeting}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_vault::template::parse;

    fn template() -> Template {
        parse(
            "t",
            "## Tóm tắt\nHai ba câu.\n\n## Quyết định\nMỗi ý một dòng.\n\n## Việc cần làm\nDanh sách.\n",
        )
        .expect("parse")
    }

    fn doc() -> MeetingDoc {
        let frontmatter = summo_vault::meeting::Frontmatter::new(
            MeetingId::from("01A".to_string()),
            "2026-08-10T09:00:00+07:00",
        );
        MeetingDoc::new(frontmatter, "Họp tuần")
    }

    /// The refusal counts both inputs, and this is the meeting it was getting wrong: the microphone
    /// caught almost nothing and the person typed the meeting down themselves. Transcript-only, that
    /// was "too short to summarise" over a page of their own writing.
    #[test]
    fn notes_count_towards_being_worth_summarising() {
        let barely = "Ừ. Vâng.";
        let typed = "Chốt ngân sách quý tới, Ngọc gửi bản dự trù trước thứ Sáu. ".repeat(10);
        assert!(
            material(barely, None) < MIN_CHARACTERS,
            "four words is not a meeting"
        );
        assert!(
            material(barely, Some(&typed)) >= MIN_CHARACTERS,
            "a page of notes is worth summarising however little was heard"
        );
    }

    /// Whitespace is not material. A body that is one newline must not push a false start over the
    /// line, which is what an untrimmed count would do for a note the editor autosaved empty.
    #[test]
    fn blank_notes_add_nothing() {
        let said = "a".repeat(100);
        assert_eq!(material(&said, Some("   \n\n  ")), material(&said, None));
    }

    #[test]
    fn sections_are_split_out_of_the_response() {
        let parsed = split_sections("## A\nfirst\n\n## B\nsecond\n");
        assert_eq!(parsed.get("A").map(String::as_str), Some("first"));
        assert_eq!(parsed.get("B").map(String::as_str), Some("second"));
    }

    /// Models wrap answers in ```markdown fences often enough that ignoring them is required.
    #[test]
    fn a_code_fence_around_the_answer_is_ignored() {
        let parsed = split_sections("```markdown\n## A\nfirst\n```\n");
        assert_eq!(parsed.get("A").map(String::as_str), Some("first"));
    }

    #[test]
    fn a_response_with_no_headings_yields_nothing() {
        assert!(split_sections("just prose\n").is_empty());
    }

    #[test]
    fn only_the_requested_sections_are_written() {
        let mut d = doc();
        let written = apply(
            &mut d,
            "## Tóm tắt\nĐã chốt.\n\n## Linh tinh\nKhông ai hỏi.\n",
            &template(),
        );
        assert_eq!(written, vec!["Tóm tắt"]);
        assert_eq!(d.section("Tóm tắt"), Some("Đã chốt."));
        assert!(
            d.section("Linh tinh").is_none(),
            "a section nobody asked for must not be added"
        );
    }

    #[test]
    fn an_omitted_section_is_left_absent_rather_than_empty() {
        let mut d = doc();
        let written = apply(&mut d, "## Tóm tắt\nNgắn thôi.\n", &template());
        assert_eq!(written, vec!["Tóm tắt"]);
        assert!(d.section("Quyết định").is_none());
    }

    #[test]
    fn a_section_the_model_left_blank_is_not_written() {
        let mut d = doc();
        let written = apply(&mut d, "## Tóm tắt\n\n## Quyết định\nCó.\n", &template());
        assert_eq!(written, vec!["Quyết định"]);
    }

    #[test]
    fn sections_are_written_in_template_order() {
        let mut d = doc();
        let written = apply(
            &mut d,
            "## Việc cần làm\nA\n\n## Tóm tắt\nB\n\n## Quyết định\nC\n",
            &template(),
        );
        // The model answered out of order; the document follows the template.
        assert_eq!(written, vec!["Tóm tắt", "Quyết định", "Việc cần làm"]);
    }

    #[test]
    fn choosing_an_unknown_template_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let templates = Templates::load_or_seed(dir.path()).unwrap();
        assert!(choose(&templates, Some("nope"), &doc()).is_err());
    }

    #[test]
    fn choosing_without_a_request_picks_by_tag() {
        let dir = tempfile::tempdir().unwrap();
        let templates = Templates::load_or_seed(dir.path()).unwrap();
        let mut d = doc();
        d.frontmatter.tags = vec!["standup".into()];
        assert_eq!(choose(&templates, None, &d).unwrap().id, "standup");
    }
}
