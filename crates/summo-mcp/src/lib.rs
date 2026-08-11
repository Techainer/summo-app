//! The vault, as a tool an assistant can call.
//!
//! Summo already knows what was said in every meeting, and the model people ask about their work is
//! usually the one in their editor. An MCP server is what closes that gap: point Claude Code or
//! Cursor at this binary and "what did we decide about the budget?" is answered from the user's own
//! transcripts rather than guessed at.
//!
//! It is also the cheapest distribution Summo has. Somebody who never opens the app can still be
//! using it, through an editor they already have open.
//!
//! ## What it deliberately does not do
//!
//! **It reads. It does not write.** No tool here creates a task, edits a note or starts a recording.
//! An MCP client is a model with a tool list, and a model that misreads an instruction should not be
//! able to rewrite somebody's meeting notes. Writing is what the app and the daemon are for, behind
//! a person pressing a button.
//!
//! **It never leaves the machine.** The vault is Markdown on disk; this reads those files directly
//! rather than going through the daemon. So it works with the app closed, and there is no port, no
//! token and no second copy of the auth story to get wrong.
//!
//! **It returns text, with citations.** Every excerpt carries the meeting id and timestamp it came
//! from, because an answer a user cannot check against the recording is an answer they have to take
//! on trust — which is the thing this exists to avoid.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use summo_core::paths::Paths;

/// The MCP revision this speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Revisions this server can also speak, oldest last.
///
/// The handshake is a negotiation, not an announcement: the spec says a server that supports the
/// version a client asked for must answer with *that* version, and only fall back to naming its own
/// when it cannot. Answering with a newer revision than the client asked for is how a client that
/// pins an older one ends up disconnecting — which is what echoing a constant did.
///
/// Every method here has been stable across all three, so supporting them costs nothing but the
/// honesty of saying which one is in use.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The revision to answer a handshake with.
///
/// The client's, when we speak it. Ours when we do not, which lets the client decide whether to
/// continue or disconnect rather than guessing on its behalf.
#[must_use]
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|asked| SUPPORTED_VERSIONS.into_iter().find(|known| *known == asked))
        .unwrap_or(PROTOCOL_VERSION)
}

