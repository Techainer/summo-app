//! What this person keeps asking for.
//!
//! [`crate::memory`] holds what an agent was *told* to remember: who people are, what the team calls
//! things. This holds something the agent is never told and can only notice — that after every
//! customer call the same person asks for the same write-up, in the same words, and has done four
//! times now.
//!
//! That is the difference between an assistant and a text box. A text box is as good on day ninety
//! as on day one. An assistant that has watched you work should, by the fourth time, offer the
//! thing before you ask, and produce it to the standard the first three established.
//!
//! ## What is recorded, and what is not
//!
//! One line per instruction the user gave an agent: the date, the instruction as they typed it, and
//! the meeting it was about. Not the answer — that is a note in the vault already — and not
//! anything the agent inferred about the user. A habits file that accumulated conclusions ("prefers
//! concise writing") would be a file of guesses nobody can check, and the first wrong one would
//! quietly steer every answer afterwards.
//!
//! It is `vault/agents/HABITS.md`, ordinary Markdown, one bullet per ask. Delete a line and the
//! habit is gone; delete the file and the agent is new again. That property is why this is a
//! document rather than a store, and it is the same argument as [`crate::memory`].
//!
//! ## Installation-wide, not per agent
//!
//! A habit belongs to the person, not to whichever agent happened to answer. "Viết báo cáo sau
//! họp" asked of the coordinator on Monday and the scribe on Thursday is one habit, and splitting
//! it per agent would mean neither noticed.

use std::path::{Path, PathBuf};

use summo_core::{Error, Result};

/// How many asks to keep. Older ones fall off the top.
///
/// Two hundred is roughly a year of ordinary use. The cap is here for the same reason
/// [`crate::memory::MAX_LINES`] is: a file that only grows becomes a prompt that only grows.
pub const MAX_ASKS: usize = 200;

/// How many times something must be asked before it is a habit rather than a Tuesday.
///
/// Twice. Suggesting after one is how an assistant offers to do again the thing somebody tried
/// once and disliked; waiting for five means the offer arrives long after the person has built the
/// habit of doing it by hand.
pub const REPEATS: usize = 2;

/// One thing the user asked an agent to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// ISO date.
    pub day: String,
    pub instruction: String,
    /// The meeting it was about, if it was about one.
    pub meeting: Option<String>,
}

/// A habit: the same request, more than once.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Habit {
    /// The most recent phrasing, which is the one to offer back — people refine how they ask.
    pub instruction: String,
    pub times: usize,
    pub last: String,
}

#[must_use]
pub fn path(agents_dir: &Path) -> PathBuf {
    agents_dir.join("HABITS.md")
}

/// Read the log.
pub fn load(agents_dir: &Path) -> Vec<Ask> {
    let Ok(text) = std::fs::read_to_string(path(agents_dir)) else {
        return Vec::new();
    };
    text.lines().filter_map(parse_line).collect()
}

/// `- 2026-08-14 — viết báo cáo sau họp — 01ABC`
fn parse_line(line: &str) -> Option<Ask> {
    let rest = line.trim().strip_prefix("- ")?;
    let (day, rest) = rest.split_once(" — ")?;
    if day.len() != 10 {
        return None;
    }
    let (instruction, meeting) = match rest.rsplit_once(" — ") {
        Some((instruction, meeting)) => (instruction, Some(meeting.trim().to_string())),
        None => (rest, None),
    };
    let instruction = instruction.trim();
    (!instruction.is_empty()).then(|| Ask {
        day: day.to_string(),
        instruction: instruction.to_string(),
        meeting: meeting.filter(|m| !m.is_empty()),
    })
}

/// Write one down.
///
/// Called after an instruction is *given*, not after it succeeds: an ask that failed is still what
/// the person wanted, and refusing to learn from it means the habit only forms when the machinery
/// happens to work.
pub fn record(
    agents_dir: &Path,
    day: &str,
    instruction: &str,
    meeting: Option<&str>,
) -> Result<()> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Ok(());
    }

    let mut asks = load(agents_dir);
    asks.push(Ask {
        day: day.to_string(),
        instruction: instruction.replace('\n', " "),
        meeting: meeting.map(str::to_string),
    });
    if asks.len() > MAX_ASKS {
        asks.drain(..asks.len() - MAX_ASKS);
    }

    let mut out = String::from(
        "# Thói quen\n\nNhững việc bạn từng nhờ agent. Xoá dòng nào cũng được — agent sẽ quên.\n\n",
    );
    for ask in &asks {
        out.push_str(&format!("- {} — {}", ask.day, ask.instruction));
        if let Some(meeting) = &ask.meeting {
            out.push_str(&format!(" — {meeting}"));
        }
        out.push('\n');
    }

    std::fs::create_dir_all(agents_dir).map_err(|e| Error::io(agents_dir, e))?;
    summo_vault::write::write_atomically(&path(agents_dir), out.as_bytes())
}

