//! What we actually ask the model.
//!
//! Prompts are pure functions returning messages rather than strings interpolated at the call site,
//! for two reasons: they can be unit-tested without a network, and a user can read exactly what
//! their transcript is wrapped in before deciding to send it anywhere.

use serde::{Deserialize, Serialize};
use summo_core::segment::Segment;

use crate::provider::Message;

/// Terms that must be translated a particular way — product names, internal jargon, people.
///
/// Without this, a model helpfully translates the company's product name into a common noun, which
/// is the single most irritating failure in meeting translation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Glossary {
    /// Source term → required translation.
    pub terms: Vec<(String, String)>,
    /// Terms to leave untouched in any language.
    pub keep_as_is: Vec<String>,
}

impl Glossary {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.keep_as_is.is_empty()
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for (from, to) in &self.terms {
            out.push_str(&format!("- \"{from}\" must be translated as \"{to}\"\n"));
        }
        if !self.keep_as_is.is_empty() {
            out.push_str(&format!(
                "- Leave these exactly as written: {}\n",
                self.keep_as_is.join(", ")
            ));
        }
        out
    }
}

/// How much structure a summary should have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SummaryStyle {
    /// A few sentences. For a short call where structure would be noise.
    Brief,
    /// Overview, decisions, action items, open questions. The default for a working meeting.
    Standard,
    /// Standard plus a topic-by-topic walkthrough. For a long meeting somebody missed.
    Detailed,
}

impl SummaryStyle {
    fn instructions(self) -> &'static str {
        match self {
            Self::Brief => {
                "Write 2-4 sentences covering what the meeting was about and what was decided."
            }
            Self::Standard => {
                "Structure the summary as:\n\
                 ## Overview — two or three sentences.\n\
                 ## Decisions — what was actually decided, one bullet each. Omit the section if \
                 nothing was decided.\n\
                 ## Action items — `- [ ] owner — task — due date`. Only include items someone \
                 committed to; do not invent owners or dates.\n\
                 ## Open questions — anything raised and left unresolved."
            }
            Self::Detailed => {
                "Structure the summary as:\n\
                 ## Overview\n## Decisions\n## Action items (`- [ ] owner — task — due date`)\n\
                 ## Open questions\n## Discussion — a paragraph per topic, in the order discussed."
            }
        }
    }
}

/// System prompt shared by every task.
///
/// The instruction not to invent is doing real work: transcripts contain recognition errors, and a
/// model asked to summarise messy text will happily smooth it into confident fiction — inventing an
/// owner for an action item nobody accepted, or a deadline nobody said.
const GROUND_RULES: &str = "You are summarising a real meeting transcript produced by automatic \
speech recognition. The transcript contains recognition errors. Never invent facts, names, numbers \
or commitments that are not in the transcript. If something is unclear, say so rather than \
guessing. Where the transcript supports a claim, cite the timestamp in the form [t=MM:SS].";

/// Build the messages for a summary request.
#[must_use]
pub fn summarize(transcript: &str, style: SummaryStyle, language: &str) -> Vec<Message> {
    vec![
        Message::system(format!(
            "{GROUND_RULES}\n\nWrite the summary in {language}.\n\n{}",
            style.instructions()
        )),
        Message::user(format!("Transcript:\n\n{transcript}")),
    ]
}

/// Build the messages for a summary whose shape comes from a user-written template.
///
/// [`summarize`] covers the three built-in styles; this covers the case the templates exist for —
/// a standup, a sales call and an interview want different write-ups, and the user is allowed to
/// describe theirs in their own words. The ground rules are the same either way: they are what
/// stops a model smoothing a messy transcript into confident fiction, and a user editing a template
/// must not be able to switch them off by accident.
#[must_use]
pub fn summarize_with(transcript: &str, instructions: &str, language: &str) -> Vec<Message> {
    vec![
        Message::system(format!(
            "{GROUND_RULES}\n\nWrite the summary in {language}.\n\n{instructions}"
        )),
        Message::user(format!("Transcript:\n\n{transcript}")),
    ]
}

