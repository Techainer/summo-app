//! Asking questions about what was said.
//!
//! "Ngọc nói gì về ngân sách?" is a different operation from summarising, and the difference is
//! where the answer may come from. A summary works on one transcript the user just recorded. An
//! answer works on a search across everything, and the model must be held to *only* what the search
//! returned — a model that falls back on general knowledge here will confidently tell the user what
//! a colleague said in a meeting that never happened.
//!
//! So the shape is: search the vault, hand the excerpts over with their timestamps, and require
//! citations. `summo_llm::prompt::answer` already carries that instruction; this supplies the
//! excerpts and makes the citations clickable.

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};
use summo_llm::{LlmClient, prompt};

/// How many meetings to draw excerpts from.
///
/// Enough to answer a question that spans a few conversations, few enough that the excerpts do not
/// crowd out the model's ability to reason over them.
const MAX_MEETINGS: usize = 6;

/// Where an answer came from, so the user can check it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub meeting: String,
    pub title: String,
    pub day: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub question: String,
    pub text: String,
    /// The meetings whose excerpts were shown to the model, in the order they were given.
    pub sources: Vec<Source>,
}

/// Answer a question from the vault, or say that the vault does not contain the answer.
pub async fn ask(paths: &Paths, client: &LlmClient, question: &str) -> Result<Answer> {
    let question = question.trim();
    if question.is_empty() {
        return Err(Error::Other("hỏi gì thì phải có câu hỏi".into()));
    }

    let library = summo_vault::library::Library::new(paths.clone());
    let hits = library.search(question, MAX_MEETINGS)?;

    if hits.is_empty() {
        return Ok(Answer {
            question: question.to_string(),
            text: "Không tìm thấy buổi họp nào nhắc tới chuyện này.".into(),
            sources: Vec::new(),
        });
    }

    let mut context = String::new();
    let mut sources = Vec::new();
    for hit in &hits {
        context.push_str(&format!(
            "### {} ({}, id:{})\n",
            hit.meeting.title, hit.meeting.day, hit.meeting.id
        ));
        for excerpt in &hit.excerpts {
            // The timestamp is what makes a citation checkable, so it travels with the text.
            match excerpt.t0 {
                Some(t) => context.push_str(&format!(
                    "[t={}] {} — {}\n",
                    clock(t),
                    excerpt.speaker.as_deref().unwrap_or("?"),
                    excerpt.text.trim()
                )),
                None => context.push_str(&format!("{}\n", excerpt.text.trim())),
            }
        }
        context.push('\n');
        sources.push(Source {
            meeting: hit.meeting.id.to_string(),
            title: hit.meeting.title.clone(),
            day: hit.meeting.day.clone(),
        });
    }

    let language = summo_core::settings::Settings::load(&paths.settings())
        .ok()
        .map(|s| s.llm.language)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "tiếng Việt".into());

    let messages = prompt::answer(question, &context, &language);
    let text = client.complete(&messages).await?;

    Ok(Answer {
        question: question.to_string(),
        text: text.trim().to_string(),
        sources,
    })
}

/// `mm:ss`, matching the `[t=MM:SS]` form the prompt asks the model to cite.
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_match_the_citation_form_the_prompt_asks_for() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(64.7), "01:04");
        assert_eq!(clock(3_725.0), "62:05", "past an hour, minutes keep counting");
    }

    #[test]
    fn a_negative_timestamp_does_not_wrap_around() {
        assert_eq!(clock(-5.0), "00:00");
    }

    #[tokio::test]
    async fn an_empty_question_is_refused_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let client = summo_llm::LlmClient::new(summo_llm::Provider::ollama("x")).unwrap();
        assert!(ask(&paths, &client, "   ").await.is_err());
    }

    /// An empty vault must answer "nothing here", not invent one — and must not spend a request
    /// finding that out.
    #[tokio::test]
    async fn a_vault_with_nothing_relevant_says_so_without_asking_a_model() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();

        // Points at a port nothing is listening on: reaching the model would hang or error, so
        // passing proves no request was made.
        let client =
            summo_llm::LlmClient::new(summo_llm::Provider::custom("x", "https://127.0.0.1:1", "m"))
                .unwrap();

        let answer = ask(&paths, &client, "ngân sách").await.expect("answer");
        assert!(answer.sources.is_empty());
        assert!(answer.text.contains("Không tìm thấy"), "{}", answer.text);
    }
}
