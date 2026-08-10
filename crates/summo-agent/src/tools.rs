//! What the agent is allowed to do.
//!
//! These implement [`aion_tools::Tool`], so the agent loop, the tool-calling protocol, retries and
//! the MCP bridge all come from aionrs. Nothing here is an agent; they are the verbs an agent has.
//!
//! The set is deliberately small and deliberately *about meetings*. An agent with a shell tool can
//! do anything, which sounds powerful and is actually the problem: the user cannot predict what it
//! will do, so they cannot decide whether to let it. Every tool here is one a user would recognise
//! as something Summo does anyway.
//!
//! Two rules hold across all of them:
//!
//! * **Nothing leaves the vault.** No tool takes a path; they take meeting ids and task ids, and
//!   resolve those against the vault themselves. A model that hallucinates `../../.ssh/id_rsa` gets
//!   "no meeting with that id".
//! * **Writes are narrow.** The agent can create and update tasks, because that is the job. It
//!   cannot delete a meeting, rewrite a transcript, or edit a summary a human wrote.

use std::sync::Arc;

use aion_protocol::events::ToolCategory;
use aion_tools::Tool;
use aion_types::tool::ToolResult;
use async_trait::async_trait;
use serde_json::{Value, json};
use summo_core::paths::Paths;
use summo_vault::tasks::{self, Status};

/// Cap on how much text one tool call returns.
///
/// A whole transcript can be hundreds of kilobytes; handing that to a model in one tool result
/// wastes the context the agent needs to actually reason.
const MAX_CHARS: usize = 8_000;

fn ok(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: text.into(),
        is_error: false,
    }
}

fn fail(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: text.into(),
        is_error: true,
    }
}

/// Truncate at a character boundary, saying so rather than silently cutting.
fn clamp(mut text: String) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(MAX_CHARS)
        .map_or(text.len(), |(i, _)| i);
    text.truncate(cut);
    text.push_str("\n\n[… đã cắt bớt, dùng công cụ tìm kiếm để thu hẹp lại]");
    text
}

/// Search every transcript in the vault.
pub struct SearchTranscripts {
    paths: Arc<Paths>,
}

