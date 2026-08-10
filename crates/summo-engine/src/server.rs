//! The loopback HTTP and WebSocket surface.
//!
//! Bound to `127.0.0.1` on an ephemeral port, with the port and token written to a file the app
//! reads. An ephemeral port rather than a fixed one is deliberate: a well-known port is something
//! another program can find and probe, and there is no reason for this daemon to be findable.

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use summo_core::{Error, Event, Result};

use crate::{
    auth::{SessionToken, origin_is_allowed, token_path},
    protocol::{Command, decode_frame},
    state::EngineState,
};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Port to bind, or 0 for an ephemeral one.
    pub port: u16,
    /// Publish the port and token where the app can find them.
    pub write_token_file: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 0,
            write_token_file: true,
        }
    }
}

/// What the daemon writes for the app to read on startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handshake {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

#[derive(Clone)]
struct AppState {
    engine: EngineState,
    token: SessionToken,
}

/// A running daemon.
pub struct Server {
    addr: SocketAddr,
    token: SessionToken,
    handle: tokio::task::JoinHandle<()>,
}

impl Server {
    /// Bind and start serving. Returns once the socket is listening.
    pub async fn start(engine: EngineState, cfg: ServerConfig) -> Result<Self> {
        let token = SessionToken::generate();
        let state = AppState {
            engine: engine.clone(),
            token: token.clone(),
        };

        let app = Router::new()
            .route("/health", get(health))
            .route("/hw", get(hardware))
            .route("/models", get(models))
            .route("/status", get(status))
            .route("/ws", get(websocket))
            .with_state(state);

        // Loopback only. Binding 0.0.0.0 would expose a user's microphone to their network.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port))
            .await
            .map_err(|e| Error::Other(format!("cannot bind loopback port: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| Error::Other(format!("cannot read bound address: {e}")))?;

        if cfg.write_token_file {
            let path = token_path(engine.paths().root());
            token.write_to(&path)?;
            let handshake = Handshake {
                port: addr.port(),
                token: token.as_str().to_string(),
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            };
            let json_path = engine.paths().root().join("engine.json");
            std::fs::write(&json_path, serde_json::to_vec_pretty(&handshake)?)
                .map_err(|e| Error::io(&json_path, e))?;
        }

        tracing::info!(%addr, "engine listening");
        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "server stopped");
            }
        });

