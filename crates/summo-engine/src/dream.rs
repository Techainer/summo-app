//! What an agent does overnight.
//!
//! Two files grow every day and neither ever improves: `MEMORY.md`, one line per thing the agent
//! was told, and `HABITS.md`, one line per thing it was asked. After a month the memory holds the
//! same fact written three ways, a correction sitting above the thing it corrected, and a note
//! about a project that finished in March. Every one of those goes into every prompt.
//!
//! So once a day, if the user turns it on, the agent sleeps on it: reads what it knows, and writes
//! back a shorter version that says the same things. Duplicates merge, the correction wins over
//! what it corrected, and what is plainly finished falls away.
//!
//! ## Three rules that make this safe to leave running
//!
//! * **Nothing is invented.** The model is given only what the agent already wrote down and asked
//!   to compress it. A dream that could add a belief would be a system that quietly rewrites what
//!   it thinks of you, overnight, forever.
//! * **Nothing is lost.** The previous memory is copied into `DREAMS.md` before the new one is
//!   written. Every night is a numbered entry with what changed, so a bad dream is one file to open
//!   and one block to paste back.
//! * **It refuses rather than guesses.** An empty answer, an answer longer than what it started
//!   with, or an answer that dropped more than half the lines is discarded and the memory left
//!   exactly as it was. A consolidation that deletes most of an agent's memory is not a
//!   consolidation, and the failure mode of "the model returned nothing useful" must not be
//!   "the agent forgot everything".
//!
//! ## Why it is off by default
//!
//! It costs a language-model call per agent per day, for a benefit nobody asked for on the first
//! day of use — an empty memory has nothing to consolidate. It is a switch in Settings, and the
//! daemon only wakes it after the hour set there, never while a meeting is being recorded.

use serde::{Deserialize, Serialize};
use summo_agent::{habits, memory, roster::Roster};
use summo_core::{Error, Result, paths::Paths};
use summo_llm::LlmClient;

/// What a night's sleep did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dreamt {
    pub agent: String,
    pub day: String,
    pub before: usize,
    pub after: usize,
    /// Present when the dream was discarded, saying why. Shown rather than logged: a feature that
    /// silently does nothing is one the user cannot tell from a feature that is broken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// The most a night may forget, as a fraction of what there was.
///
/// Half. Merging three ways of saying one thing is the job; coming back with two lines out of forty
/// is a model that failed to follow the instruction, and applying it would cost a month of memory
/// to save a prompt.
const KEEP_AT_LEAST: f32 = 0.5;

/// Consolidate one agent's memory. `None` means every agent in the roster.
pub async fn run(
    paths: &Paths,
    client: &LlmClient,
    slug: Option<&str>,
    day: &str,
) -> Result<Vec<Dreamt>> {
    let roster = Roster::load_or_seed(&paths.agents())?;
    let agents: Vec<_> = match slug {
        Some(slug) => roster
            .get(slug)
            .map(|agent| vec![agent.clone()])
            .ok_or_else(|| Error::msg("dream.no_agent", format!("không có agent {slug}")))?,
        None => roster.all().cloned().collect(),
    };

    let asks = habits::habits(&habits::load(&paths.agents()));
    let mut out = Vec::new();
    for agent in agents {
        out.push(dream_one(&agent, client, &asks, day).await?);
    }
    Ok(out)
}

async fn dream_one(
    agent: &summo_agent::roster::AgentDef,
    client: &LlmClient,
    asks: &[habits::Habit],
    day: &str,
) -> Result<Dreamt> {
    let path = agent.memory_path();
    let before = memory::load(&path);
    let mut dreamt = Dreamt {
        agent: agent.slug.clone(),
        day: day.to_string(),
        before: before.len(),
        after: before.len(),
        refused: None,
    };

    // Nothing to think about. Not an error, and not worth a request to a model: an agent used twice
    // has a memory that is already as short as it can be.
    if before.len() < 4 {
        dreamt.refused = Some("chưa có gì để ôn lại".into());
        return Ok(dreamt);
    }

    let messages = summo_llm::prompt::consolidate(
        &memory::render(&before),
        &habits::render(asks),
        &agent.head.name,
    );
    let response = client.complete(&messages).await?;
    let proposed = lines(&response);

    if let Some(reason) = refuse(&before, &proposed) {
        dreamt.refused = Some(reason);
        return Ok(dreamt);
    }

    // The old memory first, then the new one. In that order, so a crash between the two leaves the
    // copy rather than leaving nothing.
    archive(agent, day, &memory::render(&before), &proposed.join("\n"))?;
    memory::replace(&path, &proposed, day)?;
    dreamt.after = proposed.len();
    Ok(dreamt)
}

/// Whether to throw the night away, and why.
fn refuse(before: &[memory::Fact], proposed: &[String]) -> Option<String> {
    if proposed.is_empty() {
        return Some("model không trả về gì".into());
    }
    if proposed.len() > before.len() {
        // Consolidation that produces more lines than it was given is not consolidation; it is a
        // model elaborating, which is the one thing this must never do to a memory.
        return Some("model viết dài ra thay vì gọn lại".into());
    }
    #[allow(clippy::cast_precision_loss)]
    let kept = proposed.len() as f32 / before.len() as f32;
    if kept < KEEP_AT_LEAST {
        return Some(format!(
            "bỏ mất quá nhiều ({} → {} dòng)",
            before.len(),
            proposed.len()
        ));
    }
    None
}