impl SearchTranscripts {
    #[must_use]
    pub fn new(paths: Arc<Paths>) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl Tool for SearchTranscripts {
    fn name(&self) -> &str {
        "search_transcripts"
    }

    fn description(&self) -> &str {
        "Tìm trong toàn bộ transcript các buổi họp. Trả về tên buổi họp, ngày và đoạn khớp. \
         Dùng khi cần biết ai đã nói gì, hoặc một chủ đề được bàn ở buổi nào."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Từ khoá cần tìm" },
                "limit": { "type": "integer", "description": "Số kết quả tối đa", "default": 10 }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(query) = input.get("query").and_then(Value::as_str) else {
            return fail("thiếu tham số `query`");
        };
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(10)
            .clamp(1, 50) as usize;

        let library = summo_vault::library::Library::new((*self.paths).clone());
        match library.search(query, limit) {
            Ok(hits) if hits.is_empty() => ok(format!("Không tìm thấy gì cho `{query}`.")),
            Ok(hits) => {
                let mut out = String::new();
                for hit in hits {
                    out.push_str(&format!("## {} ({})\n", hit.meeting.title, hit.meeting.day));
                    for excerpt in hit.excerpts {
                        out.push_str(&format!("- {}\n", excerpt.text.trim()));
                    }
                    out.push('\n');
                }
                ok(clamp(out))
            }
            Err(e) => fail(format!("không tìm được: {e}")),
        }
    }
}

/// Read one meeting: its summary sections and transcript.
pub struct GetMeeting {
    paths: Arc<Paths>,
}

impl GetMeeting {
    #[must_use]
    pub fn new(paths: Arc<Paths>) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl Tool for GetMeeting {
    fn name(&self) -> &str {
        "get_meeting"
    }

    fn description(&self) -> &str {
        "Đọc một buổi họp theo id: tóm tắt, người tham gia, và transcript."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Id buổi họp" },
                "transcript": {
                    "type": "boolean",
                    "description": "Kèm cả transcript đầy đủ. Mặc định chỉ lấy tóm tắt.",
                    "default": false
                }
            },
            "required": ["id"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(id) = input.get("id").and_then(Value::as_str) else {
            return fail("thiếu tham số `id`");
        };
        let want_transcript = input
            .get("transcript")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let library = summo_vault::library::Library::new((*self.paths).clone());
        match library.detail(&summo_core::MeetingId::from(id.to_string())) {
            Ok(detail) => {
                let mut out = format!(
                    "# {}\nNgày: {}\nThời lượng: {} giây\nNgười tham gia: {}\n\n",
                    detail.summary.title,
                    detail.summary.day,
                    detail.summary.duration,
                    detail.summary.participants.join(", ")
                );
                for section in &detail.sections {
                    out.push_str(&format!("## {}\n{}\n\n", section.heading, section.body));
                }
                if want_transcript {
                    out.push_str("## Transcript\n");
                    for segment in &detail.transcript {
                        out.push_str(&format!(
                            "{} — {}\n",
                            segment.speaker.as_ref().map_or("?", summo_core::SpeakerId::as_str),
                            segment.text
                        ));
                    }
                }
                ok(clamp(out))
            }
            Err(e) => fail(format!("không đọc được buổi họp `{id}`: {e}")),
        }
    }
}

/// List tasks, optionally filtered.
pub struct ListTasks {
    paths: Arc<Paths>,
}

impl ListTasks {
    #[must_use]
    pub fn new(paths: Arc<Paths>) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl Tool for ListTasks {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn description(&self) -> &str {
        "Liệt kê việc cần làm trong kho. Lọc được theo người nhận hoặc trạng thái."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Chỉ lấy việc của người này" },
                "status": {
                    "type": "string",
                    "enum": ["todo", "doing", "done", "blocked", "failed"]
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let owner = input.get("owner").and_then(Value::as_str);
        let status = input.get("status").and_then(Value::as_str);

        match read_all_tasks(&self.paths) {
            Ok(all) => {
                let filtered: Vec<_> = all
                    .into_iter()
                    .filter(|t| owner.is_none_or(|o| t.owner.as_deref() == Some(o)))
                    .filter(|t| status.is_none_or(|s| t.status.as_str() == s))
                    .collect();

                if filtered.is_empty() {
                    return ok("Không có việc nào khớp.");
                }
                let mut out = String::new();
                for task in filtered {
                    out.push_str(&format!(
                        "- [{}] {} (id:{}, {}{})\n",
                        if task.status.is_finished() { "x" } else { " " },
                        task.text,
                        task.id,
                        task.status.as_str(),
                        task.owner.map(|o| format!(", @{o}")).unwrap_or_default(),
                    ));
                }
                ok(clamp(out))
            }
            Err(e) => fail(format!("không đọc được danh sách việc: {e}")),
        }
    }
}

/// Create a task attached to a meeting.
pub struct CreateTask {
    paths: Arc<Paths>,
}

impl CreateTask {
    #[must_use]
    pub fn new(paths: Arc<Paths>) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl Tool for CreateTask {
    fn name(&self) -> &str {
        "create_task"
    }

    fn description(&self) -> &str {
        "Thêm một việc cần làm vào buổi họp. Dùng khi transcript có ai đó nhận việc mà chưa được ghi lại."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "meeting": { "type": "string", "description": "Id buổi họp" },
                "text": { "type": "string", "description": "Nội dung việc" },
                "owner": { "type": "string", "description": "Người nhận việc, không có thì bỏ trống" },
                "due": { "type": "string", "description": "Hạn, dạng YYYY-MM-DD" }
            },
            "required": ["meeting", "text"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Two writes to the same Markdown file would race.
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let (Some(meeting), Some(text)) = (
            input.get("meeting").and_then(Value::as_str),
            input.get("text").and_then(Value::as_str),
        ) else {
            return fail("cần cả `meeting` và `text`");
        };

        match summo_vault::tasks_io::create(
            &self.paths,
            &summo_core::MeetingId::from(meeting.to_string()),
            text,
            input.get("owner").and_then(Value::as_str),
            input.get("due").and_then(Value::as_str),
        ) {
            Ok(task) => ok(format!("Đã thêm việc `{}` (id:{})", task.text, task.id)),
            Err(e) => fail(format!("không thêm được việc: {e}")),
        }
    }
}

/// Change a task's status or owner.
pub struct UpdateTask {
    paths: Arc<Paths>,
}

impl UpdateTask {
    #[must_use]
    pub fn new(paths: Arc<Paths>) -> Self {
        Self { paths }
    }
}

#[async_trait]
impl Tool for UpdateTask {
    fn name(&self) -> &str {
        "update_task"
    }

    fn description(&self) -> &str {
        "Đổi trạng thái hoặc người nhận của một việc đã có."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "status": {
                    "type": "string",
                    "enum": ["todo", "doing", "done", "blocked", "failed"]
                },
                "owner": { "type": "string" }
            },
            "required": ["id"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(id) = input.get("id").and_then(Value::as_str) else {
            return fail("thiếu tham số `id`");
        };
        let status = input
            .get("status")
            .and_then(Value::as_str)
            .and_then(parse_status);
        let owner = input
            .get("owner")
            .and_then(Value::as_str)
            .map(|o| Some(o.to_string()));

        match summo_vault::tasks_io::update(&self.paths, id, status, owner, None) {
            Ok(task) => ok(format!(
                "Việc `{}` giờ là {}",
                task.text,
                task.status.as_str()
            )),
            Err(e) => fail(format!("không sửa được việc: {e}")),
        }
    }
}

fn parse_status(value: &str) -> Option<Status> {
    match value {
        "todo" => Some(Status::Todo),
        "doing" => Some(Status::Doing),
        "done" => Some(Status::Done),
        "blocked" => Some(Status::Blocked),
        "failed" => Some(Status::Failed),
        _ => None,
    }
}

fn read_all_tasks(paths: &Paths) -> summo_core::Result<Vec<tasks::Task>> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault)?;
    let mut all = Vec::new();
    for entry in index.entries() {
        let path = vault.join(&entry.path);
        if let Ok(body) = std::fs::read_to_string(&path) {
            all.extend(tasks::parse(&body, &entry.path.display().to_string()));
        }
    }
    Ok(all)
}

/// Every Summo tool, ready to hand to the engine.
#[must_use]
pub fn all(paths: Arc<Paths>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchTranscripts::new(paths.clone())),
        Arc::new(GetMeeting::new(paths.clone())),
        Arc::new(ListTasks::new(paths.clone())),
        Arc::new(CreateTask::new(paths.clone())),
        Arc::new(UpdateTask::new(paths)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault_with(tasks_block: &str) -> (TempDir, Arc<Paths>) {
        let dir = TempDir::new().expect("tempdir");
        let paths = Paths::at(dir.path());
        std::fs::create_dir_all(paths.meetings()).expect("mkdir");
        std::fs::write(
            paths.meetings().join("01A.md"),
            format!(
                "---\nid: 01A\ndate: 2026-08-10T09:00:00+07:00\nduration: 600\n\
                 participants: [\"Bạn\", \"Ngọc\"]\ntags: []\n---\n# Họp ngân sách\n\n\
                 ## Tóm tắt\nChốt ngân sách quý 4.\n\n{tasks_block}\n\
                 ## Transcript\n**[00:00:10] Ngọc** — Mình cần chốt ngân sách trước thứ sáu\n"
            ),
        )
        .expect("write");
        (dir, Arc::new(paths))
    }

    #[tokio::test]
    async fn search_finds_a_meeting_by_what_was_said() {
        let (_d, paths) = vault_with("");
        let result = SearchTranscripts::new(paths)
            .execute(json!({ "query": "ngân sách" }))
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("Họp ngân sách"), "{}", result.content);
    }