/// Build the messages for translating a run of utterances.
///
/// Several lines go in one request rather than one line per request. A sentence translated without
/// the ones around it loses pronouns and register, and a meeting produces enough lines that
/// per-line requests would be both slower and worse.
#[must_use]
pub fn translate(lines: &[&str], target_language: &str, glossary: &Glossary) -> Vec<Message> {
    let mut system = format!(
        "Translate meeting speech into {target_language}. Preserve the speaker's register — \
         informal speech stays informal. Keep each numbered line separate and output exactly the \
         same number of lines, numbered the same way. Translate only; add no commentary. If a line \
         is untranslatable or empty, output the line number with the original text."
    );
    if !glossary.is_empty() {
        system.push_str("\n\nTerminology:\n");
        system.push_str(&glossary.render());
    }

    let numbered: String = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}. {line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    vec![Message::system(system), Message::user(numbered)]
}

/// Build the request for a **dedicated translation model**, one line at a time.
///
/// A different shape from [`translate`] because it talks to a different kind of model, and the
/// difference is not stylistic. [`translate`] asks a general instruction-following model to obey a
/// paragraph of rules about numbered lines. A translation model — MiLMMT, and the NLLB and M2M
/// families before it — was trained on exactly one string:
///
/// ```text
/// Translate this from Vietnamese to Japanese:
/// Vietnamese: Chiều nay mình chốt lại spec API.
/// Japanese:
/// ```
///
/// Given Summo's numbered-batch prompt instead, a 1B translation model does not translate: measured
/// against MiLMMT-46-1B, it invented a fourth line in the *source* language and translated none of
/// the three it was given. So this is the price of admission for using a small local model at all,
/// and the trade it buys is a good one — 800 MB on the CPU, no key, no per-token cost, no text
/// leaving the machine.
///
/// `source` is optional. Naming it helps a little and getting it wrong costs almost nothing —
/// measured on the same model, mislabelling Vietnamese as Thai still produced a correct English
/// translation — so an unknown source language is a reason to omit the clause, not to guess.
///
/// A glossary is *not* accepted here on purpose. These models take no instructions; terminology
/// appended to the prompt would be translated as if it were part of the sentence.
#[must_use]
pub fn mt(line: &str, source: Option<&str>, target_language: &str) -> Vec<Message> {
    // One user turn, no system message. A system prompt is a chat-model convention, and prepending
    // one moves the model off the exact string it was trained to continue.
    vec![Message::user(mt_text(line, source, target_language))]
}

/// The same request as [`mt`], as the raw string a completion takes.
///
/// Two callers need this in two shapes: the HTTP path wraps it in a chat message, and the
/// in-process runtime in `summo-mt` continues it directly. They must be the *same string* — a
/// translation that changed depending on which path produced it would be a bug nobody could see
/// from either side.
#[must_use]
pub fn mt_text(line: &str, source: Option<&str>, target_language: &str) -> String {
    let target = crate::lang::name(target_language);
    let line = line.trim();

    match source.map(crate::lang::name).filter(|s| !s.is_empty()) {
        Some(source) => {
            format!("Translate this from {source} to {target}:\n{source}: {line}\n{target}:")
        }
        None => format!("Translate this to {target}:\n{line}\n{target}:"),
    }
}

/// Clean up a dedicated translation model's reply.
///
/// Small models continue past the sentence they were asked for: another `English:` turn, a line of
/// commentary, the source sentence repeated. Only the first line is the translation, and taking
/// more of it than that puts a model's inner monologue into a subtitle file.
///
/// Returns `None` for an empty result, so the caller leaves the original line in place — the same
/// contract as [`parse_translation`], for the same reason. A subtitle with a hole in it is not a
/// subtitle.
#[must_use]
pub fn parse_mt(response: &str) -> Option<String> {
    let first = response
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;

    // `English: hello` — the model echoing the label it was asked to continue after.
    let text = match first.split_once(':') {
        Some((label, rest)) if is_label(label) => rest.trim(),
        _ => first,
    };

    (!text.is_empty()).then(|| text.to_string())
}

