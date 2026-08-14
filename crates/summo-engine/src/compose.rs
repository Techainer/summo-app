//! Turning a meeting into something you send.
//!
//! Everything up to here ends at a good note. That is where the work actually is for most people:
//! the meeting produced a decision, and now somebody has to tell the three people who were not in
//! it — and they do it by scrolling the transcript in one window and typing an email in another.
//! Summo already holds every fact that email needs.
//!
//! So: a follow-up email, a chat message, a recap for people who missed it, or the decisions and
//! actions as a list. Four shapes rather than a free-text style box, because the shape is what a
//! model gets wrong; see [`summo_llm::prompt::Compose`].
//!
//! Three deliberate limits:
//!
//! * **Nothing is sent.** The draft comes back to the screen; the user copies it, or opens their
//!   own mail client through a `mailto:` link with the subject and body already filled in. Summo
//!   holds no mail credentials, sends nothing on anybody's behalf, and cannot send the wrong
//!   version of a message to a customer.
//! * **The summary is the source, not the transcript.** A summary the user confirmed is the
//!   version of events they agreed with. Falling straight to the transcript would draft from
//!   whatever was said, including the part that was corrected two minutes later.
//! * **A gap is marked, not filled.** The prompt requires `[…]` where a fact is missing. A model
//!   asked to write a follow-up will otherwise invent a deadline, and a fluent email does not look
//!   like a wrong one.

use serde::{Deserialize, Serialize};
use summo_core::{Error, MeetingId, Result, paths::Paths};
use summo_llm::{
    LlmClient,
    prompt::{Compose, Tone},
};
use summo_vault::meeting::MeetingDoc;

/// A drafted message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Composed {
    pub kind: Compose,
    /// Present for an email, absent for everything else.
    pub subject: Option<String>,
    pub body: String,
    /// `mailto:` with the subject and body filled in, for the "open in my mail app" button.
    ///
    /// Only for an email. A `mailto:` holding a Slack message would open the wrong application and
    /// look like a bug in the button rather than a decision.
    pub mailto: Option<String>,
}

/// Ask for a draft.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[serde(default = "default_kind")]
    pub kind: Compose,
    #[serde(default)]
    pub tone: Tone,
    /// Who is going to read it, in the user's own words: "khách hàng ACME", "team kỹ thuật".
    #[serde(default)]
    pub audience: Option<String>,
    /// Language to write in, or the one configured for summaries.
    #[serde(default)]
    pub language: Option<String>,
}

fn default_kind() -> Compose {
    Compose::Email
}

/// How much of a meeting to hand the model when there is no summary.
///
/// A transcript is long, the useful part of it is at the start and the end, and a request holding
/// forty thousand characters is slow, expensive and — for a local model with a small context —
/// silently truncated at whichever end the runtime chooses.
const TRANSCRIPT_BUDGET: usize = 6_000;

/// Draft a message from a meeting.
pub async fn compose(
    paths: &Paths,
    client: &LlmClient,
    meeting: &MeetingId,
    request: &Request,
) -> Result<Composed> {
    let notes = source(paths, meeting)?;
    if notes.trim().chars().count() < 80 {
        return Err(Error::msg(
            "compose.empty",
            "buổi này chưa có đủ nội dung để soạn",
        ));
    }

    let language = request.language.clone().unwrap_or_else(|| {
        summo_core::settings::Settings::load(&paths.settings())
            .ok()
            .map(|s| s.llm.language)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "the language of the notes".into())
    });

    let messages = summo_llm::prompt::compose(
        request.kind,
        request.tone,
        request.audience.as_deref(),
        &notes,
        &language,
    );
    let response = client.complete(&messages).await?;
    Ok(finish(request.kind, &response))
}

/// Split what came back and build the link, with no model involved.
///
/// Separate from [`compose`] so the parsing is testable without a provider — the subject line is
/// the part that breaks, and it breaks per language.
#[must_use]
pub fn finish(kind: Compose, response: &str) -> Composed {
    let cleaned = strip_fence(response);
    let (subject, body) = match kind {
        Compose::Email => summo_llm::prompt::split_subject(cleaned),
        _ => (None, cleaned.trim().to_string()),
    };
    let mailto = (kind == Compose::Email).then(|| mailto(subject.as_deref(), &body));
    Composed {
        kind,
        subject,
        body,
        mailto,
    }
}

