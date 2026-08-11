//! Running an `@agent` task.
//!
//! The loop is [`aion_agent::engine::AgentEngine`]. What this adds is the bookkeeping that makes an
//! autonomous run *legible*: the agent's tool calls become steps in the task's own checklist, in
//! the file the user reads, as they happen.
//!
//! ```text
//!   - [ ] @agent Tạo lịch          ──► engine.run()
//!         │                              │  search_transcripts  ──► - [x] Tìm trong transcript
//!         │                              │  create_task         ──► - [x] Thêm việc
//!         └── status:doing ──────────────┴─ finished ───────────► - [x] @agent Tạo lịch
//! ```
//!
//! Each step is written **before** the tool runs and ticked **after**, so a crash leaves the list
//! showing where it stopped rather than losing the trace. That is the difference between an agent
//! you can supervise and a spinner.
//!
//! The engine is given Summo's tools and nothing else — no shell, no file writes. A user who cannot
//! predict what an agent will do cannot consent to it running.

use std::sync::{Arc, Mutex};

use aion_agent::engine::AgentEngine;
use aion_agent::output::OutputSink;
use aion_config::config::{CliArgs, Config};
use aion_tools::registry::ToolRegistry;
use summo_core::{Error, Result, paths::Paths};
use summo_vault::tasks::{Status, Step, Task};

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub task: String,
    pub steps: Vec<Step>,
    /// The agent's closing message, which is what the interface shows as the outcome.
    pub outcome: String,
    pub status: Status,
}

/// Records the engine's tool calls as steps, and writes them into the vault as they happen.
///
/// The engine calls this from whichever thread it is on, so the state is behind a mutex. Writes go
/// straight to disk rather than being batched at the end — batching is what turns a crashed run
/// into a task that looks like it never started.
struct StepRecorder {
    paths: Paths,
    task: Mutex<Task>,
    /// Set when the engine reports a failing tool, so the task can end `failed` rather than `done`.
    failed: Mutex<bool>,
    /// The agent's prose, kept for the outcome line.
    text: Mutex<String>,
}

impl StepRecorder {
    fn new(paths: Paths, task: Task) -> Self {
        Self {
            paths,
            task: Mutex::new(task),
            failed: Mutex::new(false),
            text: Mutex::new(String::new()),
        }
    }

    fn write(&self) {
        let task = self.task.lock().expect("step recorder poisoned");
        if let Err(e) = crate::steps::write(&self.paths, &task, &task.steps) {
            // A run that cannot record its steps is still a run worth finishing.
            tracing::warn!(task = %task.id, error = %e, "cannot record an agent step");
        }
    }

    fn snapshot(&self) -> Task {
        self.task.lock().expect("step recorder poisoned").clone()
    }
}

impl OutputSink for StepRecorder {
    fn emit_text_delta(&self, text: &str, _msg_id: &str) {
        self.text.lock().expect("poisoned").push_str(text);
    }

    fn emit_thinking(&self, _text: &str, _msg_id: &str) {
        // Reasoning is not a step. Recording it would fill the user's notes with the model's
        // internal monologue, which is not what "what did it do" means.
    }

    fn emit_tool_call(&self, _tool_use_id: &str, name: &str, input: &str) {
        {
            let mut task = self.task.lock().expect("poisoned");
            task.steps.push(Step {
                text: describe(name, input),
                done: false,
            });
        }
        // Before the tool runs, so an interrupted run shows the step it died on.
        self.write();
    }

    fn emit_tool_result(&self, _tool_use_id: &str, _name: &str, is_error: bool, _content: &str) {
        {
            let mut task = self.task.lock().expect("poisoned");
            if let Some(last) = task.steps.last_mut() {
                last.done = true;
                if is_error {
                    last.text = format!("{} — lỗi", last.text);
                }
            }
            if is_error {
                *self.failed.lock().expect("poisoned") = true;
            }
        }
        self.write();
    }

    fn emit_stream_start(&self, _msg_id: &str) {}

    fn emit_stream_end(&self, _msg_id: &str, _turns: usize, _i: u64, _o: u64, _c: u64, _r: u64) {}

    fn emit_error(&self, msg: &str) {
        *self.failed.lock().expect("poisoned") = true;
        tracing::warn!(error = %msg, "agent run reported an error");
    }

    fn emit_info(&self, _msg: &str) {}
}

/// A tool call, in words a person reading their notes would recognise.
///
/// The raw JSON is not it: `{"query":"ngân sách"}` in the middle of a meeting note is noise. The
/// argument that says *what the call was about* is worth keeping; the rest is not.
fn describe(name: &str, input: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(input).unwrap_or(serde_json::Value::Null);
    let arg = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("");

    match name {
        "search_transcripts" => format!("Tìm trong transcript: “{}”", arg("query")),
        "get_meeting" => format!("Đọc buổi họp {}", arg("id")),
        "list_tasks" => "Xem danh sách việc".to_string(),
        "create_task" => format!("Thêm việc: {}", arg("text")),
        "update_task" => format!("Cập nhật việc {}", arg("id")),
        other => format!("Gọi {other}"),
    }
}