/// A JSON-RPC request, as MCP frames it.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    /// Absent for a notification, which expects no reply.
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl Response {
    #[must_use]
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// How many meetings a search returns.
///
/// Six, matching the daemon's own answer path. Enough to span a few conversations, few enough that
/// the excerpts do not crowd out the model's ability to reason over them.
pub const MAX_HITS: usize = 6;

/// The tools this server offers.
///
/// Descriptions are written for a model, not for a changelog: each says what the tool is *for* and
/// when to reach for it, because a model choosing between `search_meetings` and `get_meeting` has
/// only this text to go on.
#[must_use]
pub fn tools() -> Value {
    json!([
        {
            "name": "search_meetings",
            "description": "Search every meeting transcript in the user's Summo vault and return \
    matching excerpts with the meeting they came from and their timestamps. Use this first for any \
    question about what was said, decided or agreed — it searches what the user actually recorded, not \
    general knowledge. Vietnamese is matched with or without diacritics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to look for." },
                    "limit": {
                        "type": "integer",
                        "description": "How many meetings to draw from. Default 6."
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_meeting",
            "description": "Read one meeting in full: its summary, sections and whole transcript. \
    Use after search_meetings when the excerpts are not enough and you need the surrounding \
    conversation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Meeting id, as search_meetings reports it." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "list_meetings",
            "description": "List recent meetings with their dates, titles and folders. Use to find \
    out what the user has been in, or when a search turns up nothing and you need to check whether the \
    meeting exists at all.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "How many to list. Default 20." }
                }
            }
        },
        {
            "name": "list_tasks",
            "description": "List the open tasks across the vault, with who they are assigned to and \
    when they are due. Tasks assigned to @agent are ones Summo runs itself.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

/// Handle one request. `None` for a notification, which gets no reply.
#[must_use]
pub fn handle(paths: &Paths, request: &Request) -> Option<Response> {
    // A notification has no id and expects nothing back. Replying to one is a protocol error that
    // some clients report as a spurious response and others hang on.
    let id = request.id.clone()?;

    match request.method.as_str() {
        "initialize" => Some(Response::ok(
            id,
            json!({
                "protocolVersion": negotiate(
                    request.params.get("protocolVersion").and_then(Value::as_str)
                ),
                // `resources` as well as `tools`, because a vault is a set of documents and that is
                // what resources are for. A client that can list and read them can show the user
                // *which* meeting an answer came from, rather than only quoting it.
                "capabilities": { "tools": {}, "resources": {}, "prompts": {} },
                "serverInfo": { "name": "summo", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "tools/list" => Some(Response::ok(id, json!({ "tools": tools() }))),
        "tools/call" => Some(call(paths, id, &request.params)),
        "resources/list" => Some(match resources(paths) {
            Ok(list) => Response::ok(id, json!({ "resources": list })),
            Err(why) => Response::err(id, -32_603, why),
        }),
        "resources/read" => Some(read_resource(paths, id, &request.params)),
        "prompts/list" => Some(Response::ok(id, json!({ "prompts": prompts() }))),
        "prompts/get" => Some(get_prompt(id, &request.params)),
        "ping" => Some(Response::ok(id, json!({}))),
        other => Some(Response::err(
            id,
            -32_601,
            format!("unknown method `{other}`"),
        )),
    }
}

fn call(paths: &Paths, id: Value, params: &Value) -> Response {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = match name {
        "search_meetings" => search(paths, &args),
        "get_meeting" => get_meeting(paths, &args),
        "list_meetings" => list_meetings(paths, &args),
        "list_tasks" => list_tasks(paths),
        other => Err(format!("unknown tool `{other}`")),
    };

    match result {
        Ok(text) => Response::ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
        // A tool failure is reported as tool content with `isError`, not as a JSON-RPC error: the
        // model should see what went wrong and be able to try something else, rather than have the
        // client treat it as a transport fault.
        Err(message) => Response::ok(
            id,
            json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true
            }),
        ),
    }
}

fn search(paths: &Paths, args: &Value) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if query.is_empty() {
        return Err("search_meetings needs a query".into());
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(MAX_HITS, |n| (n as usize).clamp(1, 20));

    let library = summo_vault::library::Library::new(paths.clone());
    let hits = library.search(query, limit).map_err(|e| e.to_string())?;

    if hits.is_empty() {
        // Said plainly, so the model does not fill the silence with general knowledge.
        return Ok(format!(
            "No meeting in the vault mentions `{query}`. Do not answer from general knowledge; say \
             the vault does not contain it."
        ));
    }

    let mut out = String::new();
    for hit in &hits {
        out.push_str(&format!(
            "## {} ({}, id:{})\n",
            hit.meeting.title, hit.meeting.day, hit.meeting.id
        ));
        for excerpt in &hit.excerpts {
            match excerpt.t0 {
                // The timestamp is what makes a citation checkable, so it travels with the text.
                Some(t) => out.push_str(&format!(
                    "[{}] {} — {}\n",
                    clock(t),
                    excerpt.speaker.as_deref().unwrap_or("?"),
                    excerpt.text.trim()
                )),
                None => out.push_str(&format!("{}\n", excerpt.text.trim())),
            }
        }
        out.push('\n');
    }
    Ok(out)
}

/// The scheme a Summo document is addressed by.
///
/// `summo://meeting/<id>` rather than a `file://` path. The id is stable across a move — a user who
/// drags a note into a folder in Finder has not changed which meeting it is — whereas a path is a
/// fact about today's filing. It also keeps the vault's location off the wire, so a transcript of
/// an agent session does not leak somebody's home directory.
pub const URI_SCHEME: &str = "summo://meeting/";

/// Every document in the vault, as a resource a client can list.
///
/// Both recordings and typed notes: they are the same documents, and a client that showed only one
/// would be hiding half the vault for a distinction the user does not make.
pub fn resources(paths: &Paths) -> Result<Value, String> {
    let library = summo_vault::library::Library::new(paths.clone());
    let index = library.scan().map_err(|e| e.to_string())?;

    let listed: Vec<Value> = index
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "uri": format!("{URI_SCHEME}{}", entry.id.as_str()),
                "name": entry.title,
                // The date and the folder, because a list of forty meetings all called "Weekly" is
                // a list a model cannot choose from.
                "description": describe(entry),
                "mimeType": "text/markdown",
            })
        })
        .collect();
    Ok(Value::Array(listed))
}

/// Enough to tell two meetings with the same title apart.
fn describe(entry: &summo_vault::index::MeetingEntry) -> String {
    let kind = if entry.kind.is_note() {
        "Note"
    } else {
        "Meeting"
    };
    let mut out = format!("{kind}, {}", entry.day);
    if !entry.folder.is_empty() {
        out.push_str(&format!(", in {}", entry.folder));
    }
    if !entry.participants.is_empty() {
        // Without the brackets. `[[Ngọc]]` is how a name is stored so that Obsidian links it; it is
        // not how a person is called, and putting the syntax in front of a model invites it to
        // repeat the syntax back.
        let people: Vec<&str> = entry
            .participants
            .iter()
            .map(|p| p.trim().trim_start_matches("[[").trim_end_matches("]]"))
            .collect();
        out.push_str(&format!(", with {}", people.join(", ")));
    }
    out
}

/// Read one document by URI.
fn read_resource(paths: &Paths, id: Value, params: &Value) -> Response {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    let Some(meeting) = uri.strip_prefix(URI_SCHEME) else {
        return Response::err(
            id,
            -32_602,
            format!("uri must start with `{URI_SCHEME}`, got `{uri}`"),
        );
    };

    // The same rendering `get_meeting` produces, so a client that reads a resource and a model that
    // calls the tool are looking at the same document rather than at two renderings that differ.
    match get_meeting(paths, &json!({ "id": meeting })) {
        Ok(text) => Response::ok(
            id,
            json!({
                "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": text }]
            }),
        ),
        Err(why) => Response::err(id, -32_602, why),
    }
}