    #[tokio::test]
    async fn search_says_so_when_there_is_nothing() {
        let (_d, paths) = vault_with("");
        let result = SearchTranscripts::new(paths)
            .execute(json!({ "query": "không tồn tại xyz" }))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("Không tìm thấy"));
    }

    #[tokio::test]
    async fn a_missing_argument_is_an_error_not_a_panic() {
        let (_d, paths) = vault_with("");
        assert!(SearchTranscripts::new(paths.clone()).execute(json!({})).await.is_error);
        assert!(GetMeeting::new(paths.clone()).execute(json!({})).await.is_error);
        assert!(UpdateTask::new(paths).execute(json!({})).await.is_error);
    }

    #[tokio::test]
    async fn get_meeting_omits_the_transcript_unless_asked() {
        let (_d, paths) = vault_with("");
        let tool = GetMeeting::new(paths);

        let brief = tool.execute(json!({ "id": "01A" })).await;
        assert!(brief.content.contains("Chốt ngân sách quý 4"));
        assert!(!brief.content.contains("Mình cần chốt ngân sách trước thứ sáu"));

        let full = tool.execute(json!({ "id": "01A", "transcript": true })).await;
        assert!(full.content.contains("Mình cần chốt ngân sách trước thứ sáu"));
    }

    /// The tools take ids, never paths, so a hallucinated path is simply not a meeting.
    #[tokio::test]
    async fn a_path_where_an_id_belongs_finds_nothing() {
        let (_d, paths) = vault_with("");
        for id in ["../../../etc/passwd", "/etc/passwd", "01A/../../secret"] {
            let result = GetMeeting::new(paths.clone()).execute(json!({ "id": id })).await;
            assert!(result.is_error, "{id} was accepted: {}", result.content);
            assert!(!result.content.contains("root:"), "{id} leaked a file");
        }
    }