/// The endpoint the agent should use, translated into what `aion-agent` accepts.
struct Chosen {
    /// `anthropic` or `openai`; those are the two request shapes aion implements.
    provider: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
}

/// Route the agent at whatever the user configured for everything else.
///
/// Without this the agent took aion's own defaults, which are Anthropic and a key from
/// `ANTHROPIC_API_KEY`. So a machine set to Ollama — the default, and the one the settings screen
/// was displaying — ran `@agent` tasks against a hosted provider it had never been pointed at, and
/// failed with a missing-key error naming a variable that appears nowhere in Summo's interface.
///
/// Every preset except Anthropic speaks the OpenAI shape, so the mapping is the wire format plus a
/// base URL. If settings cannot be read or name something unresolvable, aion's defaults stand:
/// refusing to run a task because a settings file is unreadable helps nobody.
fn chosen_provider(paths: &Paths) -> Chosen {
    let settings = summo_core::Settings::load(&paths.settings()).unwrap_or_default();
    let id = settings.llm.provider.trim();

    let catalogue = summo_llm::provider::catalogue(&paths.providers());
    let provider = summo_llm::provider::Provider::resolve_in(
        &catalogue,
        id,
        settings.llm.model.as_deref(),
        None,
    );

    let Ok(provider) = provider else {
        tracing::warn!(provider = id, "cannot resolve the configured provider for the agent");
        return Chosen {
            provider: "anthropic".into(),
            model: settings.llm.model.unwrap_or_else(|| "claude-opus-5".into()),
            api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            base_url: None,
        };
    };

    match provider.wire {
        summo_llm::Wire::Anthropic => Chosen {
            provider: "anthropic".into(),
            model: provider.model,
            api_key: provider.api_key.unwrap_or_default(),
            base_url: Some(provider.base_url),
        },
        summo_llm::Wire::OpenAi => Chosen {
            provider: "openai".into(),
            model: provider.model,
            // A local server ignores the credential but the client still sends a header, and an
            // empty one is rejected by some of them. aion's own documentation uses this shape.
            api_key: provider.api_key.unwrap_or_else(|| "local".into()),
            base_url: Some(provider.base_url),
        },
    }
}

/// Run one `@agent` task to completion.
///
/// `instruction` is the task's own text — the agent is told to do the thing the checkbox says, and
/// nothing broader.
pub async fn run(paths: &Paths, task: &Task) -> Result<Ran> {
    if !task.is_agent() {
        return Err(Error::Other(format!(
            "task {} is not the agent's to run",
            task.id
        )));
    }

    // Mark it running and clear any plan from a previous attempt, so a retry does not append to
    // the steps of the run that failed.
    let mut task = task.clone();
    task.steps.clear();
    summo_vault::tasks_io::update(paths, &task.id, Some(Status::Doing), None, None)?;
    crate::steps::write(paths, &task, &[])?;

    let recorder = Arc::new(StepRecorder::new(paths.clone(), task.clone()));

    let chosen = chosen_provider(paths);

    let config = Config::resolve(&CliArgs {
        provider: Some(chosen.provider),
        api_key: Some(chosen.api_key),
        base_url: chosen.base_url,
        model: Some(chosen.model),
        max_tokens: None,
        thinking: None,
        thinking_budget: None,
        // A task that has not finished in a dozen turns is stuck, and a loop that keeps paying for
        // tokens while stuck is worse than one that stops and says so.
        max_turns: Some(12),
        max_tool_call_malformed_turns: Some(3),
        max_tool_call_failure_turns: Some(3),
        system_prompt: Some(SYSTEM_PROMPT.to_string()),
        profile: None,
        // The tool set has no shell and no file writes; approving each call would be ceremony.
        auto_approve: true,
        project_dir: Some(paths.root().to_path_buf()),
    })
    .map_err(|e| Error::Other(format!("cannot configure the agent: {e}")))?;

    let mut registry = ToolRegistry::new();
    for tool in crate::tools::all(Arc::new(paths.clone())) {
        registry.register(tool);
    }

    let mut engine = AgentEngine::new(
        config,
        registry,
        recorder.clone() as Arc<dyn OutputSink>,
        paths.root().to_path_buf(),
    );

    let outcome = engine.run(&task.text, &task.id).await;

    let finished = recorder.snapshot();
    let failed = *recorder.failed.lock().expect("poisoned") || outcome.is_err();
    let status = if failed { Status::Failed } else { Status::Done };

    summo_vault::tasks_io::update(paths, &task.id, Some(status), None, None)?;
    crate::steps::write(paths, &finished, &finished.steps)?;

    let text = recorder.text.lock().expect("poisoned").trim().to_string();
    Ok(Ran {
        task: task.id.clone(),
        steps: finished.steps,
        outcome: match &outcome {
            Ok(_) if !text.is_empty() => text,
            Ok(_) => "Xong.".to_string(),
            Err(e) => format!("Không hoàn thành: {e}"),
        },
        status,
    })
}

