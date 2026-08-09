//! The client against a real socket.
//!
//! Unit tests cover request shaping and response parsing, but neither exercises the part most
//! likely to break: streaming. Server-sent events arrive split across TCP reads at arbitrary
//! boundaries, and a parser that assumes one event per chunk works perfectly in a unit test and
//! drops text in production. These tests deliberately split events mid-line.

use std::net::SocketAddr;

use summo_llm::{LlmClient, Message, Provider};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Serve one HTTP response, then close. Returns the address to point a client at.
async fn serve_once(response: Vec<u8>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0_u8; 4096];
        let _ = socket.read(&mut buf).await;
        socket.write_all(&response).await.unwrap();
        socket.flush().await.unwrap();
    });

    addr
}

/// Serve a streaming response, writing each piece separately so the client sees split reads.
async fn serve_stream(pieces: Vec<String>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0_u8; 4096];
        let _ = socket.read(&mut buf).await;

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for piece in pieces {
            socket.write_all(piece.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });

    addr
}

fn http_json(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn client_for(addr: SocketAddr) -> LlmClient {
    LlmClient::new(Provider::custom(
        "mock",
        &format!("http://{addr}/v1"),
        "test-model",
    ))
    .unwrap()
}

#[tokio::test]
async fn a_complete_request_returns_the_assistant_text() {
    let addr = serve_once(http_json(
        "200 OK",
        r#"{"choices":[{"message":{"content":"xin chào"}}]}"#,
    ))
    .await;

    let text = client_for(addr)
        .complete(&[Message::user("hello")])
        .await
        .unwrap();

    assert_eq!(text, "xin chào");
}

#[tokio::test]
async fn a_provider_error_surfaces_its_own_message() {
    // Status alone is useless: "model not found" and "out of quota" are both 4xx, and the user can
    // act on one and not the other.
    let addr = serve_once(http_json(
        "404 Not Found",
        r#"{"error":{"message":"model `nope` not found"}}"#,
    ))
    .await;

    let err = client_for(addr)
        .complete(&[Message::user("hello")])
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("404"), "got: {err}");
    assert!(
        err.contains("not found"),
        "the provider's own message must survive: {err}"
    );
}

#[tokio::test]
async fn malformed_json_is_an_error_not_an_empty_answer() {
    let addr = serve_once(http_json("200 OK", "{ this is not json")).await;

    let err = client_for(addr)
        .complete(&[Message::user("hello")])
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("malformed"), "got: {err}");
}

#[tokio::test]
async fn streaming_reassembles_events_split_across_reads() {
    // The event for "chào" is deliberately cut in half mid-JSON, which is exactly what a real
    // socket does and what a naive per-chunk parser silently drops.
    let addr = serve_stream(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"xin \"}}]}\n\n".into(),
        "data: {\"choices\":[{\"delta\":{\"cont".into(),
        "ent\":\"chào\"}}]}\n\n".into(),
        "data: [DONE]\n\n".into(),
    ])
    .await;

    let mut chunks = Vec::new();
    let full = client_for(addr)
        .stream(&[Message::user("hello")], |c| chunks.push(c.to_string()))
        .await
        .unwrap();

    assert_eq!(full, "xin chào");
    assert_eq!(
        chunks,
        vec!["xin ", "chào"],
        "each delta should arrive as it lands"
    );
}

#[tokio::test]
async fn streaming_stops_at_the_done_marker() {
    let addr = serve_stream(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n".into(),
        "data: [DONE]\n\n".into(),
        "data: {\"choices\":[{\"delta\":{\"content\":\" ignored\"}}]}\n\n".into(),
    ])
    .await;

    let full = client_for(addr)
        .stream(&[Message::user("hello")], |_| {})
        .await
        .unwrap();

    assert_eq!(full, "ok", "anything after [DONE] must be discarded");
}

#[tokio::test]
async fn keepalive_comments_and_blank_lines_are_skipped() {
    // Providers send `:` comment lines to hold the connection open.
    let addr = serve_stream(vec![
        ": ping\n\n".into(),
        "\n".into(),
        "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n\n".into(),
        "data: [DONE]\n\n".into(),
    ])
    .await;

    let full = client_for(addr)
        .stream(&[Message::user("x")], |_| {})
        .await
        .unwrap();

    assert_eq!(full, "one");
}

#[tokio::test]
async fn a_stream_that_ends_without_done_still_returns_what_arrived() {
    // Connections drop. Losing a partly-streamed translation because the terminator never came
    // would be worse than showing what was received.
    let addr = serve_stream(vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"một nửa\"}}]}\n\n".into(),
    ])
    .await;

    let full = client_for(addr)
        .stream(&[Message::user("x")], |_| {})
        .await
        .unwrap();

    assert_eq!(full, "một nửa");
}

#[tokio::test]
async fn a_health_check_round_trips() {
    let addr = serve_once(http_json(
        "200 OK",
        r#"{"choices":[{"message":{"content":"ok"}}]}"#,
    ))
    .await;
    assert_eq!(client_for(addr).health_check().await.unwrap(), "ok");
}

#[tokio::test]
async fn an_unreachable_endpoint_fails_quickly_and_says_which_provider() {
    // Port 1 on loopback refuses immediately.
    let client =
        LlmClient::new(Provider::custom("My Server", "http://127.0.0.1:1/v1", "m")).unwrap();
    let err = client.health_check().await.unwrap_err().to_string();
    assert!(
        err.contains("My Server"),
        "the message should name the endpoint: {err}"
    );
}