/// A `mailto:` the operating system will hand to whichever mail application is set up.
///
/// No recipient: Summo does not know the address and guessing one from the attendee list is how a
/// draft goes to the wrong person. The user's mail client asks, which it was going to do anyway.
fn mailto(subject: Option<&str>, body: &str) -> String {
    let encoded = |s: &str| {
        let mut out = String::with_capacity(s.len());
        for byte in s.as_bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(*byte as char);
                }
                other => out.push_str(&format!("%{other:02X}")),
            }
        }
        out
    };
    let mut url = String::from("mailto:?");
    if let Some(subject) = subject {
        url.push_str(&format!("subject={}&", encoded(subject)));
    }
    url.push_str(&format!("body={}", encoded(body)));
    url
}

/// Keep the draft, as a note in the vault.
///
/// Because the alternative is that it lives in a text box until the tab closes. A note is a file
/// the user already knows how to find, it is searchable and the assistant can read it, and the
/// meeting it came from is linked from it.
pub fn save(paths: &Paths, meeting: &MeetingId, title: &str, body: &str) -> Result<MeetingId> {
    let day = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
        .to_string();
    let body = format!("{}\n\n---\n\nTừ buổi họp `{meeting}`.\n", body.trim());
    let (id, _) = summo_vault::note::create(paths, title, &day, &body)?;
    Ok(id)
}

/// What the model is shown: the confirmed write-up, or the transcript if there is not one.
fn source(paths: &Paths, meeting: &MeetingId) -> Result<String> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;
    let entry = index
        .entries()
        .iter()
        .find(|e| &e.id == meeting)
        .ok_or_else(|| Error::msg("compose.missing", format!("không có buổi nào id {meeting}")))?;
    let path = vault.join(&entry.path);
    let markdown = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let doc = MeetingDoc::parse(&markdown)?;

    let mut written = String::new();
    if !doc.body.trim().is_empty() {
        written.push_str(doc.body.trim());
        written.push_str("\n\n");
    }
    for section in &doc.sections {
        written.push_str(&format!(
            "## {}\n{}\n\n",
            section.heading,
            section.body.trim()
        ));
    }

    if written.trim().chars().count() >= 200 {
        return Ok(format!("# {}\n\n{written}", doc.title));
    }

    // No summary worth the name. The transcript, trimmed from the middle: an opening and an ending
    // are what a meeting is about, and the middle is where the tangent about lunch lives.
    let transcript = summo_llm::prompt::render_transcript(&doc.transcript);
    Ok(format!(
        "# {}\n\n{}\n\n{written}",
        doc.title,
        clip(&transcript)
    ))
}

/// Keep the start and the end, and say where the cut is.
fn clip(text: &str) -> String {
    if text.chars().count() <= TRANSCRIPT_BUDGET {
        return text.to_string();
    }
    let half = TRANSCRIPT_BUDGET / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = {
        let all: Vec<char> = text.chars().collect();
        all[all.len() - half..].iter().collect()
    };
    format!("{head}\n\n[…]\n\n{tail}")
}

fn strip_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = rest.split_once('\n').map_or(rest, |(_, body)| body);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_gets_its_subject_split_off_and_a_link_built() {
        let out = finish(
            Compose::Email,
            "Subject: Chốt giá gói doanh nghiệp\n\nChào anh,\n\nNhư đã trao đổi…",
        );
        assert_eq!(out.subject.as_deref(), Some("Chốt giá gói doanh nghiệp"));
        assert!(out.body.starts_with("Chào anh"));
        let link = out.mailto.unwrap();
        assert!(link.starts_with("mailto:?subject="));
        // Percent-encoded, because a raw newline or `&` in a URL truncates the body at the point
        // it appears — the mail client opens with half a message and nobody notices which half.
        assert!(!link.contains(' ') && !link.contains('\n'));
        assert!(link.contains("%C3%A1") || link.contains("%E1%BB"));
    }

    /// The first line of a chat message is a sentence, not a subject, and promoting it to one
    /// deletes it from the body.
    #[test]
    fn a_chat_message_keeps_its_first_line() {
        let out = finish(Compose::Message, "Chốt giá 12 triệu.\nBình gửi hợp đồng.");
        assert!(out.subject.is_none());
        assert!(out.body.starts_with("Chốt giá"));
        assert!(out.mailto.is_none(), "a chat message is not an email");
    }

    #[test]
    fn a_fenced_answer_is_unwrapped() {
        let out = finish(Compose::Recap, "```markdown\nHọp đã chốt giá.\n```");
        assert_eq!(out.body, "Họp đã chốt giá.");
    }

    #[test]
    fn a_long_transcript_is_clipped_from_the_middle() {
        let text = "a".repeat(TRANSCRIPT_BUDGET * 2);
        let clipped = clip(&text);
        assert!(clipped.chars().count() < text.chars().count());
        assert!(clipped.contains("[…]"));
    }
}