/// Ready-made questions, so a client can offer them rather than expecting the user to phrase one.
///
/// Deliberately few. A prompt list is a menu, and a menu of twenty is a menu nobody reads.
#[must_use]
pub fn prompts() -> Value {
    json!([
        {
            "name": "decisions",
            "description": "What was decided about a topic, with the meetings it was decided in.",
            "arguments": [
                { "name": "topic", "description": "What to look for.", "required": true }
            ]
        },
        {
            "name": "catch_up",
            "description": "What has happened recently, and what is still open.",
            "arguments": []
        }
    ])
}

fn get_prompt(id: Value, params: &Value) -> Response {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let topic = params
        .get("arguments")
        .and_then(|a| a.get("topic"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let text = match name {
        "decisions" => format!(
            "Search the Summo vault for what was decided about \"{topic}\". Use search_meetings \
             first. Quote the decision and name the meeting and timestamp it came from, so I can \
             check it against the recording. If nothing was decided, say so rather than inferring."
        ),
        "catch_up" => "List my recent meetings and my open tasks. Summarise what changed and what \
             is waiting on me. Cite the meeting each point came from."
            .to_string(),
        other => return Response::err(id, -32_602, format!("unknown prompt `{other}`")),
    };

    Response::ok(
        id,
        json!({
            "messages": [{ "role": "user", "content": { "type": "text", "text": text } }]
        }),
    )
}

fn get_meeting(paths: &Paths, args: &Value) -> Result<String, String> {
    let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
    if id.is_empty() {
        return Err("get_meeting needs an id".into());
    }

    let library = summo_vault::library::Library::new(paths.clone());
    let detail = library
        .detail(&summo_core::MeetingId::from(id.to_string()))
        .map_err(|e| e.to_string())?;

    let mut out = format!(
        "# {}\n\n{}, {} minutes\n\n",
        detail.summary.title,
        detail.summary.day,
        detail.summary.duration / 60
    );

    for section in &detail.sections {
        out.push_str(&format!(
            "## {}\n{}\n\n",
            section.heading,
            section.body.trim()
        ));
    }

    if !detail.transcript.is_empty() {
        out.push_str("## Transcript\n");
        for segment in &detail.transcript {
            out.push_str(&format!(
                "[{}] {} — {}\n",
                clock(segment.t0),
                segment.speaker.as_ref().map_or("?", |s| s.as_str()),
                segment.text.trim()
            ));
        }
    }
    Ok(out)
}

fn list_meetings(paths: &Paths, args: &Value) -> Result<String, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(20usize, |n| (n as usize).clamp(1, 200));

    let library = summo_vault::library::Library::new(paths.clone());
    // Ungrouped: the interface groups by day for reading, and a model wants one flat list.
    let query = summo_vault::library::LibraryQuery {
        group: summo_vault::library::GroupBy::None,
        ..Default::default()
    };
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let view = library.view(&query, now).map_err(|e| e.to_string())?;

    // The view is grouped for the interface; a model wants a flat, newest-first list.
    let meetings: Vec<_> = view.groups.iter().flat_map(|g| &g.meetings).collect();
    if meetings.is_empty() {
        return Ok("The vault has no meetings yet.".into());
    }

    let mut out = String::from("| id | date | title | folder |\n|---|---|---|---|\n");
    for meeting in meetings.into_iter().take(limit) {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            meeting.id, meeting.day, meeting.title, meeting.folder
        ));
    }
    Ok(out)
}

