//! Agents, as directories of Markdown.
//!
//! There is no agent registry, no schema, no table. An agent is a folder in the vault:
//!
//! ```text
//! ~/.summo/vault/agents/
//!   AGENTS.md                  rules every agent inherits
//!   coordinator/
//!     AGENT.md                 who it is: frontmatter, then its brief in prose
//!     MEMORY.md                what it has learned, oldest first
//!     TASKS.md                 its own checklist, which it maintains
//!   researcher/
//!     AGENT.md  MEMORY.md  TASKS.md
//! ```
//!
//! ## Why files, again
//!
//! ADR 0002 argued the vault should be Markdown because a meeting the user cannot open is a meeting
//! they do not own. The same argument applies with more force to an agent, because an agent is a
//! thing that *acts on your behalf*: you have to be able to read what it was told, see what it
//! remembers, and change either — without a settings screen mediating.
//!
//! Four properties fall out of that choice rather than being built:
//!
//! * **Editable.** Customising an agent is editing a file. So is creating one: `mkdir` plus an
//!   `AGENT.md`. There is no "add agent" endpoint because there is nothing to add it *to*.
//! * **Self-modifying, safely.** An agent given the file tools can rewrite another agent's brief or
//!   append to its own memory. That is a text edit, visible in a diff, revertible with `git
//!   checkout` — not an opaque state mutation.
//! * **Syncable for free.** Whatever carries the vault carries the agents. One mechanism, not two.
//! * **Inspectable when it goes wrong.** "Why did it do that" is answered by reading three files.
//!
//! ## What the roster is not
//!
//! It is not a scheduler and not a supervisor. It reads definitions and answers questions about
//! them. Running an agent is [`crate::run`], and who may run whom is [`AgentDef::spawns`] — a
//! declaration in a file the user can read, so "which agents can start other agents" is never a
//! surprise.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

/// The file naming an agent inside its directory.
const DEFINITION: &str = "AGENT.md";
/// The shared rules, at the root of the agents directory.
const BASE: &str = "AGENTS.md";
const MEMORY: &str = "MEMORY.md";
const TASKS: &str = "TASKS.md";

/// The YAML head of an `AGENT.md`.
///
/// Everything is optional except the name. An agent that says only what it is called and then
/// describes itself in prose is a complete agent; the rest are overrides for when the defaults are
/// wrong.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Head {
    pub name: String,
    /// Free text, shown in the interface next to the name.
    pub description: String,
    /// Endpoint id, when this agent should not use the one in settings. A cheap local model for a
    /// summariser and a strong hosted one for a planner is the obvious reason.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Tools this agent may call. Empty means the ones the base grants.
    ///
    /// An allow-list rather than a deny-list: a tool added to Summo next year must not silently
    /// become available to an agent nobody re-reviewed.
    pub tools: Vec<String>,
    /// Agents this one may start. Empty means it may start none.
    ///
    /// The coordinator is simply the agent with entries here. There is no `coordinator: true` flag,
    /// because the interesting question is never "is this the boss" but "what can it reach".
    pub spawns: Vec<String>,
    /// Turns before the run is abandoned. `None` follows the base.
    pub max_turns: Option<usize>,
}

/// One agent, as read from its directory.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentDef {
    /// Directory name. The id everything else refers to it by.
    pub slug: String,
    pub head: Head,
    /// The prose under the frontmatter: what this agent is for, in the user's own words.
    pub brief: String,
    pub dir: PathBuf,
}

impl AgentDef {
    /// What the model is told, assembled from the shared rules, this agent's brief and its memory.
    ///
    /// In that order deliberately. The base is what every agent must obey and goes first so a brief
    /// cannot quietly override it by being longer. Memory goes last because it is the most specific
    /// and the most recent.
    #[must_use]
    pub fn system_prompt(&self, base: &str, memory: &str) -> String {
        let mut out = String::new();
        if !base.trim().is_empty() {
            out.push_str(base.trim());
            out.push_str("\n\n");
        }
        out.push_str(&format!("# {}\n\n", self.head.name));
        if !self.brief.trim().is_empty() {
            out.push_str(self.brief.trim());
            out.push_str("\n\n");
        }
        if !memory.trim().is_empty() {
            out.push_str("## What you have learned so far\n\n");
            out.push_str(memory.trim());
            out.push('\n');
        }
        out.trim_end().to_string()
    }