/// Whether a prefix looks like the label the prompt ended with, rather than part of the sentence.
///
/// Deliberately narrow. "Deadline: Friday" is a sentence somebody said in a meeting and must
/// survive; "English" and "Chinese (Simplified)" are labels. Letters, spaces and brackets only, and
/// short.
fn is_label(prefix: &str) -> bool {
    let prefix = prefix.trim();
    !prefix.is_empty()
        && prefix.len() <= 24
        && prefix
            .chars()
            .all(|c| c.is_alphabetic() || c == ' ' || c == '(' || c == ')')
        && crate::lang::known(prefix)
}

/// Split a translated response back into one entry per input line.
///
/// Models drop or merge numbered lines often enough that this cannot assume success: the result is
/// aligned by number, and any line the model failed to return comes back as `None` so the caller
/// can leave the original in place instead of shifting every subsequent translation by one.
#[must_use]
pub fn parse_translation(response: &str, expected: usize) -> Vec<Option<String>> {
    let mut out = vec![None; expected];
    for line in response.lines() {
        let line = line.trim();
        let Some((number, text)) = line.split_once('.') else {
            continue;
        };
        let Ok(index) = number.trim().parse::<usize>() else {
            continue;
        };
        if index >= 1 && index <= expected {
            let text = text.trim();
            if !text.is_empty() {
                out[index - 1] = Some(text.to_string());
            }
        }
    }
    out
}

/// Build the messages for a question about one or more meetings.
#[must_use]
pub fn answer(question: &str, context: &str, language: &str) -> Vec<Message> {
    vec![
        Message::system(format!(
            "{GROUND_RULES}\n\nAnswer in {language}. Answer only from the excerpts provided. If \
             they do not contain the answer, say so plainly — do not fall back on general \
             knowledge. Cite the timestamp of each excerpt you rely on."
        )),
        Message::user(format!("Excerpts:\n\n{context}\n\nQuestion: {question}")),
    ]
}

/// What to write out of a meeting.
///
/// Not a free-text "style" field: each of these wants a different *shape*, and a shape is what the
/// model gets wrong when asked politely. An email needs a subject line and a greeting; a chat
/// message must fit in a glance or nobody reads it; a recap is for people who were not there and
/// therefore cannot be told "as discussed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compose {
    /// An email: subject line, greeting, body, sign-off.
    Email,
    /// A short message for a chat app — Slack, Teams, Zalo.
    Message,
    /// A recap for people who were not in the meeting.
    Recap,
    /// The decisions and who owes what, as a list.
    Actions,
}

impl Compose {
    fn shape(self) -> &'static str {
        match self {
            Self::Email => {
                "Write an email. First line: `Subject: …` and nothing else on it. Then a blank \
                 line, then a greeting, the body in short paragraphs, and a sign-off. Do not sign a \
                 name — leave the sign-off as a greeting the sender completes."
            }
            Self::Message => {
                "Write a chat message for Slack or Teams: under 80 words, no subject line, no \
                 greeting, no sign-off. Lead with the outcome. Bullets only if there is genuinely \
                 a list."
            }
            Self::Recap => {
                "Write a recap for people who were not at the meeting. No subject line. Say what \
                 was decided and what happens next. Never write \"as discussed\" or \"as you \
                 know\" — the reader was not there."
            }
            Self::Actions => {
                "List the decisions, then the actions as `- [ ] @person — what — when`. One line \
                 each. Only people who actually took something on. No preamble and no closing \
                 paragraph."
            }
        }
    }
}

/// How it should sound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    #[default]
    Neutral,
    Friendly,
    Formal,
}

impl Tone {
    fn describe(self) -> &'static str {
        match self {
            Self::Neutral => "Plain and direct. No enthusiasm the facts do not support.",
            Self::Friendly => "Warm and colloquial, the way a colleague writes. Still brief.",
            Self::Formal => "Formal and precise, for a client or an executive. No contractions.",
        }
    }
}

