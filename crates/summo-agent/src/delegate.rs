//! One agent asking another to do something.
//!
//! This is what makes a *roster* different from a list of prompts. The coordinator's job is to
//! decide who should handle a request; without a way to hand over, that decision has nowhere to go
//! and the coordinator ends up doing the work badly with the wrong tools — which is exactly what it
//! did before this existed.
//!
//! ## Three limits, and why each is here
//!
//! **Who.** An agent may only reach the agents its own `AGENT.md` names in `spawns`. Not a
//! capability it acquires at runtime, not something the model can talk its way into: a line in a
//! file the user can read. "Which agents can start other agents" should be answerable by looking,
//! not by tracing.
//!
//! **How deep.** [`MAX_DEPTH`] stops A → B → A from running until the budget is gone. Two levels is
//! enough for coordinator → worker, which is the shape that is actually useful; anything deeper is
//! usually a loop rather than a plan.
//!
//! **How wide.** A delegated run gets no task bookkeeping — it does not mark the parent's checkbox,
//! and it cannot start a run of its own beyond the depth limit. It answers and returns. The parent
//! stays responsible for the task, which keeps one owner per checkbox.

use std::sync::Arc;

use aion_agent::engine::AgentEngine;
use aion_config::config::{CliArgs, Config};
use aion_tools::Tool;
use aion_tools::registry::ToolRegistry;
use aion_protocol::events::ToolCategory;
use aion_types::tool::ToolResult;
use async_trait::async_trait;
use serde_json::{Value, json};
use summo_core::{Error, Result, paths::Paths};

use crate::roster::Roster;

/// How many levels of delegation are allowed.
///
/// Coordinator → worker is one. A worker that delegates again is usually a model going in circles,
/// and the cost of that circle is paid by the user in tokens.
pub const MAX_DEPTH: usize = 2;

/// Run a named agent on an instruction and return what it said.
///
/// No vault writes, no task status, no step recording: this is a question, not a job. The caller
/// owns whatever checkbox prompted it.
pub async fn ask(paths: &Paths, slug: &str, instruction: &str, depth: usize) -> Result<String> {
    if depth >= MAX_DEPTH {
        return Err(Error::Other(format!(
            "delegation is {MAX_DEPTH} deep already; {slug} was not started"
        )));
    }

    let roster = Roster::load(&paths.agents())?;
    let agent = roster
        .get(slug)
        .ok_or_else(|| Error::Other(format!("no agent called {slug}")))?
        .clone();

    let memory = crate::memory::render(&crate::memory::load(&agent.memory_path()));
    let system_prompt = agent.system_prompt(roster.base(), &memory);
    let chosen = crate::run::chosen_provider(paths, Some(&agent));

    let config = Config::resolve(&CliArgs {
        provider: Some(chosen.provider),
        api_key: Some(chosen.api_key),
        base_url: chosen.base_url,
        model: Some(chosen.model),
        max_tokens: None,
        thinking: None,
        thinking_budget: None,
        max_turns: Some(agent.head.max_turns.unwrap_or(8)),
        max_tool_call_malformed_turns: Some(3),
        max_tool_call_failure_turns: Some(3),
        system_prompt: Some(system_prompt),
        profile: None,
        auto_approve: true,
        project_dir: Some(paths.root().to_path_buf()),
    })
    .map_err(|e| Error::Other(format!("cannot configure {slug}: {e}")))?;

    let base_tools = roster.base_tools().to_vec();
    let mut registry = ToolRegistry::new();
    let today = summo_core::today();
    for tool in crate::tools::all_for(Arc::new(paths.clone()), Some(&agent), &today) {
        if !agent.may_call(tool.name(), &base_tools) {
            continue;
        }
        registry.register(tool);
    }
    // A delegate may delegate, up to the depth limit — so a coordinator can hand to a planner that
    // hands to a worker, but no further.
    if !agent.head.spawns.is_empty() {
        registry.register(Box::new(SpawnAgent::new(
            Arc::new(paths.clone()),
            agent.head.spawns.clone(),
            depth + 1,
        )));
    }

    let said = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = Arc::new(Collect(said.clone()));
    let mut engine = AgentEngine::new(
        config,
        registry,
        sink as Arc<dyn aion_agent::output::OutputSink>,
        paths.root().to_path_buf(),
    );

    tracing::info!(agent = slug, depth, "delegating");
    let outcome = engine.run(instruction, &format!("delegate-{slug}")).await;
    let text = said.lock().expect("poisoned").trim().to_string();

    match outcome {
        Ok(_) if !text.is_empty() => Ok(text),
        Ok(_) => Ok(format!("{slug} finished without saying anything")),
        Err(e) => Err(Error::Other(format!("{slug} failed: {e}"))),
    }
}

/// Collects the delegate's prose. It has no checkbox to write into, so its answer *is* its output.
struct Collect(Arc<std::sync::Mutex<String>>);