    #[must_use]
    pub fn memory_path(&self) -> PathBuf {
        self.dir.join(MEMORY)
    }

    #[must_use]
    pub fn tasks_path(&self) -> PathBuf {
        self.dir.join(TASKS)
    }

    /// Whether this agent is allowed to start another.
    #[must_use]
    pub fn may_spawn(&self, slug: &str) -> bool {
        self.head.spawns.iter().any(|s| s == slug)
    }

    /// Whether this agent may call a tool, given what the base grants.
    ///
    /// An empty list on the agent means "the base's", not "everything" — the permissive reading
    /// would make a typo in the frontmatter widen access rather than narrow it.
    #[must_use]
    pub fn may_call(&self, tool: &str, base_tools: &[String]) -> bool {
        let allowed = if self.head.tools.is_empty() {
            base_tools
        } else {
            &self.head.tools
        };
        allowed.iter().any(|t| t == tool)
    }
}

/// Every agent, plus the rules they share.
#[derive(Debug, Clone, Default)]
pub struct Roster {
    base: String,
    base_tools: Vec<String>,
    agents: BTreeMap<String, AgentDef>,
    /// Directories that looked like agents and would not parse, so a broken one is visible rather
    /// than absent. The same reasoning as [`summo_vault::MeetingIndex::skipped`].
    skipped: Vec<(PathBuf, String)>,
}

impl Roster {
    /// Read the agents directory, seeding it on first run.
    pub fn load_or_seed(dir: &Path) -> Result<Self> {
        if !dir.join(BASE).is_file() {
            seed(dir)?;
        }
        Self::load(dir)
    }

    /// Read the agents directory as it stands. A missing directory is an empty roster, not an
    /// error: a user who deleted it has said something, and crashing is a poor way to hear it.
    pub fn load(dir: &Path) -> Result<Self> {
        let base = std::fs::read_to_string(dir.join(BASE)).unwrap_or_default();
        let base_tools = tools_in(&base);

        let mut agents = BTreeMap::new();
        let mut skipped = Vec::new();

        let Ok(entries) = std::fs::read_dir(dir) else {
            return Ok(Self {
                base,
                base_tools,
                ..Default::default()
            });
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || !path.join(DEFINITION).is_file() {
                continue;
            }
            let slug = match path.file_name().and_then(|n| n.to_str()) {
                Some(slug) => slug.to_string(),
                None => continue,
            };
            match read_definition(&path, &slug) {
                Ok(def) => {
                    agents.insert(slug, def);
                }
                Err(e) => skipped.push((path, e.to_string())),
            }
        }

        Ok(Self {
            base,
            base_tools,
            agents,
            skipped,
        })
    }

    #[must_use]
    pub fn base(&self) -> &str {
        &self.base
    }

    #[must_use]
    pub fn base_tools(&self) -> &[String] {
        &self.base_tools
    }

    #[must_use]
    pub fn get(&self, slug: &str) -> Option<&AgentDef> {
        self.agents.get(slug)
    }

