//! `summo mcp`, driven the way a client drives it.
//!
//! Every test this feature had called `summo_mcp::handle` — a function, in process, with a struct
//! handed to it. What an MCP client actually does is spawn a binary and speak JSON-RPC down its
//! stdin, and none of that had ever run: not the argument parsing, not the framing, not where the
//! logs go, not whether the process stays alive across a malformed line.
//!
//! The last of those is the one worth spelling out. Logging goes to stderr because stdout is the
//! protocol stream, and a single log line on stdout is a parse error the client reports as the
//! server being broken rather than as a log line. Both `main`s say so in a comment. A comment is
//! not a check, and this is the only place that can make it one — in process there is no stdout to
//! get wrong.
//!
//! Spawned through `CARGO_BIN_EXE_summo`, so it is the binary this crate builds rather than a
//! rebuilt copy or one that happens to be on the path.

#![cfg(feature = "mcp")]

use std::io::Write;
use std::process::{Command, Stdio};

/// A vault with one meeting in it, so a search has something to find.
fn vault() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("a temporary home");
    let meetings = home.path().join("vault/meetings");
    std::fs::create_dir_all(&meetings).expect("the vault");
    std::fs::write(
        meetings.join("2026-09-20-ngan-sach.md"),
        "---\n\
         id: 01MCP1\n\
         date: 2026-09-20T10:00:00+07:00\n\
         duration: 600\n\
         participants: [\"[[Ngọc]]\"]\n\
         ---\n\n\
         # Họp ngân sách\n\n\
         ## Tóm tắt\n\
         Chốt ngân sách quý bốn.\n\n\
         ## Transcript\n\
         **[00:00:10] Ngọc** — Mình chốt ngân sách hôm nay <!-- seq:0 end:15.0 -->\n",
    )
    .expect("one meeting");
    home
}

/// Feed `lines` to `summo mcp` and collect what comes back on each stream.
fn talk(home: &std::path::Path, lines: &[&str]) -> (Vec<serde_json::Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_summo"))
        .args(["--home", home.to_str().expect("a utf-8 path"), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("summo is built by the time its tests run");

    {
        let stdin = child.stdin.as_mut().expect("piped");
        for line in lines {
            writeln!(stdin, "{line}").expect("the child is still listening");
        }
        // Dropped here, which is the end-of-input a client sends when it disconnects. Without it
        // the loop below waits forever for a process that is waiting forever for another line.
    }
    child.stdin.take();

    let done = child.wait_with_output().expect("the child ended");
    let stdout = String::from_utf8(done.stdout).expect("stdout is utf-8");
    let stderr = String::from_utf8_lossy(&done.stderr).into_owned();

    let replies = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("a line on stdout that is not one JSON object: {line:?} ({e})")
            })
        })
        .collect();

    assert!(done.status.success(), "summo mcp exited: {stderr}");
    (replies, stderr)
}

/// The handshake, a notification, a tool list and a real search, in the order a client sends them.
#[test]
fn a_client_can_connect_list_tools_and_get_an_answer_with_a_citation() {
    let home = vault();
    let (replies, _) = talk(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_meetings","arguments":{"query":"ngân sách"}}}"#,
        ],
    );

    // Three, not four. A notification carries no id, so a reply to it would be a response to
    // nothing — and a client that receives one has nowhere to put it.
    assert_eq!(replies.len(), 3, "{replies:#?}");

    // The version the *client* asked for, not this server's own. Answering with a newer revision
    // than the client pinned is how a client disconnects mid-handshake.
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "summo");

    assert_eq!(replies[1]["id"], 2);
    let tools = replies[1]["result"]["tools"]
        .as_array()
        .expect("a tool list");
    assert!(tools.len() >= 4, "{tools:#?}");

    // And the answer carries where it came from. An excerpt a user cannot check against the
    // recording is one they have to take on trust, which is the thing this server exists to avoid.
    assert_eq!(replies[2]["id"], 3);
    let text = replies[2]["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.contains("Họp ngân sách"), "{text}");
    assert!(
        text.contains("01MCP1"),
        "the excerpt cites no meeting: {text}"
    );
}

/// Nothing but protocol reaches stdout, and the log reaches stderr.
///
/// The invariant both `main`s document in a comment and neither could check: in process there is
/// no stdout to get wrong. One `tracing` line written to the wrong stream is a parse error at the
/// client, reported as the server being broken.
#[test]
fn the_log_goes_to_stderr_and_stdout_carries_only_json() {
    let home = vault();
    let (replies, stderr) = talk(
        home.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#],
    );

    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], 1);
    // The startup line names the vault, so its absence here would mean the check is passing
    // because nothing was logged at all rather than because it was logged in the right place.
    assert!(
        stderr.contains("serving the vault over stdio"),
        "nothing was logged, so this proves nothing: {stderr:?}"
    );
}

/// A bad line does not end the session.
///
/// A transport that dies on one malformed request takes the whole conversation with it, and the
/// user sees an editor that lost its connection for no reason they can see.
#[test]
fn a_malformed_line_is_skipped_and_the_next_one_still_answers() {
    let home = vault();
    let (replies, _) = talk(
        home.path(),
        &[
            "{not json at all",
            "",
            "   ",
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_meetings","arguments":{}}}"#,
        ],
    );

    assert_eq!(replies.len(), 1, "{replies:#?}");
    assert_eq!(replies[0]["id"], 5);
}

/// An unknown method answers with its id rather than going quiet.
///
/// Silence is indistinguishable from a hang, and a client waiting on a reply that will never come
/// sends whoever is debugging it looking in the wrong place.
#[test]
fn an_unknown_method_is_an_error_and_not_a_hang() {
    let home = vault();
    let (replies, _) = talk(
        home.path(),
        &[r#"{"jsonrpc":"2.0","id":9,"method":"sing/aSong","params":{}}"#],
    );

    assert_eq!(replies.len(), 1, "{replies:#?}");
    assert_eq!(replies[0]["id"], 9);
    assert_eq!(replies[0]["error"]["code"], -32_601);
}

/// It reads and does not write — checked against the disk rather than against the tool list.
///
/// The tool list is the claim; the vault is the evidence. A future tool that writes would pass a
/// check that only read the list of tool names.
#[test]
fn nothing_a_client_can_say_changes_the_vault() {
    let home = vault();
    let file = home.path().join("vault/meetings/2026-09-20-ngan-sach.md");
    let before = std::fs::read_to_string(&file).expect("the meeting");

    talk(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_meeting","arguments":{"id":"01MCP1"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"resources/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"prompts/list","params":{}}"#,
        ],
    );

    assert_eq!(
        std::fs::read_to_string(&file).expect("the meeting is still there"),
        before,
        "an MCP session rewrote a meeting"
    );
}