/// Draft something to send, out of what a meeting contained.
///
/// The hardest rule to get a model to keep here is the one that matters most: **invent nothing**. A
/// model asked to write a follow-up email will cheerfully add a deadline nobody agreed to, and the
/// user sends it, because a fluent email does not look like a wrong one. So the instruction is not
/// "be accurate" — it is to leave a marker where it wanted a fact it does not have, which is
/// visible in the draft and impossible to send by accident.
#[must_use]
pub fn compose(
    kind: Compose,
    tone: Tone,
    audience: Option<&str>,
    notes: &str,
    language: &str,
) -> Vec<Message> {
    let audience = match audience.map(str::trim).filter(|a| !a.is_empty()) {
        Some(who) => format!("The reader is: {who}. Write for them.\n\n"),
        None => String::new(),
    };
    // Deliberately not [`GROUND_RULES`]. Those end with "cite the timestamp in the form [t=MM:SS]",
    // which is right for a summary of a recording and absurd in an email to a customer — the first
    // draft this produced was a perfectly polite message with `[t=03:12]` in the middle of it. What
    // carries over is the part that matters everywhere: invent nothing.
    vec![
        Message::system(format!(
            "You are writing a message on behalf of somebody who was at a meeting. Its notes are \
             below. Never invent a fact, a name, a number, a date or a commitment that is not in \
             them. If something is needed and missing, write `[…]` in its place rather than \
             guessing, so the sender can see what to fill in. Never write a timestamp or a \
             citation — this is a message to a person, not a summary of a recording.\n\nWrite in \
             {language}.\n\n{}\n\n{}\n\n{audience}Return only the message itself — no \
             explanation of what you wrote and no Markdown code fence around it.",
            kind.shape(),
            tone.describe(),
        )),
        Message::user(format!("Notes from the meeting:\n\n{notes}")),
    ]
}

/// Pull `Subject: …` off the front of a composed email.
///
/// Returns `(subject, body)`, and `None` for the subject when the model did not write one — which
/// is correct for every kind but [`Compose::Email`], and which must not turn the first line of a
/// chat message into a subject line.
#[must_use]
pub fn split_subject(text: &str) -> (Option<String>, String) {
    let text = text.trim();
    let Some((first, rest)) = text.split_once('\n') else {
        return (None, text.to_string());
    };
    for prefix in ["Subject:", "Tiêu đề:", "Chủ đề:", "件名:", "主题:"] {
        if let Some(subject) = first.trim().strip_prefix(prefix) {
            return (
                Some(subject.trim().to_string()),
                rest.trim_start().to_string(),
            );
        }
    }
    (None, text.to_string())
}

/// Rewrite one selected passage, and nothing else.
///
/// The model is given the whole section for context but asked to return **only** the replacement
/// for the selected span. That is the difference between "sửa đoạn này" working and being
/// exhausting: a model asked to rewrite a section it was shown will quietly reword the sentences
/// nobody complained about, and the user has to re-read the whole thing to find out what changed.
/// Returning just the span means everything outside it stays byte-identical.
#[must_use]
pub fn revise_selection(
    section: &str,
    selection: &str,
    instruction: &str,
    language: &str,
) -> Vec<Message> {
    vec![
        Message::system(format!(
            "{GROUND_RULES}\n\nWrite in {language}.\n\nYou are editing one passage of a meeting \
             summary. You will be shown the whole section for context, then the passage to change \
             and what to change about it.\n\nReturn ONLY the replacement text for that passage. No \
             preamble, no explanation, no quotation marks around it, no Markdown heading. Do not \
             touch anything outside the passage. If the instruction cannot be followed from the \
             transcript, return the passage unchanged."
        )),
        Message::user(format!(
            "Section:\n\n{section}\n\n---\n\nPassage to change:\n\n{selection}\n\n---\n\n\
             Instruction: {instruction}"
        )),
    ]
}

/// Revise a whole draft in response to a message in the chat panel.
///
/// Unlike [`revise_selection`], the user has not said where — so the model has to decide, and the
/// contract is that it returns the entire draft again in the same section structure. Anything it
/// leaves out is a section it deleted, which is why the instruction says so explicitly.
#[must_use]
pub fn revise_draft(draft: &str, message: &str, transcript: &str, language: &str) -> Vec<Message> {
    vec![
        Message::system(format!(
            "{GROUND_RULES}\n\nWrite in {language}.\n\nYou are revising a draft summary of a \
             meeting, in conversation with the person who will publish it.\n\nReturn the COMPLETE \
             revised draft, using the same `## ` headings. Keep every section the user did not ask \
             you to change exactly as it is. Do not add a preamble or explain what you changed."
        )),
        Message::user(format!(
            "Transcript:\n\n{transcript}\n\n---\n\nCurrent draft:\n\n{draft}\n\n---\n\n{message}"
        )),
    ]
}