    /// Every agent, in directory order — which is alphabetical, so a listing is stable.
    pub fn all(&self) -> impl Iterator<Item = &AgentDef> {
        self.agents.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    #[must_use]
    pub fn skipped(&self) -> &[(PathBuf, String)] {
        &self.skipped
    }

    /// The agents that can start others, which is what "coordinator" means here.
    pub fn coordinators(&self) -> impl Iterator<Item = &AgentDef> {
        self.all().filter(|a| !a.head.spawns.is_empty())
    }

    /// Check that every `spawns` entry names an agent that exists.
    ///
    /// A typo here is otherwise invisible until a run tries to delegate and finds nothing — at
    /// which point the failure is attributed to the model rather than to the file.
    #[must_use]
    pub fn dangling_spawns(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for agent in self.all() {
            for target in &agent.head.spawns {
                if !self.agents.contains_key(target) {
                    out.push((agent.slug.clone(), target.clone()));
                }
            }
        }
        out
    }
}

/// Parse an `AGENT.md`.
fn read_definition(dir: &Path, slug: &str) -> Result<AgentDef> {
    let path = dir.join(DEFINITION);
    let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;

    let (head, brief) = match split_frontmatter(&text) {
        Some((yaml, body)) => {
            let head: Head = serde_yaml::from_str(yaml)
                .map_err(|e| Error::Vault(format!("{}: {e}", path.display())))?;
            (head, body.to_string())
        }
        // No frontmatter is not an error, for the same reason a hand-written note is not: somebody
        // made a folder and described what they wanted in it. The directory name is the name.
        None => (
            Head {
                name: slug.to_string(),
                ..Default::default()
            },
            text.clone(),
        ),
    };

    let mut head = head;
    if head.name.trim().is_empty() {
        head.name = slug.to_string();
    }

    Ok(AgentDef {
        slug: slug.to_string(),
        head,
        brief: strip_leading_heading(&brief),
        dir: dir.to_path_buf(),
    })
}

/// Drop a leading `# Title`, which duplicates the name and reads as noise once the prompt adds one.
fn strip_leading_heading(body: &str) -> String {
    let trimmed = body.trim_start();
    match trimmed.strip_prefix("# ") {
        Some(rest) => rest
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .trim()
            .to_string(),
        None => trimmed.trim().to_string(),
    }
}

fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let after = rest[end + 4..].trim_start_matches('\n');
    Some((&rest[..end], after))
}

