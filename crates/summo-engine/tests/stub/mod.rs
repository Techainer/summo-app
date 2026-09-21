//! A language model that is a socket.
//!
//! Shared by the integration tests that care about the HTTP conversation rather than about a
//! provider. Deliberately not a mock of `LlmClient`: the things these tests exist to catch — a
//! prompt assembled wrongly, a response parsed into the wrong sections, one request where there
//! should have been three — all live on the wire, and a mock at the client boundary passes while
//! the wire is wrong.
//!
//! Extracted from `tests/translate.rs`, which had the only copy. The summary path needed the same
//! server and a second copy of a request reader is a second thing to get subtly different.

// Each integration test is its own crate and compiles its own copy of this module, so anything only
// one of them uses is dead code in the other. `failing` is used by `summarize.rs` and not by
// `translate.rs`; without this, adding a helper for one suite breaks the build of the other.
#![allow(dead_code)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Every request body the stub saw, in arrival order.
pub type Seen = Arc<Mutex<Vec<String>>>;

/// A model that answers every request with `reply`, recording what it was asked.
pub async fn stub(reply: &'static str) -> (SocketAddr, Seen) {
    serve(move |_| {
        let body = format!(
            r#"{{"choices":[{{"message":{{"role":"assistant","content":{}}}}}]}}"#,
            serde_json::to_string(reply).unwrap()
        );
        ok(&body)
    })
    .await
}

/// A model that fails, so a caller's handling of somebody else's error can be driven.
pub async fn failing(status: &'static str, body: &'static str) -> (SocketAddr, Seen) {
    serve(move |_| {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    })
    .await
}

/// Wrap a JSON body in a 200.
fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Accept connections until dropped, answering each with `respond(request_body)`.
async fn serve<F>(respond: F) -> (SocketAddr, Seen)
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let respond = Arc::new(respond);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            let respond = respond.clone();
            tokio::spawn(async move {
                let Some(body) = read_request(&mut socket).await else {
                    return;
                };
                let response = respond(&body);
                recorded.lock().unwrap().push(body);
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (addr, seen)
}

/// Read one request and return its body.
///
/// Reads until the body is complete rather than taking the first chunk. `Content-Length` is the
/// only framing here, and stopping at the first read truncates anything larger than a TCP segment —
/// which a transcript always is, so the assertions would be checking half a prompt.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buf = [0_u8; 8192];
    loop {
        let n = socket.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        request.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&request);
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let want: usize = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length: ")
                        .or(l.strip_prefix("Content-Length: "))
                })
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if body.len() >= want {
                return Some(body.to_string());
            }
        }
    }
}