/// Scan the vault for tasks.
///
/// Deliberately not routed through the daemon's `board` module: depending on `summo-engine` would
/// pull an HTTP server into a stdio binary, and the parsing this needs is one call.
fn list_tasks(paths: &Paths) -> Result<String, String> {
    let vault = paths.vault();
    let index = summo_vault::index::MeetingIndex::scan(&vault).map_err(|e| e.to_string())?;

    let mut open = Vec::new();
    for entry in index.entries() {
        let Ok(body) = std::fs::read_to_string(vault.join(&entry.path)) else {
            // A file that vanished between the scan and the read is not worth failing the whole
            // list over.
            continue;
        };
        open.extend(
            summo_vault::tasks::parse(&body, &entry.path.display().to_string())
                .into_iter()
                .filter(|t| t.status != summo_vault::tasks::Status::Done),
        );
    }

    if open.is_empty() {
        return Ok("Nothing open.".into());
    }

    let mut out = String::from("| owner | task | due | status |\n|---|---|---|---|\n");
    for task in open {
        out.push_str(&format!(
            "| {} | {} | {} | {:?} |\n",
            task.owner.as_deref().unwrap_or(""),
            task.text.trim(),
            task.due.as_deref().unwrap_or(""),
            task.status
        ));
    }
    Ok(out)
}