    #[tokio::test]
    async fn tasks_can_be_listed_and_filtered() {
        let (_d, paths) = vault_with(
            "## Việc cần làm\n\
             - [ ] @ngoc Chốt spec <!-- id:T1 status:doing -->\n\
             - [ ] @binh Gọi khách <!-- id:T2 -->\n",
        );
        let tool = ListTasks::new(paths);

        let all = tool.execute(json!({})).await;
        assert!(all.content.contains("Chốt spec") && all.content.contains("Gọi khách"));

        let mine = tool.execute(json!({ "owner": "ngoc" })).await;
        assert!(mine.content.contains("Chốt spec"));
        assert!(!mine.content.contains("Gọi khách"));

        let doing = tool.execute(json!({ "status": "doing" })).await;
        assert!(doing.content.contains("Chốt spec"));
        assert!(!doing.content.contains("Gọi khách"));
    }

    #[tokio::test]
    async fn a_task_can_be_created_and_then_moved() {
        let (_d, paths) = vault_with("## Việc cần làm\n- [ ] @ngoc Cũ <!-- id:T1 -->\n");

        let created = CreateTask::new(paths.clone())
            .execute(json!({ "meeting": "01A", "text": "Gửi báo giá", "owner": "binh" }))
            .await;
        assert!(!created.is_error, "{}", created.content);

        let listed = ListTasks::new(paths.clone()).execute(json!({ "owner": "binh" })).await;
        assert!(listed.content.contains("Gửi báo giá"), "{}", listed.content);

        let moved = UpdateTask::new(paths.clone())
            .execute(json!({ "id": "T1", "status": "done" }))
            .await;
        assert!(!moved.is_error, "{}", moved.content);
        let after = ListTasks::new(paths).execute(json!({ "status": "done" })).await;
        assert!(after.content.contains("Cũ"), "{}", after.content);
    }

    /// Two writes to the same Markdown file would race, so the engine must not run them together.
    #[test]
    fn writing_tools_are_not_concurrency_safe() {
        let (_d, paths) = vault_with("");
        assert!(!CreateTask::new(paths.clone()).is_concurrency_safe(&json!({})));
        assert!(!UpdateTask::new(paths.clone()).is_concurrency_safe(&json!({})));
        assert!(SearchTranscripts::new(paths).is_concurrency_safe(&json!({})));
    }

    #[test]
    fn every_tool_is_offered_with_a_schema_and_a_category() {
        let (_d, paths) = vault_with("");
        let tools = all(paths);
        assert_eq!(tools.len(), 5);
        for tool in &tools {
            assert!(!tool.name().is_empty());
            assert!(tool.description().len() > 20, "{} needs a real description", tool.name());
            assert_eq!(tool.input_schema()["type"], "object", "{}", tool.name());
        }
        // The two that write are the two the engine should be able to gate on.
        let writers: Vec<&str> = tools
            .iter()
            .filter(|t| matches!(t.category(), ToolCategory::Edit))
            .map(|t| t.name())
            .collect();
        assert_eq!(writers, vec!["create_task", "update_task"]);
    }

    #[test]
    fn long_output_is_cut_at_a_character_boundary_and_says_so() {
        let long = "ề".repeat(MAX_CHARS + 500);
        let out = clamp(long);
        assert!(out.contains("đã cắt bớt"));
        assert!(out.is_char_boundary(out.len()));
    }
}