/// What the agent is told it is.
///
/// Short on purpose. The tools describe themselves, and a long preamble is context spent before the
/// work starts. The parts that matter are the boundaries.
const SYSTEM_PROMPT: &str = "Bạn là trợ lý trong Summo, một ứng dụng ghi chú cuộc họp chạy trên máy \
người dùng. Bạn được giao một việc lấy từ ghi chú của họ.\n\n\
Chỉ làm đúng việc được giao. Không tự mở rộng phạm vi.\n\
Chỉ dựa vào những gì có trong kho ghi chú, qua các công cụ được cấp. Không bịa tên người, con số \
hay cam kết không có trong transcript.\n\
Nếu thiếu thông tin để làm, hãy dừng và nói rõ thiếu gì — đoán mò tệ hơn là không làm.\n\
Trả lời bằng tiếng Việt, ngắn gọn: một hai câu về việc đã làm.";

#[cfg(test)]
mod tests {
    use super::*;

    fn task(owner: &str) -> Task {
        Task {
            id: "T1".into(),
            text: "Tạo lịch cho mốc ra mắt".into(),
            owner: Some(owner.into()),
            status: Status::Todo,
            due: None,
            steps: Vec::new(),
            file: "meetings/01A.md".into(),
            line: 5,
        }
    }

    #[tokio::test]
    async fn a_task_that_is_not_the_agents_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = run(&Paths::at(dir.path()), &task("ngoc")).await.unwrap_err();
        assert!(err.to_string().contains("not the agent's"), "{err}");
    }

    #[test]
    fn a_tool_call_is_described_in_words_not_json() {
        assert_eq!(
            describe("search_transcripts", r#"{"query":"ngân sách"}"#),
            "Tìm trong transcript: “ngân sách”"
        );
        assert_eq!(describe("create_task", r#"{"text":"Gửi báo giá"}"#), "Thêm việc: Gửi báo giá");
        assert_eq!(describe("list_tasks", "{}"), "Xem danh sách việc");
    }

    #[test]
    fn an_unknown_tool_still_gets_a_readable_line() {
        assert_eq!(describe("some_mcp_tool", "{}"), "Gọi some_mcp_tool");
    }

    /// A model can emit malformed arguments; the step list must still read as a list.
    #[test]
    fn malformed_arguments_do_not_break_the_description() {
        assert_eq!(
            describe("search_transcripts", "not json at all"),
            "Tìm trong transcript: “”"
        );
    }

    #[test]
    fn steps_are_recorded_before_the_tool_runs_and_ticked_after() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let recorder = StepRecorder::new(paths, task("agent"));

        recorder.emit_tool_call("1", "list_tasks", "{}");
        let mid = recorder.snapshot();
        assert_eq!(mid.steps.len(), 1);
        assert!(!mid.steps[0].done, "a step is open until its tool returns");

        recorder.emit_tool_result("1", "list_tasks", false, "ok");
        let after = recorder.snapshot();
        assert!(after.steps[0].done);
        assert!(!*recorder.failed.lock().unwrap());
    }

    #[test]
    fn a_failing_tool_marks_the_step_and_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = StepRecorder::new(Paths::at(dir.path()), task("agent"));

        recorder.emit_tool_call("1", "create_task", r#"{"text":"x"}"#);
        recorder.emit_tool_result("1", "create_task", true, "nope");

        let after = recorder.snapshot();
        assert!(after.steps[0].done, "a failed step is finished, not pending");
        assert!(after.steps[0].text.contains("lỗi"), "{}", after.steps[0].text);
        assert!(*recorder.failed.lock().unwrap());
    }

    /// Reasoning is not something to write into somebody's meeting notes.
    #[test]
    fn thinking_is_not_recorded_as_a_step() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = StepRecorder::new(Paths::at(dir.path()), task("agent"));
        recorder.emit_thinking("hmm, có lẽ nên...", "m1");
        assert!(recorder.snapshot().steps.is_empty());
    }

    #[test]
    fn the_agents_prose_is_collected_for_the_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = StepRecorder::new(Paths::at(dir.path()), task("agent"));
        recorder.emit_text_delta("Đã tạo ", "m1");
        recorder.emit_text_delta("3 sự kiện.", "m1");
        assert_eq!(recorder.text.lock().unwrap().as_str(), "Đã tạo 3 sự kiện.");
    }

    #[test]
    fn an_engine_error_marks_the_run_failed() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = StepRecorder::new(Paths::at(dir.path()), task("agent"));
        recorder.emit_error("provider unreachable");
        assert!(*recorder.failed.lock().unwrap());
    }
}