/// Tool names listed in the base file, as `- `name`` bullets under a Tools heading.
///
/// Read from the Markdown rather than a parallel config, so the document a user edits to grant a
/// capability is the same one that grants it.
fn tools_in(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in base.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            inside = heading.trim().eq_ignore_ascii_case("tools");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.trim().strip_prefix("- ") {
            let name = rest.trim().trim_matches('`');
            let name = name
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('`');
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Write a starting roster: the shared rules and one coordinator that can reach two workers.
///
/// Seeded rather than assumed, so the first thing a curious user finds is a working example they
/// can copy — the same reason `templates/` ships four files instead of documenting a format.
fn seed(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    write(&dir.join(BASE), BASE_RULES)?;

    for (slug, definition) in [
        ("coordinator", COORDINATOR),
        ("librarian", LIBRARIAN),
        ("scribe", SCRIBE),
    ] {
        let agent = dir.join(slug);
        std::fs::create_dir_all(&agent).map_err(|e| Error::io(&agent, e))?;
        write(&agent.join(DEFINITION), definition)?;
        write(&agent.join(MEMORY), "# Memory\n\nNothing learned yet.\n")?;
        write(&agent.join(TASKS), "# Tasks\n\n")?;
    }
    Ok(())
}

/// Write only if absent. Seeding must never overwrite something a user changed.
fn write(path: &Path, body: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, body).map_err(|e| Error::io(path, e))
}

const BASE_RULES: &str = r#"# Rules every agent follows

You are part of Summo, a meeting assistant that runs on the user's own machine. These rules apply
to you whatever else your own brief says.

- Work only from what is in the vault. If you do not know something, say so rather than inventing
  it. A confident wrong answer about what was said in a meeting is worse than no answer.
- Never claim a task is done that you have not verified.
- Quote timestamps as `[hh:mm:ss]` so the user can jump to the moment you mean.
- Write in the language the user writes in.
- You may read anything in the vault. You may write only through the tools listed below.

## Tools

- `search_transcripts`
- `get_meeting`
- `list_tasks`
- `create_task`
- `update_task`
- `remember`

## Memory

`remember` appends a line to your own `MEMORY.md`. Use it for things that will still be true next
week — who people are, how this team names things, what the user has told you to stop doing. Do not
use it for the contents of a meeting; that is already in the vault.
"#;

const COORDINATOR: &str = r#"---
name: Coordinator
description: Decides which agent should handle a request, and hands it over.
spawns:
  - librarian
  - scribe
tools:
  - search_transcripts
  - list_tasks
  - create_task
  - remember
max_turns: 8
---

Your job is routing, not doing.

Read the request, decide which agent it belongs to, and hand it over with a clear instruction. If
it belongs to none of them, do it yourself only if it is a single lookup; otherwise say plainly
that no agent covers it, and suggest what a new one would need to do.

- **librarian** — finding things that were said, across meetings.
- **scribe** — writing: summaries, notes, action items.

Prefer handing over one clear task to guessing at three.
"#;

const LIBRARIAN: &str = r#"---
name: Librarian
description: Finds what was actually said, and where.
tools:
  - search_transcripts
  - get_meeting
  - remember
max_turns: 6
---

You answer questions about what was said, and you cite where.

Every claim you make must carry the meeting and the `[hh:mm:ss]` it came from. If the vault does
not contain the answer, say that — do not reason your way to a plausible one.
"#;

const SCRIBE: &str = r#"---
name: Scribe
description: Writes summaries and turns decisions into tasks.
tools:
  - get_meeting
  - create_task
  - update_task
  - remember
max_turns: 8
---

You write. Summaries, notes, action items.

Keep the user's own words where you can. A summary that reads like the person who was there wrote
it is worth more than one that reads like a report. Every action item needs an owner; if the
meeting did not name one, leave it unassigned rather than guessing.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> (tempfile::TempDir, Roster) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents");
        let roster = Roster::load_or_seed(&dir).unwrap();
        (tmp, roster)
    }

    #[test]
    fn a_fresh_install_gets_a_working_roster() {
        let (_tmp, roster) = roster();
        assert_eq!(roster.len(), 3);
        assert!(roster.get("coordinator").is_some());
        assert!(roster.get("librarian").is_some());
        assert!(roster.get("scribe").is_some());
    }

    #[test]
    fn every_agent_is_a_directory_of_markdown() {
        let (_tmp, roster) = roster();
        let agent = roster.get("librarian").unwrap();
        assert!(agent.dir.join("AGENT.md").is_file());
        assert!(agent.memory_path().is_file());
        assert!(agent.tasks_path().is_file());
    }

    /// Seeding must never overwrite something the user changed, or the first edit to an agent
    /// survives exactly until the next launch.
    #[test]
    fn seeding_twice_does_not_overwrite_an_edit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("agents");
        Roster::load_or_seed(&dir).unwrap();

        let brief = dir.join("scribe").join(DEFINITION);
        std::fs::write(&brief, "---\nname: Mine\n---\n\nDo it my way.\n").unwrap();
        // Removing the base is what makes `load_or_seed` try to seed again.
        std::fs::remove_file(dir.join(BASE)).unwrap();

        let roster = Roster::load_or_seed(&dir).unwrap();
        assert_eq!(roster.get("scribe").unwrap().head.name, "Mine");
    }

    /// Creating an agent is `mkdir` plus a file. If that needs frontmatter it is not really files.
    #[test]
    fn a_directory_with_a_bare_markdown_file_is_an_agent() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        let mine = dir.join("translator");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join(DEFINITION), "Translate what you are given.\n").unwrap();

        let roster = Roster::load(&dir).unwrap();
        let agent = roster.get("translator").unwrap();
        assert_eq!(agent.head.name, "translator", "the folder names it");
        assert_eq!(agent.brief, "Translate what you are given.");
    }

    #[test]
    fn a_leading_heading_is_not_repeated_in_the_brief() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        let mine = dir.join("x");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(
            mine.join(DEFINITION),
            "---\nname: X\n---\n\n# X\n\nThe actual brief.\n",
        )
        .unwrap();

        let roster = Roster::load(&dir).unwrap();
        assert_eq!(roster.get("x").unwrap().brief, "The actual brief.");
    }

    #[test]
    fn a_broken_definition_is_reported_and_the_others_still_load() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(DEFINITION), "---\nname: [unterminated\n---\n").unwrap();

        let roster = Roster::load(&dir).unwrap();
        assert_eq!(roster.len(), 3, "the good ones still load");
        assert_eq!(roster.skipped().len(), 1);
    }

    #[test]
    fn a_directory_without_a_definition_is_not_an_agent() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        std::fs::create_dir_all(dir.join("notes-about-agents")).unwrap();
        assert_eq!(Roster::load(&dir).unwrap().len(), 3);
    }

    // ---- the prompt ------------------------------------------------------------------------

    /// The base goes first so a long brief cannot bury a rule by out-weighing it.
    #[test]
    fn the_shared_rules_come_before_the_agent_s_own_brief() {
        let (_tmp, roster) = roster();
        let agent = roster.get("librarian").unwrap();
        let prompt = agent.system_prompt("NEVER invent an answer.", "");

        let rule = prompt.find("NEVER invent").unwrap();
        let brief = prompt.find("You answer questions").unwrap();
        assert!(rule < brief, "{prompt}");
    }

    #[test]
    fn memory_comes_last_because_it_is_the_most_specific() {
        let (_tmp, roster) = roster();
        let agent = roster.get("librarian").unwrap();
        let prompt = agent.system_prompt("Base.", "- Ngọc leads product.");

        let brief = prompt.find("You answer questions").unwrap();
        let memory = prompt.find("Ngọc leads product").unwrap();
        assert!(brief < memory, "{prompt}");
    }

    #[test]
    fn an_agent_with_no_memory_yet_has_no_empty_heading() {
        let (_tmp, roster) = roster();
        let prompt = roster.get("scribe").unwrap().system_prompt("Base.", "   ");
        assert!(!prompt.contains("learned so far"), "{prompt}");
    }

    // ---- who may do what -------------------------------------------------------------------

    #[test]
    fn only_the_coordinator_can_start_other_agents() {
        let (_tmp, roster) = roster();
        assert!(roster.get("coordinator").unwrap().may_spawn("librarian"));
        assert!(!roster.get("librarian").unwrap().may_spawn("scribe"));

        let bosses: Vec<&str> = roster.coordinators().map(|a| a.slug.as_str()).collect();
        assert_eq!(bosses, vec!["coordinator"]);
    }

    #[test]
    fn an_agent_cannot_start_one_it_does_not_name() {
        let (_tmp, roster) = roster();
        assert!(!roster.get("coordinator").unwrap().may_spawn("translator"));
    }

    /// A `spawns` entry naming nothing is otherwise invisible until a run tries to delegate, at
    /// which point the failure looks like the model's fault.
    #[test]
    fn a_spawn_pointing_at_nobody_is_reported() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        std::fs::write(
            dir.join("coordinator").join(DEFINITION),
            "---\nname: C\nspawns: [librarian, ghost]\n---\n\nRoute.\n",
        )
        .unwrap();

        let dangling = Roster::load(&dir).unwrap().dangling_spawns();
        assert_eq!(dangling, vec![("coordinator".into(), "ghost".into())]);
    }

    #[test]
    fn the_base_file_is_where_tools_are_granted() {
        let (_tmp, roster) = roster();
        assert!(
            roster
                .base_tools()
                .contains(&"search_transcripts".to_string())
        );
        assert!(roster.base_tools().contains(&"remember".to_string()));
    }

    #[test]
    fn an_agent_narrows_the_base_grant_rather_than_widening_it() {
        let (_tmp, roster) = roster();
        let base = roster.base_tools().to_vec();
        let librarian = roster.get("librarian").unwrap();

        assert!(librarian.may_call("search_transcripts", &base));
        assert!(
            !librarian.may_call("create_task", &base),
            "the base grants it; this agent does not list it"
        );
    }

    /// The permissive reading of an empty list would make a typo widen access instead of narrowing
    /// it, which is the wrong direction for a mistake to fail in.
    #[test]
    fn an_agent_listing_no_tools_gets_the_base_ones_not_all_of_them() {
        let (tmp, _) = roster();
        let dir = tmp.path().join("agents");
        let mine = dir.join("quiet");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join(DEFINITION), "---\nname: Quiet\n---\n\nWait.\n").unwrap();

        let roster = Roster::load(&dir).unwrap();
        let base = roster.base_tools().to_vec();
        let quiet = roster.get("quiet").unwrap();
        assert!(quiet.may_call("search_transcripts", &base));
        assert!(!quiet.may_call("rm_rf", &base));
    }
}