/// Bullets out of whatever the model returned.
fn lines(response: &str) -> Vec<String> {
    response
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let text = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or_else(|| {
                    // A model that answered in plain sentences is still answering; only a heading
                    // or a fence is noise.
                    if line.starts_with('#') || line.starts_with("```") {
                        ""
                    } else {
                        line
                    }
                })
                .trim();
            (!text.is_empty()).then(|| text.to_string())
        })
        .take(memory::MAX_LINES)
        .collect()
}

/// Keep the night in `DREAMS.md`, newest last.
///
/// The whole previous memory, verbatim, because that is what makes this reversible by a person
/// with a text editor and no undo history.
fn archive(
    agent: &summo_agent::roster::AgentDef,
    day: &str,
    before: &str,
    after: &str,
) -> Result<()> {
    let path = agent.dir.join("DREAMS.md");
    let mut out = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        String::from(
            "# Những đêm đã ngủ\n\nMỗi mục là trí nhớ trước và sau một lần ôn lại. Không thích \
             thì chép phần \"Trước\" ngược lại vào MEMORY.md.\n",
        )
    });
    out.push_str(&format!(
        "\n## {day}\n\n### Trước\n\n{before}\n\n### Sau\n\n{after}\n"
    ));
    summo_vault::write::write_atomically(&path, out.as_bytes())
}

/// The last night, for the interface to show and the scheduler to avoid repeating.
///
/// `~/.summo/dreams.json`, outside the vault: it is a fact about this installation's clock, not
/// about the notes, and syncing it would make two machines argue about whose night it was.
#[must_use]
pub fn last(paths: &Paths) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(paths.root().join("dreams.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write down that tonight happened, whatever it did.
///
/// Recorded even when every agent refused, and that is deliberate: without it a daemon left
/// running would retry a refusal every half hour, which is a language-model call every half hour
/// for an answer that will not change until the memory does.
pub fn mark(paths: &Paths, day: &str, dreamt: &[Dreamt]) {
    let path = paths.root().join("dreams.json");
    let record = serde_json::json!({ "day": day, "agents": dreamt });
    if let Ok(bytes) = serde_json::to_vec_pretty(&record) {
        let _ = summo_vault::write::write_atomically(&path, &bytes);
    }
}

/// Whether tonight is still owed, given the clock and what was written down.
#[must_use]
pub fn due(paths: &Paths, today: &str, hour: u8, after: u8, recording: bool) -> bool {
    if recording || hour < after {
        return false;
    }
    last(paths)
        .and_then(|record| {
            record
                .get("day")
                .and_then(|d| d.as_str())
                .map(|d| d != today)
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(n: usize) -> Vec<memory::Fact> {
        (0..n)
            .map(|i| memory::Fact {
                learned: "2026-08-14".into(),
                text: format!("điều {i}"),
            })
            .collect()
    }

    /// The failure that would matter: a model answers with nothing, or with one line, and an agent
    /// that knew forty things knows one. Refused, and the memory is untouched.
    #[test]
    fn a_night_that_forgets_too_much_is_thrown_away() {
        let before = facts(40);
        assert!(refuse(&before, &[]).is_some());
        assert!(refuse(&before, &["chỉ còn một dòng".into()]).is_some());
        assert!(
            refuse(
                &before,
                &facts(10).iter().map(|f| f.text.clone()).collect::<Vec<_>>()
            )
            .is_some()
        );
    }

    #[test]
    fn a_night_that_writes_more_than_it_read_is_thrown_away() {
        let before = facts(5);
        let longer: Vec<String> = (0..9).map(|i| format!("dòng {i}")).collect();
        assert!(refuse(&before, &longer).is_some());
    }

    #[test]
    fn a_real_consolidation_is_kept() {
        let before = facts(10);
        let after: Vec<String> = (0..7).map(|i| format!("gọn {i}")).collect();
        assert!(refuse(&before, &after).is_none());
    }

    #[test]
    fn a_night_happens_once_and_not_before_its_hour() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.root()).unwrap();

        assert!(
            !due(&paths, "2026-08-14", 1, 3, false),
            "not before the hour"
        );
        assert!(
            !due(&paths, "2026-08-14", 5, 3, true),
            "never while recording"
        );
        assert!(
            due(&paths, "2026-08-14", 5, 3, false),
            "owed, and nothing written down"
        );

        mark(&paths, "2026-08-14", &[]);
        assert!(
            !due(&paths, "2026-08-14", 5, 3, false),
            "already slept tonight"
        );
        // Even a night where every agent refused counts as slept: otherwise the daemon asks a
        // model the same question every half hour until the memory changes.
        assert!(
            due(&paths, "2026-08-15", 5, 3, false),
            "tomorrow is owed again"
        );
    }

    #[test]
    fn bullets_headings_and_fences_are_sorted_out() {
        let parsed = lines(
            "# Memory\n\n- Ngọc phụ trách sản phẩm\n* Bình lo hợp đồng\n\nkhông gạch đầu dòng\n```\n",
        );
        assert_eq!(
            parsed,
            vec![
                "Ngọc phụ trách sản phẩm",
                "Bình lo hợp đồng",
                "không gạch đầu dòng"
            ]
        );
    }

    #[test]
    fn a_model_that_rambles_is_capped() {
        let long = (0..memory::MAX_LINES + 20)
            .map(|i| format!("- dòng {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(lines(&long).len(), memory::MAX_LINES);
    }
}