impl aion_agent::output::OutputSink for Collect {
    fn emit_text_delta(&self, text: &str, _msg_id: &str) {
        self.0.lock().expect("poisoned").push_str(text);
    }
    fn emit_thinking(&self, _text: &str, _msg_id: &str) {}
    fn emit_tool_call(&self, _id: &str, _name: &str, _input: &str) {}
    fn emit_tool_result(&self, _id: &str, _name: &str, _err: bool, _content: &str) {}
    fn emit_stream_start(&self, _msg_id: &str) {}
    fn emit_stream_end(&self, _m: &str, _t: usize, _i: u64, _o: u64, _c: u64, _r: u64) {}
    fn emit_error(&self, msg: &str) {
        tracing::warn!(error = %msg, "a delegated run reported an error");
    }
    fn emit_info(&self, _msg: &str) {}
}

/// The tool a coordinator uses to hand work over.
pub struct SpawnAgent {
    paths: Arc<Paths>,
    /// Exactly what this agent's own file permits, resolved before the model sees the tool.
    allowed: Vec<String>,
    depth: usize,
    description: String,
}

impl SpawnAgent {
    #[must_use]
    pub fn new(paths: Arc<Paths>, allowed: Vec<String>, depth: usize) -> Self {
        // The list goes in the description as well as the schema. A model that can see which agents
        // exist picks between them; one that has to guess a name spends turns being told no.
        let description = format!(
            "Giao việc cho một agent khác và nhận lại câu trả lời của nó. Có thể giao cho: {}.",
            allowed.join(", ")
        );
        Self {
            paths,
            allowed,
            depth,
            description,
        }
    }
}

#[async_trait]
impl Tool for SpawnAgent {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    // Enumerated, so the wrong name is rejected by the protocol rather than by a
                    // round trip that costs a turn.
                    "enum": self.allowed,
                    "description": "Agent nhận việc"
                },
                "instruction": {
                    "type": "string",
                    "description": "Việc cần làm, nói rõ ràng và đủ một mình đọc cũng hiểu"
                }
            },
            "required": ["agent", "instruction"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // A delegate may write to the vault, and two of them could be writing to the same file.
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let (Some(slug), Some(instruction)) = (
            input.get("agent").and_then(Value::as_str),
            input.get("instruction").and_then(Value::as_str),
        ) else {
            return ToolResult {
                content: "cần cả `agent` và `instruction`".into(),
                is_error: true,
            };
        };

        // Checked here as well as in the schema. The schema is a hint to a model; this is the rule.
        if !self.allowed.iter().any(|a| a == slug) {
            return ToolResult {
                content: format!(
                    "không được phép giao cho `{slug}`. Chỉ có thể giao cho: {}",
                    self.allowed.join(", ")
                ),
                is_error: true,
            };
        }

        match ask(&self.paths, slug, instruction, self.depth).await {
            Ok(answer) => ToolResult {
                content: answer,
                is_error: false,
            },
            Err(e) => ToolResult {
                content: e.to_string(),
                is_error: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster_at(dir: &std::path::Path) -> Roster {
        Roster::load_or_seed(dir).unwrap()
    }

    #[tokio::test]
    async fn an_agent_that_does_not_exist_is_named_in_the_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        roster_at(&paths.agents());

        let err = ask(&paths, "nobody", "do a thing", 0).await.unwrap_err();
        assert!(err.to_string().contains("nobody"), "{err}");
    }

    /// A → B → A would otherwise run until the token budget is gone, and the user would see a bill
    /// rather than an answer.
    #[tokio::test]
    async fn delegation_stops_at_the_depth_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        roster_at(&paths.agents());

        let err = ask(&paths, "librarian", "go", MAX_DEPTH).await.unwrap_err();
        assert!(err.to_string().contains("deep"), "{err}");
    }

    /// The schema is a hint to a model. This is the rule, and it has to hold when the model ignores
    /// the hint — which is the case that matters.
    #[tokio::test]
    async fn an_agent_cannot_reach_one_its_file_does_not_name() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        roster_at(&paths.agents());

        let tool = SpawnAgent::new(Arc::new(paths.clone()), vec!["librarian".into()], 0);
        let result = tool
            .execute(json!({ "agent": "scribe", "instruction": "write it" }))
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("không được phép"), "{}", result.content);
    }

    #[tokio::test]
    async fn a_call_missing_its_arguments_says_which() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let tool = SpawnAgent::new(Arc::new(paths), vec!["librarian".into()], 0);

        let result = tool.execute(json!({ "agent": "librarian" })).await;
        assert!(result.is_error);
        assert!(result.content.contains("instruction"), "{}", result.content);
    }

    /// A model that can see the names picks between them; one that has to guess spends turns being
    /// told no.
    #[test]
    fn the_reachable_agents_are_named_in_the_description_and_the_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Arc::new(Paths::at(tmp.path()));
        let tool = SpawnAgent::new(paths, vec!["librarian".into(), "scribe".into()], 0);

        assert!(tool.description().contains("librarian"));
        assert!(tool.description().contains("scribe"));

        let schema = tool.input_schema();
        let allowed = schema["properties"]["agent"]["enum"].as_array().unwrap();
        assert_eq!(allowed.len(), 2);
    }
}