        Ok(Self {
            addr,
            token,
            handle,
        })
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    #[must_use]
    pub fn token(&self) -> &SessionToken {
        &self.token
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

/// Reject anything that is not an authenticated native client.
///
/// Returns `Ok(())` or the status and message to send back.
fn authorize(
    headers: &HeaderMap,
    query_token: Option<&str>,
    expected: &SessionToken,
) -> std::result::Result<(), (StatusCode, &'static str)> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !origin_is_allowed(origin) {
        // A web page cannot forge this header, so an origin at all means a browser is calling.
        return Err((
            StatusCode::FORBIDDEN,
            "this daemon does not serve web origins",
        ));
    }

    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    // WebSocket clients cannot set headers in a browser API, so the token may also arrive as a
    // query parameter. It is the same secret either way, and the origin check above is what stops a
    // page from using it.
    let presented = bearer.or(query_token);

    match presented {
        Some(token) if expected.matches(token) => Ok(()),
        Some(_) => Err((StatusCode::UNAUTHORIZED, "invalid token")),
        None => Err((StatusCode::UNAUTHORIZED, "missing token")),
    }
}

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn hardware(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(&headers, q.token.as_deref(), &state.token) {
        return rejection.into_response();
    }
    Json(state.engine.hardware().clone()).into_response()
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(&headers, q.token.as_deref(), &state.token) {
        return rejection.into_response();
    }
    Json(state.engine.status()).into_response()
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(&headers, q.token.as_deref(), &state.token) {
        return rejection.into_response();
    }
    Json(state.engine.store().list()).into_response()
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(&headers, q.token.as_deref(), &state.token) {
        return rejection.into_response();
    }
    upgrade
        .on_upgrade(move |socket| handle_socket(socket, state.engine))
        .into_response()
}

/// One client connection.
///
/// Audio arrives as binary frames and commands as text; both are handled on this task, which does
/// no decoding itself — that happens on a worker so a slow model cannot stall the socket and back
/// pressure the app's audio queue.
async fn handle_socket(mut socket: WebSocket, engine: EngineState) {
    let to_frame = |event: &Event| {
        let json = serde_json::to_string(event).unwrap_or_default();
        Message::Text(json.into())
    };

    // The recognition pipeline for this connection, created on `session_start`. Held here rather
    // than in shared state because it is not `Sync` and belongs to exactly one client.
    #[cfg(feature = "models")]
    let mut runner: Option<crate::runner::SessionRunner> = None;

    while let Some(Ok(message)) = socket.recv().await {
        let reply = match message {
            #[cfg(feature = "models")]
            Message::Text(text) => {
                let (events, next) = handle_command_with_models(&text, &engine, runner.take());
                runner = next;
                events
            }
            #[cfg(not(feature = "models"))]
            Message::Text(text) => handle_command(&text, &engine),

            #[cfg(feature = "models")]
            Message::Binary(bytes) => handle_audio_with_models(&bytes, &engine, runner.as_mut()),
            #[cfg(not(feature = "models"))]
            Message::Binary(bytes) => handle_audio(&bytes, &engine),

            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
        };

        for event in reply {
            if socket.send(to_frame(&event)).await.is_err() {
                break;
            }
        }
    }

    // A client that vanishes mid-recording must not leave the daemon believing it is still
    // recording, or the next session would be refused.
    if engine.status().is_recording() {
        tracing::warn!("client disconnected while recording; ending the session");
        engine.end();
    }
}

fn handle_command(text: &str, engine: &EngineState) -> Vec<Event> {
    let command: Command = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            return vec![Event::Error {
                message: format!("malformed command: {e}"),
                transient: false,
            }];
        }
    };

    match command {
        Command::Ping => vec![Event::info("pong")],
        Command::SessionStart(spec) => {
            if let Err(e) = spec.validate() {
                return vec![Event::error(&e)];
            }
            match engine.begin(&spec) {
                Ok(()) => vec![Event::info(format!(
                    "session started with {}",
                    spec.live_model
                ))],
                Err(e) => vec![Event::error(&e)],
            }
        }
        Command::SessionStop => {
            engine.end();
            vec![Event::info("session stopped")]
        }
        Command::ModelLoad { id } | Command::ModelSwap { id } | Command::ModelPull { id } => {
            // Model management lands with the `models` feature; refusing clearly beats accepting
            // and silently doing nothing.
            vec![Event::Error {
                message: format!("model management is not built into this binary (requested {id})"),
                transient: false,
            }]
        }
    }
}

/// Command handling when recognition is compiled in: owns creating and tearing down the pipeline.
#[cfg(feature = "models")]
fn handle_command_with_models(
    text: &str,
    engine: &EngineState,
    runner: Option<crate::runner::SessionRunner>,
) -> (Vec<Event>, Option<crate::runner::SessionRunner>) {
    let command: Command = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            return (
                vec![Event::Error {
                    message: format!("malformed command: {e}"),
                    transient: false,
                }],
                runner,
            );
        }
    };

    match command {
        Command::SessionStart(spec) => {
            if let Err(e) = engine.begin(&spec) {
                return (vec![Event::error(&e)], runner);
            }
            match crate::runner::SessionRunner::new(&spec, &engine.store(), engine.hardware()) {
                Ok(new_runner) => (
                    vec![Event::info(format!(
                        "session started with {}",
                        spec.live_model
                    ))],
                    Some(new_runner),
                ),
                Err(e) => {
                    // Loading failed, so the engine must not be left believing it is recording.
                    engine.end();
                    (vec![Event::error(&e)], None)
                }
            }
        }
        Command::SessionStop => {
            let mut events = Vec::new();
            if let Some(mut runner) = runner {
                match runner.flush() {
                    Ok(flushed) => events.extend(flushed),
                    Err(e) => events.push(Event::error(&e)),
                }
            }
            engine.end();
            events.push(Event::info("session stopped"));
            (events, None)
        }
        other => {
            let events = handle_command(&serde_json::to_string(&other).unwrap_or_default(), engine);
            (events, runner)
        }
    }
}