/// The requests that have become habits, most-asked first.
#[must_use]
pub fn habits(asks: &[Ask]) -> Vec<Habit> {
    let mut groups: Vec<(String, Vec<&Ask>)> = Vec::new();
    for ask in asks {
        let key = normalise(&ask.instruction);
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, members)) => members.push(ask),
            None => groups.push((key, vec![ask])),
        }
    }

    let mut out: Vec<Habit> = groups
        .into_iter()
        .filter(|(_, members)| members.len() >= REPEATS)
        .map(|(_, members)| Habit {
            // The latest phrasing, not the first: "viết email cho khách" beats the version from
            // three months ago that the person has since stopped using.
            instruction: members
                .last()
                .map(|a| a.instruction.clone())
                .unwrap_or_default(),
            times: members.len(),
            last: members.last().map(|a| a.day.clone()).unwrap_or_default(),
        })
        .collect();
    out.sort_by(|a, b| b.times.cmp(&a.times).then(b.last.cmp(&a.last)));
    out
}

/// Two ways of asking the same thing, reduced to one key.
///
/// Case and punctuation only. Not stemming and not embeddings: this decides whether to *offer*
/// something, and an offer that is subtly the wrong thing is worse than no offer — a person who
/// types a genuinely different instruction should get a genuinely different suggestion.
fn normalise(instruction: &str) -> String {
    instruction
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The habits, as a paragraph for a system prompt.
///
/// Given to the model so a repeated request is answered the way the earlier ones were, rather than
/// re-invented each time — which is the actual complaint about assistants: the fourth report looks
/// nothing like the first three.
#[must_use]
pub fn render(habits: &[Habit]) -> String {
    if habits.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "Người dùng thường nhờ những việc sau. Nếu yêu cầu lần này là một trong số đó, hãy làm \
         đúng cách bạn đã làm những lần trước — cùng bố cục, cùng độ dài, cùng giọng văn:\n",
    );
    for habit in habits.iter().take(8) {
        out.push_str(&format!("- {} ({} lần)\n", habit.instruction, habit.times));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn an_ask_survives_being_written_and_read() {
        let tmp = dir();
        record(
            tmp.path(),
            "2026-08-14",
            "viết báo cáo sau họp",
            Some("01ABC"),
        )
        .unwrap();
        record(tmp.path(), "2026-08-15", "gửi email cho khách", None).unwrap();

        let asks = load(tmp.path());
        assert_eq!(asks.len(), 2);
        assert_eq!(asks[0].instruction, "viết báo cáo sau họp");
        assert_eq!(asks[0].meeting.as_deref(), Some("01ABC"));
        assert_eq!(asks[1].meeting, None);
    }

    /// The point of the whole module: the fourth time, it is a habit.
    #[test]
    fn the_same_request_twice_is_a_habit() {
        let asks = vec![
            Ask {
                day: "2026-08-01".into(),
                instruction: "Viết báo cáo sau họp".into(),
                meeting: None,
            },
            Ask {
                day: "2026-08-08".into(),
                instruction: "viết báo cáo sau họp!".into(),
                meeting: None,
            },
            Ask {
                day: "2026-08-09".into(),
                instruction: "tóm tắt cho sếp".into(),
                meeting: None,
            },
        ];
        let found = habits(&asks);
        assert_eq!(found.len(), 1, "asked once is not a habit");
        assert_eq!(found[0].times, 2);
        // The latest phrasing, because people refine how they ask.
        assert_eq!(found[0].instruction, "viết báo cáo sau họp!");
        assert_eq!(found[0].last, "2026-08-08");
    }

    /// A user who deletes a line has deleted the habit. That is the property that makes this a
    /// document rather than a store, and it has to actually hold.
    #[test]
    fn deleting_a_line_forgets_it() {
        let tmp = dir();
        record(tmp.path(), "2026-08-14", "viết báo cáo", None).unwrap();
        record(tmp.path(), "2026-08-15", "viết báo cáo", None).unwrap();
        assert_eq!(habits(&load(tmp.path())).len(), 1);

        let text = std::fs::read_to_string(path(tmp.path())).unwrap();
        let pruned: String = text
            .lines()
            .filter(|l| !l.contains("2026-08-15"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path(tmp.path()), pruned).unwrap();

        assert!(habits(&load(tmp.path())).is_empty());
    }

    #[test]
    fn the_log_does_not_grow_forever() {
        let tmp = dir();
        for i in 0..MAX_ASKS + 10 {
            record(tmp.path(), "2026-08-14", &format!("việc số {i}"), None).unwrap();
        }
        let asks = load(tmp.path());
        assert_eq!(asks.len(), MAX_ASKS);
        assert_eq!(
            asks.last().unwrap().instruction,
            format!("việc số {}", MAX_ASKS + 9)
        );
    }

    #[test]
    fn nothing_learned_says_nothing_in_the_prompt() {
        assert!(render(&[]).is_empty());
        assert!(render(&habits(&[])).is_empty());
    }

    /// An instruction with an em dash in it must not be read back as an instruction plus a meeting
    /// id — that is the separator this file uses, and users type dashes.
    #[test]
    fn a_dash_in_the_instruction_does_not_become_a_meeting() {
        let tmp = dir();
        record(
            tmp.path(),
            "2026-08-14",
            "viết email — ngắn thôi",
            Some("01ABC"),
        )
        .unwrap();
        let asks = load(tmp.path());
        assert_eq!(asks[0].meeting.as_deref(), Some("01ABC"));
        assert_eq!(asks[0].instruction, "viết email — ngắn thôi");
    }
}
