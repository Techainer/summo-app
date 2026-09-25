//! Anything that starts the daemon also listens for `summo stop`.
//!
//! There are two entry points. `summo serve` waits on Ctrl-C and on `Server::stop_requested`, in a
//! `select!`, with a comment saying why both. The standalone `summo-engine` binary — the one the
//! desktop app spawns as its sidecar — waited on Ctrl-C alone.
//!
//! So `POST /shutdown` answered `{"stopping": true}` and nothing was listening. `summo stop` polled
//! for five seconds and reported *"daemon nhận lệnh dừng nhưng vẫn đang chạy"*, which was exactly
//! true and read like a hang. Starting the daemon again then overwrote `engine.json`, leaving the
//! first process serving a port no later command could find — two daemons on one vault, one of them
//! unreachable for the rest of its life.
//!
//! `stop_requested`'s own documentation had claimed the opposite for as long as it existed:
//! *"Awaited beside Ctrl-C, so a daemon started in the background and one started in a terminal
//! stop the same way and run the same cleanup."* One of the two did.
//!
//! A source check rather than a running one on purpose. The defect was never in `Server` — the
//! route worked, the notify worked, the waiter was correct — it was one caller of two forgetting to
//! await it, and the shape of that mistake is visible in the file that makes it.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// Every Rust source in the workspace's crates.
fn sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root().join("crates")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Starts a server, and correctly does not wait to be told to stop.
///
/// `embedded.rs` is the engine running *inside* the app's own process, which is the only shape
/// mobile allows. There is no second process to outlive anything and no `summo` binary on a phone
/// to send `summo stop` from: the app owns the engine's lifetime and calls `Embedded::shutdown`
/// when the window really is closing. A recording has to survive the window being hidden, so
/// stopping is an explicit act by the owner rather than something the server decides for itself.
const OWNED_BY_ITS_HOST: &[&str] = &["summo-engine/src/embedded.rs"];

#[test]
fn a_daemon_that_can_be_started_can_be_stopped() {
    let mut deaf = Vec::new();

    for path in sources() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // The server's own module defines `start` and `stop_requested`; its tests start servers to
        // exercise routes and shut them down directly. Neither is a daemon somebody runs.
        if path.ends_with("summo-engine/src/server.rs") {
            continue;
        }
        if OWNED_BY_ITS_HOST.iter().any(|named| path.ends_with(named)) {
            continue;
        }
        if !text.contains("Server::start(") {
            continue;
        }
        // A test that starts a server to make one request is not a daemon either — it ends when the
        // test does. What this is about is a process a person starts and later wants to stop.
        if path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        if !text.contains("stop_requested()") {
            deaf.push(
                path.strip_prefix(root())
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }

    assert!(
        deaf.is_empty(),
        "this starts a daemon and never awaits `Server::stop_requested()`, so `summo stop` reaches \
         it, is answered, and changes nothing:\n  {}",
        deaf.join("\n  ")
    );
}