/// Audio handling when recognition is compiled in.
#[cfg(feature = "models")]
fn handle_audio_with_models(
    bytes: &[u8],
    engine: &EngineState,
    runner: Option<&mut crate::runner::SessionRunner>,
) -> Vec<Event> {
    let (lane, samples) = match decode_frame(bytes) {
        Ok(frame) => frame,
        Err(e) => return vec![Event::error(&e)],
    };
    engine.advance(summo_core::audio::samples_to_secs(samples.len()), 0);

    let Some(runner) = runner else {
        // Audio before `session_start`. Saying so beats discarding it silently, which looks
        // identical to a microphone that is not working.
        return vec![Event::Error {
            message: "audio received before the session was started".into(),
            transient: true,
        }];
    };

    match runner.accept(lane, &samples) {
        Ok(events) => {
            let finals = events
                .iter()
                .filter(|e| matches!(e, Event::Final(_)))
                .count();
            engine.advance(0.0, finals as u64);
            events
        }
        Err(e) => vec![Event::error(&e)],
    }
}

#[cfg_attr(feature = "models", allow(dead_code))]
fn handle_audio(bytes: &[u8], engine: &EngineState) -> Vec<Event> {
    match decode_frame(bytes) {
        Ok((_lane, samples)) => {
            engine.advance(summo_core::audio::samples_to_secs(samples.len()), 0);
            Vec::new()
        }
        Err(e) => vec![Event::error(&e)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::paths::Paths;

    async fn running() -> (tempfile::TempDir, Server) {
        let tmp = tempfile::tempdir().unwrap();
        let engine = EngineState::new(Paths::at(tmp.path())).unwrap();
        let server = Server::start(
            engine,
            ServerConfig {
                port: 0,
                write_token_file: true,
            },
        )
        .await
        .unwrap();
        (tmp, server)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn health_needs_no_token() {
        // Something has to be reachable without a credential, or the app cannot tell a daemon that
        // is starting from one that is not there.
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/health", server.addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.json::<serde_json::Value>().await.unwrap()["status"],
            "ok"
        );
        server.shutdown();
    }

    #[tokio::test]
    async fn everything_else_requires_the_token() {
        let (_tmp, server) = running().await;
        for path in ["hw", "models", "status"] {
            let resp = client()
                .get(format!("http://{}/{path}", server.addr()))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 401, "/{path} should have required a token");
        }
        server.shutdown();
    }

    #[tokio::test]
    async fn a_valid_token_is_accepted_as_a_bearer_header() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/hw", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert!(resp.json::<serde_json::Value>().await.unwrap()["cores"].is_number());
        server.shutdown();
    }

    #[tokio::test]
    async fn a_wrong_token_is_refused() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/hw", server.addr()))
            .bearer_auth("0".repeat(64))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_web_page_is_refused_even_with_a_valid_token() {
        // The attack that matters: a page in a background tab reaching the daemon on loopback.
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/hw", server.addr()))
            .bearer_auth(server.token().as_str())
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        server.shutdown();
    }

    #[tokio::test]
    async fn the_daemon_binds_loopback_only() {
        let (_tmp, server) = running().await;
        assert!(
            server.addr().ip().is_loopback(),
            "binding beyond loopback would expose a microphone to the network"
        );
        server.shutdown();
    }

    #[tokio::test]
    async fn the_handshake_file_tells_the_app_where_to_connect() {
        let (tmp, server) = running().await;
        let handshake: Handshake =
            serde_json::from_slice(&std::fs::read(tmp.path().join("engine.json")).unwrap())
                .unwrap();

        assert_eq!(handshake.port, server.addr().port());
        assert!(server.token().matches(&handshake.token));
        assert_eq!(handshake.pid, std::process::id());
        server.shutdown();
    }

    // --- command handling, without a socket ---

    fn engine() -> (tempfile::TempDir, EngineState) {
        let tmp = tempfile::tempdir().unwrap();
        let engine = EngineState::new(Paths::at(tmp.path())).unwrap();
        (tmp, engine)
    }

    #[test]
    fn a_malformed_command_produces_an_error_event_not_a_disconnect() {
        let (_tmp, engine) = engine();
        let events = handle_command("{not json", &engine);
        assert!(matches!(
            events[0],
            Event::Error {
                transient: false,
                ..
            }
        ));
    }

    #[test]
    fn session_start_and_stop_move_the_engine_state() {
        let (_tmp, engine) = engine();
        let start = serde_json::to_string(&Command::SessionStart(Box::new(
            crate::protocol::SessionSpec::new("gipformer-65m"),
        )))
        .unwrap();

        handle_command(&start, &engine);
        assert!(engine.status().is_recording());

        handle_command(r#"{"cmd":"session_stop"}"#, &engine);
        assert!(!engine.status().is_recording());
    }

    #[test]
    fn an_invalid_session_spec_is_reported_rather_than_started() {
        let (_tmp, engine) = engine();
        // Diarization without the system lane: valid JSON, invalid session.
        let start = r#"{"cmd":"session_start","live_model":"m","diarize":true}"#;
        let events = handle_command(start, &engine);

        assert!(matches!(events[0], Event::Error { .. }));
        assert!(
            !engine.status().is_recording(),
            "a refused session must not start"
        );
    }

    #[test]
    fn ping_is_answered() {
        let (_tmp, engine) = engine();
        let events = handle_command(r#"{"cmd":"ping"}"#, &engine);
        assert!(matches!(&events[0], Event::Info { text } if text == "pong"));
    }

    #[test]
    fn model_commands_refuse_clearly_when_not_compiled_in() {
        // Accepting and doing nothing would look like a hung download.
        let (_tmp, engine) = engine();
        let events = handle_command(r#"{"cmd":"model_pull","id":"x"}"#, &engine);
        let Event::Error { message, .. } = &events[0] else {
            panic!("expected an error")
        };
        assert!(
            message.contains("not built into this binary"),
            "got: {message}"
        );
    }

    #[test]
    fn audio_frames_advance_the_clock() {
        let (_tmp, engine) = engine();
        engine
            .begin(&crate::protocol::SessionSpec::new("m"))
            .unwrap();

        let frame = crate::protocol::encode_frame(
            summo_core::segment::Lane::Mic,
            &[0.0; summo_core::audio::FRAME_LEN],
        );
        assert!(handle_audio(&frame, &engine).is_empty());

        let SessionStatusRecording { elapsed_s } = recording_of(&engine);
        assert!(
            (elapsed_s - 0.1).abs() < 1e-6,
            "one 100 ms frame, got {elapsed_s}"
        );
    }

    #[test]
    fn a_corrupt_audio_frame_is_reported_and_ignored() {
        let (_tmp, engine) = engine();
        engine
            .begin(&crate::protocol::SessionSpec::new("m"))
            .unwrap();

        let events = handle_audio(&[9, 0, 0, 0, 0], &engine);
        assert!(matches!(events[0], Event::Error { .. }));

        let SessionStatusRecording { elapsed_s } = recording_of(&engine);
        assert_eq!(
            elapsed_s, 0.0,
            "a rejected frame must not advance the clock"
        );
    }

    struct SessionStatusRecording {
        elapsed_s: f64,
    }

    fn recording_of(engine: &EngineState) -> SessionStatusRecording {
        match engine.status() {
            crate::state::SessionStatus::Recording { elapsed_s, .. } => {
                SessionStatusRecording { elapsed_s }
            }
            other => panic!("expected a recording state, got {other:?}"),
        }
    }
}