/// `mm:ss`, matching the citation form the daemon uses.
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: Value) -> Request {
        Request {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    fn empty_vault() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        std::fs::create_dir_all(paths.meetings()).unwrap();
        (tmp, paths)
    }

    /// A meeting and a typed note, because a client that listed only one would be hiding half the
    /// vault for a distinction the user does not make.
    fn vault() -> (tempfile::TempDir, Paths) {
        let (tmp, paths) = empty_vault();
        std::fs::create_dir_all(paths.notes()).unwrap();
        std::fs::write(
            paths.meetings().join("2026-08-10-hop.md"),
            "---\nid: 01A\ndate: 2026-08-10T10:00:00+07:00\nduration: 600\n\
             participants: [\"[[Ngọc]]\"]\n---\n# Họp đầu tuần\n\n\
             ## Tóm tắt\nChốt ngân sách quý bốn.\n\n\
             ## Transcript\n**[00:01:00] Ngọc** — Chốt ngân sách nhé <!-- seq:0 end:65.0 -->\n",
        )
        .unwrap();
        std::fs::write(
            paths.notes().join("y-tuong.md"),
            "---\ntags: [sản-phẩm]\n---\n# Ý tưởng giá\n\nBán 3–4 đô một tháng.\n",
        )
        .unwrap();
        (tmp, paths)
    }

    /// The handshake is a negotiation. A client that pins an older revision must be answered with
    /// *that* revision, or it is entitled to disconnect — which is what echoing a constant caused.
    #[test]
    fn the_handshake_answers_with_the_version_the_client_asked_for() {
        assert_eq!(negotiate(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate(Some("2025-03-26")), "2025-03-26");
        assert_eq!(negotiate(Some("2025-06-18")), "2025-06-18");
    }

    /// And names its own when it cannot, so the client decides whether to continue.
    #[test]
    fn an_unknown_version_gets_ours_rather_than_an_echo() {
        assert_eq!(negotiate(Some("1999-01-01")), PROTOCOL_VERSION);
        assert_eq!(negotiate(None), PROTOCOL_VERSION);
    }

    #[test]
    fn the_handshake_negotiates_through_the_request() {
        let (_tmp, paths) = vault();
        let request = request("initialize", json!({ "protocolVersion": "2024-11-05" }));
        let result = handle(&paths, &request).unwrap().result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["resources"].is_object());
        assert!(result["capabilities"]["prompts"].is_object());
    }

    /// A vault is a set of documents, and a client that can list them can show *which* meeting an
    /// answer came from rather than only quoting it.
    #[test]
    fn every_document_is_listed_as_a_resource() {
        let (_tmp, paths) = vault();
        let result = handle(&paths, &request("resources/list", json!({})))
            .unwrap()
            .result
            .unwrap();
        let listed = result["resources"].as_array().unwrap();
        assert!(!listed.is_empty(), "the seeded vault must list");
        for entry in listed {
            let uri = entry["uri"].as_str().unwrap();
            assert!(uri.starts_with(URI_SCHEME), "{uri}");
            assert_eq!(entry["mimeType"], "text/markdown");
            assert!(!entry["description"].as_str().unwrap().is_empty());
        }
    }

    /// The id, not the path. A user who drags a note into a folder has not changed which meeting it
    /// is — and a path on the wire would put their home directory in an agent's transcript.
    #[test]
    fn a_resource_uri_carries_no_filesystem_path() {
        let (_tmp, paths) = vault();
        let result = handle(&paths, &request("resources/list", json!({})))
            .unwrap()
            .result
            .unwrap();
        let text = result.to_string();
        assert!(
            !text.contains("/tmp/"),
            "a path leaked into the listing: {text}"
        );
        assert!(
            !text.contains(".md"),
            "a filename leaked into the listing: {text}"
        );
    }

    #[test]
    fn reading_a_resource_returns_the_same_document_the_tool_does() {
        let (_tmp, paths) = vault();
        let listed = handle(&paths, &request("resources/list", json!({})))
            .unwrap()
            .result
            .unwrap();
        let uri = listed["resources"][0]["uri"].as_str().unwrap().to_string();
        let id = uri.strip_prefix(URI_SCHEME).unwrap().to_string();

        let read = handle(
            &paths,
            &request("resources/read", json!({ "uri": uri.clone() })),
        )
        .unwrap()
        .result
        .unwrap();
        let via_resource = read["contents"][0]["text"].as_str().unwrap().to_string();
        let via_tool = get_meeting(&paths, &json!({ "id": id })).unwrap();
        assert_eq!(
            via_resource, via_tool,
            "two renderings of one document would drift"
        );
    }

    #[test]
    fn a_uri_in_another_scheme_is_refused_rather_than_read() {
        let (_tmp, paths) = vault();
        for uri in ["file:///etc/passwd", "../../etc/passwd", "summo://note/01A"] {
            let response =
                handle(&paths, &request("resources/read", json!({ "uri": uri }))).unwrap();
            assert!(response.error.is_some(), "{uri} should have been refused");
        }
    }

    #[test]
    fn prompts_are_listed_and_can_be_fetched() {
        let (_tmp, paths) = vault();
        let listed = handle(&paths, &request("prompts/list", json!({})))
            .unwrap()
            .result
            .unwrap();
        assert_eq!(listed["prompts"].as_array().unwrap().len(), 2);

        let got = handle(
            &paths,
            &request(
                "prompts/get",
                json!({ "name": "decisions", "arguments": { "topic": "ngân sách" } }),
            ),
        )
        .unwrap()
        .result
        .unwrap();
        let text = got["messages"][0]["content"]["text"].as_str().unwrap();
        assert!(
            text.contains("ngân sách"),
            "the argument must reach the prompt: {text}"
        );
    }

    #[test]
    fn an_unknown_prompt_is_an_error_not_an_empty_message() {
        let (_tmp, paths) = vault();
        let response = handle(&paths, &request("prompts/get", json!({ "name": "nope" }))).unwrap();
        assert!(response.error.is_some());
    }

    #[test]
    fn initialize_reports_the_protocol_this_actually_speaks() {
        let (_tmp, paths) = empty_vault();
        let response = handle(&paths, &request("initialize", json!({}))).unwrap();
        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "summo");
        assert!(result["capabilities"]["tools"].is_object());
    }

    /// Replying to a notification is a protocol error some clients report as a spurious response
    /// and others hang on.
    #[test]
    fn a_notification_gets_no_reply() {
        let (_tmp, paths) = empty_vault();
        let notification = Request {
            jsonrpc: Some("2.0".into()),
            id: None,
            method: "notifications/initialized".into(),
            params: json!({}),
        };
        assert!(handle(&paths, &notification).is_none());
    }

    #[test]
    fn the_tool_list_names_every_tool_that_can_be_called() {
        let (_tmp, paths) = empty_vault();
        let response = handle(&paths, &request("tools/list", json!({}))).unwrap();
        let names: Vec<String> = response.result.unwrap()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "search_meetings",
                "get_meeting",
                "list_meetings",
                "list_tasks"
            ]
        );
    }

    /// Every advertised tool must be callable. A list that names a tool `tools/call` rejects is the
    /// kind of bug a model reports as "the tool is broken" and works around forever.
    #[test]
    fn every_advertised_tool_is_dispatchable() {
        let (_tmp, paths) = empty_vault();
        for tool in tools().as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            let response = handle(
                &paths,
                &request("tools/call", json!({ "name": name, "arguments": {} })),
            )
            .unwrap();
            let result = response.result.expect("a result, not a transport error");
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(
                !text.contains("unknown tool"),
                "{name} is advertised but not dispatched"
            );
        }
    }

    #[test]
    fn an_unknown_method_is_a_jsonrpc_error() {
        let (_tmp, paths) = empty_vault();
        let response = handle(&paths, &request("nope", json!({}))).unwrap();
        assert_eq!(response.error.unwrap().code, -32_601);
    }

    /// A tool failure is content the model can read and act on, not a transport fault the client
    /// swallows.
    #[test]
    fn an_unknown_tool_reports_itself_as_tool_content_not_a_transport_error() {
        let (_tmp, paths) = empty_vault();
        let response = handle(
            &paths,
            &request("tools/call", json!({ "name": "nope", "arguments": {} })),
        )
        .unwrap();
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown tool")
        );
    }

    #[test]
    fn searching_for_nothing_is_refused_rather_than_returning_the_whole_vault() {
        let (_tmp, paths) = empty_vault();
        let response = handle(
            &paths,
            &request(
                "tools/call",
                json!({ "name": "search_meetings", "arguments": { "query": "  " } }),
            ),
        )
        .unwrap();
        assert_eq!(response.result.unwrap()["isError"], true);
    }

    /// The whole point of pointing a model at somebody's transcripts is that it stops guessing. A
    /// silent empty result invites it to fill the gap.
    #[test]
    fn an_empty_vault_tells_the_model_not_to_answer_from_general_knowledge() {
        let (_tmp, paths) = empty_vault();
        let response = handle(
            &paths,
            &request(
                "tools/call",
                json!({ "name": "search_meetings", "arguments": { "query": "ngân sách" } }),
            ),
        )
        .unwrap();
        let text = response.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("general knowledge"), "{text}");
    }

    #[test]
    fn getting_a_meeting_without_an_id_says_so() {
        let (_tmp, paths) = empty_vault();
        let response = handle(
            &paths,
            &request(
                "tools/call",
                json!({ "name": "get_meeting", "arguments": {} }),
            ),
        )
        .unwrap();
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("needs an id")
        );
    }

    #[test]
    fn an_empty_vault_lists_no_meetings_without_failing() {
        let (_tmp, paths) = empty_vault();
        let response = handle(
            &paths,
            &request(
                "tools/call",
                json!({ "name": "list_meetings", "arguments": {} }),
            ),
        )
        .unwrap();
        let result = response.result.unwrap();
        assert!(result.get("isError").is_none());
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no meetings")
        );
    }

    #[test]
    fn ping_answers() {
        let (_tmp, paths) = empty_vault();
        assert!(
            handle(&paths, &request("ping", json!({})))
                .unwrap()
                .error
                .is_none()
        );
    }

    /// Read-only is a property of the tool list, and it has to stay one. Anything here that could
    /// write would let a model that misread an instruction rewrite somebody's meeting notes.
    #[test]
    fn no_tool_is_a_writing_tool() {
        for tool in tools().as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            for verb in [
                "create", "update", "delete", "write", "start", "record", "set",
            ] {
                assert!(
                    !name.starts_with(verb),
                    "`{name}` looks like it writes; this server is read-only"
                );
            }
        }
    }

    #[test]
    fn timestamps_are_the_same_form_the_daemon_cites() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(64.7), "01:04");
        assert_eq!(clock(-5.0), "00:00");
    }
}