/// Render segments as a transcript for prompting, with timestamps so the model can cite them.
#[must_use]
pub fn render_transcript(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| {
            let speaker = s.speaker.as_ref().map_or("?", |sp| sp.as_str());
            let minutes = (s.t0 as u64) / 60;
            let seconds = (s.t0 as u64) % 60;
            format!("[{minutes:02}:{seconds:02}] {speaker}: {}", s.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod mt_tests {
    use super::*;

    #[test]
    fn the_prompt_is_the_exact_string_the_model_was_trained_on() {
        let messages = mt("Chiều nay chốt spec.", Some("vi"), "ja");
        assert_eq!(
            messages.len(),
            1,
            "no system turn: it moves the model off the template"
        );
        assert_eq!(
            messages[0].content,
            "Translate this from Vietnamese to Japanese:\nVietnamese: Chiều nay chốt spec.\nJapanese:"
        );
    }

    /// Tags reach this from the settings file and the translation file on disk; names reach it from
    /// a user who typed one. A model handed `vi` translates into whatever it guesses.
    #[test]
    fn tags_and_names_both_become_names() {
        assert_eq!(
            mt("x", Some("Vietnamese"), "English")[0].content,
            mt("x", Some("vi"), "en")[0].content
        );
    }

    /// Summo does not always know what language was spoken — the ASR model may have been left on
    /// auto. Guessing would be worse than saying nothing: the model translates fine without it.
    #[test]
    fn an_unknown_source_drops_the_clause_rather_than_guessing() {
        assert_eq!(
            mt("xin chào", None, "en")[0].content,
            "Translate this to English:\nxin chào\nEnglish:"
        );
        assert_eq!(
            mt("xin chào", Some("  "), "en")[0].content,
            mt("xin chào", None, "en")[0].content
        );
    }

    #[test]
    fn the_first_line_is_the_translation() {
        assert_eq!(
            parse_mt("Settle the API spec.").as_deref(),
            Some("Settle the API spec.")
        );
        assert_eq!(parse_mt("\n\n  hello  \n").as_deref(), Some("hello"));
    }

    /// What a 1B model does when it does not stop: it keeps going, and everything after the first
    /// line is its own continuation rather than the sentence asked for.
    #[test]
    fn a_model_that_kept_talking_is_cut_at_the_first_line() {
        let reply = "Settle the API spec.\nVietnamese: Chiều nay chốt spec.\nEnglish: Settle it.";
        assert_eq!(parse_mt(reply).as_deref(), Some("Settle the API spec."));
    }

    #[test]
    fn an_echoed_label_is_not_part_of_the_sentence() {
        assert_eq!(
            parse_mt("English: Settle the spec.").as_deref(),
            Some("Settle the spec.")
        );
        assert_eq!(
            parse_mt("Chinese (Simplified): 敲定规格。").as_deref(),
            Some("敲定规格。")
        );
    }

    /// The reason [`is_label`] checks against the language table rather than "anything before a
    /// colon": people say these sentences in meetings, and eating the first half of one is a silent
    /// data loss that only shows up when somebody reads the transcript back.
    #[test]
    fn a_sentence_that_merely_contains_a_colon_survives_whole() {
        assert_eq!(
            parse_mt("Deadline: Friday").as_deref(),
            Some("Deadline: Friday")
        );
        assert_eq!(
            parse_mt("Ngoc: I will send it").as_deref(),
            Some("Ngoc: I will send it")
        );
        assert_eq!(parse_mt("9:00 tomorrow").as_deref(), Some("9:00 tomorrow"));
    }

    /// The same contract as `parse_translation`: nothing back means the original stays, rather than
    /// a subtitle file with a hole in it.
    #[test]
    fn an_empty_reply_is_none_so_the_original_survives() {
        assert!(parse_mt("").is_none());
        assert!(parse_mt("   \n  \n").is_none());
        assert!(parse_mt("English:").is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::{SpeakerId, segment::Lane};

    #[test]
    fn every_prompt_forbids_inventing_facts() {
        // The failure this guards against is a model turning "maybe Ngọc could look at it" into an
        // action item assigned to Ngọc with a deadline.
        for messages in [
            summarize("...", SummaryStyle::Standard, "Vietnamese"),
            answer("what was decided?", "...", "English"),
        ] {
            let system = &messages[0].content;
            assert!(
                system.contains("Never invent"),
                "missing ground rules: {system}"
            );
        }
    }

    #[test]
    fn summary_styles_differ_in_structure() {
        let brief = summarize("x", SummaryStyle::Brief, "English")[0]
            .content
            .clone();
        let standard = summarize("x", SummaryStyle::Standard, "English")[0]
            .content
            .clone();
        let detailed = summarize("x", SummaryStyle::Detailed, "English")[0]
            .content
            .clone();

        assert!(!brief.contains("Action items"));
        assert!(standard.contains("Action items"));
        assert!(detailed.contains("Discussion"));
    }

    #[test]
    fn the_summary_language_is_stated_explicitly() {
        let messages = summarize("x", SummaryStyle::Standard, "Vietnamese");
        assert!(messages[0].content.contains("Vietnamese"));
    }

    #[test]
    fn translation_numbers_its_input() {
        let messages = translate(&["xin chào", "khỏe không"], "English", &Glossary::default());
        assert_eq!(messages[1].content, "1. xin chào\n2. khỏe không");
        assert!(messages[0].content.contains("English"));
    }

    #[test]
    fn a_glossary_reaches_the_model() {
        let glossary = Glossary {
            terms: vec![("máy chủ".into(), "server".into())],
            keep_as_is: vec!["Summo".into()],
        };
        let system = translate(&["x"], "English", &glossary)[0].content.clone();

        assert!(system.contains("\"máy chủ\" must be translated as \"server\""));
        assert!(system.contains("Summo"));
    }

    #[test]
    fn translations_are_realigned_by_number() {
        let parsed = parse_translation("1. hello\n2. how are you", 2);
        assert_eq!(parsed[0].as_deref(), Some("hello"));
        assert_eq!(parsed[1].as_deref(), Some("how are you"));
    }

    #[test]
    fn a_dropped_line_does_not_shift_the_rest() {
        // The model skipped line 2. Naive splitting would put line 3's translation onto line 2 and
        // silently mistranslate everything after it.
        let parsed = parse_translation("1. hello\n3. goodbye", 3);
        assert_eq!(parsed[0].as_deref(), Some("hello"));
        assert_eq!(parsed[1], None, "the missing line must stay missing");
        assert_eq!(parsed[2].as_deref(), Some("goodbye"));
    }

    #[test]
    fn commentary_around_the_translation_is_ignored() {
        let parsed = parse_translation(
            "Here is the translation:\n\n1. hello\n2. goodbye\n\nLet me know if you need changes.",
            2,
        );
        assert_eq!(parsed[0].as_deref(), Some("hello"));
        assert_eq!(parsed[1].as_deref(), Some("goodbye"));
    }

    #[test]
    fn out_of_range_line_numbers_are_discarded() {
        let parsed = parse_translation("1. ok\n9. stray", 2);
        assert_eq!(parsed[0].as_deref(), Some("ok"));
        assert_eq!(parsed[1], None);
    }

    #[test]
    fn transcripts_carry_timestamps_the_model_can_cite() {
        let mut a = Segment::new(0, Lane::Mic, "chốt dùng Rust", 724.0, 730.0);
        a.speaker = Some(SpeakerId::me());
        let mut b = Segment::new(1, Lane::System, "đồng ý", 731.0, 733.0);
        b.speaker = Some(SpeakerId::auto(0));

        let rendered = render_transcript(&[a, b]);
        assert_eq!(rendered, "[12:04] me: chốt dùng Rust\n[12:11] S1: đồng ý");
    }

    #[test]
    fn question_answering_refuses_to_leave_the_transcript() {
        let system = answer("q", "ctx", "English")[0].content.clone();
        assert!(
            system.contains("do not fall back on general"),
            "the model must not answer from world knowledge: {system}"
        );
    }
}
