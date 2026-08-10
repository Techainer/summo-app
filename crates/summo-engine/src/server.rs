//! The loopback HTTP and WebSocket surface.
//!
//! Bound to `127.0.0.1` on an ephemeral port, with the port and token written to a file the app
//! reads. An ephemeral port rather than a fixed one is deliberate: a well-known port is something
//! another program can find and probe, and there is no reason for this daemon to be findable.

use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use summo_core::{Error, Event, Result};
use summo_vault::library::{Library, LibraryQuery};

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
    /// Accept requests from pages served on this machine.
    ///
    /// Off in anything shipped. It exists so the interface can be developed against a Vite server
    /// and driven in a browser test; see [`crate::auth::origin_is_allowed`] for why it is opt-in.
    pub allow_loopback_origins: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 0,
            write_token_file: true,
            allow_loopback_origins: false,
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
    /// Loaded once and shared: the book answers "who just spoke?" for every utterance, so it is
    /// held in memory rather than re-read per request. See `crate::voicebook`.
    book: crate::voicebook::SharedBook,
    library: Library,
    token: SessionToken,
    allow_loopback_origins: bool,
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
            book: crate::voicebook::SharedBook::load(&engine.paths().voices())?,
            library: Library::new(engine.paths().clone()),
            engine: engine.clone(),
            token: token.clone(),
            allow_loopback_origins: cfg.allow_loopback_origins,
        };

        if cfg.allow_loopback_origins {
            tracing::warn!(
                "development mode: pages served from this machine may reach the daemon. \
                 Do not run a shipped build this way."
            );
        }

        let app = Router::new()
            .route("/health", get(health))
            .route("/hw", get(hardware))
            .route("/models", get(models))
            .route("/status", get(status))
            .route("/storage", get(storage))
            .route("/storage/prune", post(prune_storage))
            .route("/meetings/{id}/audio", axum::routing::delete(forget_audio))
            .route("/settings", get(settings))
            .route("/settings/llm", post(set_llm))
            .route("/settings/llm/test", post(test_llm))
            .route("/report", get(report))
            .route("/tasks", get(tasks))
            .route("/tasks/{id}", post(update_task))
            .route("/meetings/{id}/tasks", post(create_task))
            .route("/templates", get(templates))
            .route("/meetings/{id}/summarize", post(summarize_meeting))
            .route("/people", get(people))
            .route("/people/{id}/name", post(rename_person))
            .route("/people/{id}/avatar", post(set_person_avatar))
            .route("/people/{id}/merge", post(merge_person))
            .route("/people/{id}", axum::routing::delete(forget_person))
            .route("/meetings/{id}/audio/{lane}", get(meeting_audio))
            .route("/meetings/{id}/voices", get(unknown_voices))
            .route("/meetings/{id}/voices/{label}", post(name_voice))
            .route("/library", get(library))
            .route("/library/search", get(search))
            .route("/meetings/{id}", get(meeting))
            .route("/meetings/{id}/folder", post(set_folder))
            .route("/meetings/{id}/tags", post(set_tags))
            .route("/meetings/{id}/title", post(set_title))
            .route("/meetings/{id}/trash", post(trash))
            .route("/ws", get(websocket))
            .layer(middleware::from_fn_with_state(state.clone(), cors))
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

/// Tell the browser what the daemon already decided.
///
/// [`origin_is_allowed`] governs whether a request is served at all; this reports that same
/// decision back as CORS headers, because a browser blocks a cross-origin reply it was not
/// explicitly promised — the daemon would answer 200 and the app would still see a network error.
/// The two must never drift, so this asks the same function rather than keeping its own list.
///
/// In a shipped build no origin is allowed, so no page gets these headers.
async fn cors(State(state): State<AppState>, request: Request<axum::body::Body>, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let allowed = origin
        .as_deref()
        .is_some_and(|o| origin_is_allowed(Some(o), state.allow_loopback_origins));

    // A preflight carries no token — it is the browser asking whether it may send one — so it is
    // answered before authorization rather than rejected for lacking it.
    let mut response = if request.method() == Method::OPTIONS && allowed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };

    if let Some(origin) = origin {
        let headers = response.headers_mut();
        // Vary regardless of the outcome: a cache must not serve one origin's answer to another.
        headers.insert(header::VARY, HeaderValue::from_static("origin"));
        if allowed && let Ok(value) = HeaderValue::from_str(&origin) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("authorization, content-type"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, OPTIONS"),
            );
        }
    }
    response
}

/// Reject anything that is not an authenticated native client.
///
/// Returns `Ok(())` or the status and message to send back.
fn authorize(
    headers: &HeaderMap,
    query_token: Option<&str>,
    expected: &SessionToken,
    allow_loopback_origins: bool,
) -> std::result::Result<(), (StatusCode, &'static str)> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !origin_is_allowed(origin, allow_loopback_origins) {
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
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    Json(state.engine.hardware().clone()).into_response()
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    Json(state.engine.status()).into_response()
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    Json(state.engine.store().list()).into_response()
}

/// Turn a vault failure into a status a client can act on.
///
/// A missing meeting is a 404 and everything else a 400: the app distinguishes "this is gone,
/// refresh the list" from "that request was wrong", and a blanket 500 would hide both.
fn vault_error(e: &Error) -> (StatusCode, String) {
    let message = e.to_string();
    // "Not there" and "you asked for something impossible" are different answers, and an interface
    // that shows a stale person should be told to drop them rather than shown a validation error.
    let status = if message.contains("no meeting with id")
        || message.contains("no person with id")
        || message.contains("no task with id")
        || message.contains("no template with id")
        || message.contains("no voice log for meeting")
    {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, message)
}

fn as_response(result: Result<impl Serialize>) -> axum::response::Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => {
            let (status, message) = vault_error(&e);
            (status, Json(serde_json::json!({ "error": message }))).into_response()
        }
    }
}

async fn library(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LibraryQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    // The clock is read here rather than inside the vault so "the last seven days" is anchored to
    // the machine the user is looking at, in its own offset.
    let now = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    as_response(state.library.view(&q, now))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    token: Option<String>,
    #[serde(default)]
    q: String,
    limit: Option<usize>,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        query.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    // A search box sends a request per keystroke; the cap keeps the worst case bounded no matter
    // what a client asks for.
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    as_response(state.library.search(&query.q, limit))
}

async fn meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(state.library.detail(&summo_core::MeetingId::from(id)))
}

#[derive(Debug, Deserialize)]
struct FolderBody {
    folder: String,
}

async fn set_folder(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<FolderBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let id = summo_core::MeetingId::from(id);
    as_response(
        state
            .library
            .move_to_folder(&id, &body.folder)
            .map(|_| serde_json::json!({ "folder": body.folder })),
    )
}

#[derive(Debug, Deserialize)]
struct TagsBody {
    tags: Vec<String>,
}

async fn set_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<TagsBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let id = summo_core::MeetingId::from(id);
    as_response(
        state
            .library
            .set_tags(&id, body.tags)
            .map(|tags| serde_json::json!({ "tags": tags })),
    )
}

#[derive(Debug, Deserialize)]
struct TitleBody {
    title: String,
}

async fn set_title(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<TitleBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let id = summo_core::MeetingId::from(id);
    as_response(
        state
            .library
            .rename(&id, &body.title)
            .map(|title| serde_json::json!({ "title": title })),
    )
}

async fn trash(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let id = summo_core::MeetingId::from(id);
    as_response(
        state
            .library
            .trash(&id)
            .map(|_| serde_json::json!({ "trashed": true })),
    )
}

async fn storage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(summo_vault::storage::usage(state.engine.paths()))
}

#[derive(Debug, Deserialize)]
struct PruneQuery {
    token: Option<String>,
    /// Report what would go without deleting it. Defaults to true, so a mistyped request cannot
    /// delete anything.
    #[serde(default = "yes")]
    dry_run: bool,
}

fn yes() -> bool {
    true
}

async fn prune_storage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PruneQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    as_response((|| {
        let settings = summo_core::Settings::load(&paths.settings())?;
        let now = time::OffsetDateTime::now_local()
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        summo_vault::storage::prune(
            paths,
            settings.storage.audio_retention_days,
            &now.date().to_string(),
            q.dry_run,
        )
    })())
}

async fn forget_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(
        summo_vault::storage::forget_audio(state.engine.paths(), &summo_core::MeetingId::from(id))
            .map(|freed| serde_json::json!({ "freed_bytes": freed })),
    )
}

#[derive(Debug, Deserialize)]
struct ReportQuery {
    #[serde(default)]
    token: Option<String>,
    /// Inclusive `YYYY-MM-DD` start. Defaults to `to`, making a single-day report.
    #[serde(default)]
    from: Option<String>,
    /// Inclusive `YYYY-MM-DD` end. Defaults to today in the daemon's local offset.
    #[serde(default)]
    to: Option<String>,
}

/// What a day, or a range of days, contained.
///
/// No model runs: this is arithmetic over the vault, so it is instant, works offline, and cannot be
/// wrong the way a generated summary can.
async fn report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ReportQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    // The daemon's own day. A report asked for "today" means the user's today.
    let today = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
        .to_string();
    let to = q.to.unwrap_or(today);
    let from = q.from.unwrap_or_else(|| to.clone());
    as_response(summo_vault::report::between(
        &state.engine.paths().vault(),
        &from,
        &to,
    ))
}

/// Stream one lane of a meeting's recording, honouring `Range` so the player can seek.
async fn meeting_audio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, lane)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }

    let meeting = summo_core::MeetingId::from(id);
    let path = match crate::audio_stream::locate(state.engine.paths(), &meeting, &lane) {
        Ok(path) => path,
        Err(e) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    };
    let total = match std::fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(e) => {
            let message = format!("cannot read {}: {e}", path.display());
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": message }))).into_response();
        }
    };

    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let span = match crate::audio_stream::parse_range(range, total) {
        Ok(span) => span,
        // A well-formed range that cannot be satisfied is a 416, and the header telling the client
        // how long the file actually is is the part that lets it recover.
        Err(_) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            )
                .into_response();
        }
    };

    // `Accept-Ranges` is what tells the player it may seek at all.
    let common = [
        (header::CONTENT_TYPE, "audio/ogg".to_string()),
        (header::ACCEPT_RANGES, "bytes".to_string()),
        // The vault is the user's own machine; caching a recording that never changes is free.
        (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
    ];

    match span {
        Some(span) => match crate::audio_stream::read_span(&path, span) {
            Ok(bytes) => (
                StatusCode::PARTIAL_CONTENT,
                common,
                [(header::CONTENT_RANGE, span.content_range())],
                bytes,
            )
                .into_response(),
            Err(e) => vault_error_response(&e),
        },
        None => match std::fs::read(&path) {
            Ok(bytes) => (StatusCode::OK, common, bytes).into_response(),
            Err(e) => vault_error_response(&Error::io(&path, e)),
        },
    }
}

fn vault_error_response(e: &Error) -> Response {
    let (status, message) = vault_error(e);
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Every task in the vault, grouped into columns.
async fn tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::board::read(state.engine.paths()))
}

#[derive(Debug, Deserialize)]
struct TaskUpdateBody {
    #[serde(default)]
    status: Option<summo_vault::tasks::Status>,
    /// `Some(None)` clears the owner; absent leaves it alone. Serde gives us both from
    /// `Option<Option<T>>` only when the field is explicitly `null`, which is what we want.
    #[serde(default, deserialize_with = "double_option")]
    owner: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    due: Option<Option<String>>,
}

/// Distinguish "field absent" from "field set to null".
fn double_option<'de, D, T>(deserializer: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

async fn update_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<TaskUpdateBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::board::update(
        state.engine.paths(),
        &id,
        body.status,
        body.owner,
        body.due,
    ))
}

#[derive(Debug, Deserialize)]
struct TaskCreateBody {
    text: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    due: Option<String>,
}

async fn create_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<TaskCreateBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::board::create(
        state.engine.paths(),
        &summo_core::MeetingId::from(id),
        &body.text,
        body.owner.as_deref(),
        body.due.as_deref(),
    ))
}

/// The summary shapes installed, so the interface can offer a choice.
async fn templates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(
        summo_vault::template::Templates::load_or_seed(&state.engine.paths().templates())
            .map(|t| t.all().to_vec()),
    )
}

#[derive(Debug, Deserialize)]
struct SummarizeBody {
    /// Template id, or absent to let the meeting's tags and title choose.
    #[serde(default)]
    template: Option<String>,
}

/// Write, or rewrite, one meeting's summary.
///
/// Synchronous, unlike the automatic run on stop: the user asked and is waiting for the answer, so
/// a failure should reach them rather than a log file.
async fn summarize_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<SummarizeBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }

    let settings = match summo_core::settings::Settings::load(&state.engine.paths().settings()) {
        Ok(settings) => settings,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let provider = match summo_llm::Provider::resolve(
        &settings.llm.provider,
        settings.llm.model.as_deref(),
        api_key().as_deref(),
    ) {
        Ok(provider) => provider,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let client = match summo_llm::LlmClient::new(provider) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let result = crate::summarize::run(
        state.engine.paths(),
        &client,
        &summo_core::MeetingId::from(id),
        body.template.as_deref(),
    )
    .await
    .map(|done| serde_json::json!({ "template": done.template, "sections": done.sections }));
    as_response(result)
}

/// Everyone Summo can recognise.
async fn people(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::list(&state.book))
}

#[derive(Debug, Deserialize)]
struct NameBody {
    name: String,
}

async fn rename_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<NameBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::rename(&state.book, &id, &body.name))
}

#[derive(Debug, Deserialize)]
struct AvatarBody {
    /// Vault-relative path, or absent to clear the picture.
    #[serde(default)]
    avatar: Option<String>,
}

async fn set_person_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<AvatarBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::set_avatar(&state.book, &id, body.avatar))
}

#[derive(Debug, Deserialize)]
struct MergeBody {
    /// The profile to fold in. It disappears; `id` in the path survives.
    from: String,
}

async fn merge_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<MergeBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::merge(&state.book, &body.from, &id))
}

async fn forget_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(
        crate::people::forget(&state.book, &id)
            .map(|removed| serde_json::json!({ "removed": removed })),
    )
}

/// The voices in one meeting that nobody has named yet, with who they might be.
async fn unknown_voices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::unknowns(
        &state.book,
        &state.engine.paths().voices(),
        &summo_core::MeetingId::from(id),
    ))
}

/// Name a voice, and fix every meeting that guessed it wrong.
async fn name_voice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, label)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<NameBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    as_response(crate::people::name_voice(
        &state.book,
        &state.engine.paths().voices(),
        &summo_core::MeetingId::from(id),
        &label,
        &body.name,
    ))
}

/// The whole settings file.
///
/// Safe to hand to the interface as-is: by construction it holds no secrets. API keys live in the
/// environment or the OS keychain precisely so that this endpoint can exist without a redaction
/// step that someone would eventually forget to update.
async fn settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }
    let path = state.engine.paths().settings();
    as_response(summo_core::Settings::load(&path).map(|settings| {
        serde_json::json!({
            "settings": settings,
            // Whether a key is present, never the key. The screen needs to say "set" or "not set"
            // and nothing more.
            "api_key_present": api_key().is_some(),
        })
    }))
}

#[derive(Debug, Deserialize)]
struct LlmBody {
    provider: String,
    model: Option<String>,
    language: Option<String>,
    summarize_on_stop: Option<bool>,
}

/// The API key, from the environment.
///
/// Read at the moment it is used rather than held, so a key rotated in the shell that launched the
/// daemon does not require a restart to take effect on the next call.
fn api_key() -> Option<String> {
    std::env::var("SUMMO_API_KEY").ok().filter(|k| !k.trim().is_empty())
}

async fn set_llm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<LlmBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }

    let path = state.engine.paths().settings();
    as_response((|| {
        // Refuse a provider that cannot be resolved rather than writing it and failing later, when
        // the user has moved on and the error has nothing to do with what they are doing.
        summo_llm::Provider::resolve(&body.provider, body.model.as_deref(), Some("probe"))?;

        let mut settings = summo_core::Settings::load(&path)?;
        settings.llm.provider = body.provider.trim().to_string();
        settings.llm.model = body.model.filter(|m| !m.trim().is_empty());
        if let Some(language) = body.language.filter(|l| !l.trim().is_empty()) {
            settings.llm.language = language;
        }
        if let Some(on_stop) = body.summarize_on_stop {
            settings.llm.summarize_on_stop = on_stop;
        }
        settings.save(&path)?;
        Ok(settings)
    })())
}

async fn test_llm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<LlmBody>,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
        return rejection.into_response();
    }

    let provider =
        match summo_llm::Provider::resolve(&body.provider, body.model.as_deref(), api_key().as_deref()) {
            Ok(provider) => provider,
            Err(e) => return as_response(Err::<(), _>(e)),
        };
    let local = provider.is_local();
    let base_url = provider.base_url.clone();

    match summo_llm::LlmClient::new(provider) {
        Ok(client) => match client.health_check().await {
            Ok(detail) => Json(serde_json::json!({
                "ok": true,
                "base_url": base_url,
                // The screen says this out loud: it is the one setting that decides whether
                // transcript text leaves the machine.
                "local": local,
                "detail": detail,
            }))
            .into_response(),
            Err(e) => Json(serde_json::json!({
                "ok": false,
                "base_url": base_url,
                "local": local,
                "detail": e.to_string(),
            }))
            .into_response(),
        },
        Err(e) => as_response(Err::<(), _>(e)),
    }
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    if let Err(rejection) = authorize(
        &headers,
        q.token.as_deref(),
        &state.token,
        state.allow_loopback_origins,
    ) {
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

    // The pipeline and the file it writes into, created on `session_start`. Held here rather than
    // in shared state because neither is `Sync` and both belong to exactly one client.
    #[cfg(feature = "models")]
    let mut session: Option<ActiveSession> = None;

    while let Some(Ok(message)) = socket.recv().await {
        let reply = match message {
            #[cfg(feature = "models")]
            Message::Text(text) => {
                let (events, next) = handle_command_with_models(&text, &engine, session.take());
                session = next;
                events
            }
            #[cfg(not(feature = "models"))]
            Message::Text(text) => handle_command(&text, &engine),

            #[cfg(feature = "models")]
            Message::Binary(bytes) => handle_audio_with_models(&bytes, &engine, session.as_mut()),
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
        Command::ModelPull { id } => {
            // Downloading is a long, cancellable, resumable operation with its own progress
            // reporting, and it does not belong on a socket that is also carrying live audio.
            // The CLI owns it, so point there rather than accepting and doing nothing.
            vec![Event::Error {
                message: format!(
                    "the daemon does not download models. Run `summo pull {id}`, or \
                     `summo setup` to install what this machine needs."
                ),
                transient: false,
            }]
        }
        Command::ModelLoad { id } | Command::ModelSwap { id } => vec![Event::Error {
            message: format!(
                "cannot load `{id}`: this binary was built without recognition support. \
                 Rebuild with `--features models`."
            ),
            transient: false,
        }],
    }
}

/// A running recording: the pipeline, the file it is being written into, and when it started.
#[cfg(feature = "models")]
struct ActiveSession {
    runner: crate::runner::SessionRunner,
    recorder: crate::recorder::Recorder,
    archive: crate::archive::AudioArchive,
    started: std::time::Instant,
}

/// Command handling when recognition is compiled in: owns creating and tearing down the pipeline.
#[cfg(feature = "models")]
fn handle_command_with_models(
    text: &str,
    engine: &EngineState,
    session: Option<ActiveSession>,
) -> (Vec<Event>, Option<ActiveSession>) {
    let command: Command = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            return (
                vec![Event::Error {
                    message: format!("malformed command: {e}"),
                    transient: false,
                }],
                session,
            );
        }
    };

    match command {
        Command::SessionStart(spec) => {
            if let Err(e) = engine.begin(&spec) {
                return (vec![Event::error(&e)], session);
            }
            match start_session(&spec, engine) {
                Ok(active) => {
                    let path = active.recorder.path().display().to_string();
                    (
                        vec![Event::info(format!(
                            "session started with {} — writing to {path}",
                            spec.live_model
                        ))],
                        Some(active),
                    )
                }
                Err(e) => {
                    // Loading failed, so the engine must not be left believing it is recording, or
                    // the next attempt would be refused as already in progress.
                    engine.end();
                    (vec![Event::error(&e)], None)
                }
            }
        }
        Command::SessionStop => {
            let mut events = Vec::new();
            if let Some(mut active) = session {
                match active.runner.flush() {
                    Ok(flushed) => {
                        for event in &flushed {
                            active.recorder.apply(event);
                        }
                        events.extend(flushed);
                    }
                    Err(e) => events.push(Event::error(&e)),
                }

                let elapsed = active.started.elapsed().as_secs_f64();

                // Close the audio first: the transcript's save is the operation allowed to fail
                // loudly, and it should not run while encoder buffers are still unflushed.
                let archived = active.archive.finish();
                if !archived.is_empty() {
                    let bytes: u64 = archived.iter().map(|f| f.bytes).sum();
                    events.push(Event::info(format!(
                        "kept {} of audio in {} file{}",
                        summo_vault::storage::human_bytes(bytes),
                        archived.len(),
                        if archived.len() == 1 { "" } else { "s" }
                    )));
                }

                // Captured before `finish` consumes the recorder.
                let meeting = active.recorder.document().frontmatter.id.clone();
                match active.recorder.finish(elapsed) {
                    Ok(path) => {
                        events.push(Event::info(format!("saved to {}", path.display())));
                        // Detached: the transcript is already saved and correct, and a user who
                        // stops recording and closes the lid should still find a summary later.
                        crate::summarize::spawn(engine.paths().clone(), meeting);
                    }
                    // The transcript is still on screen, so say the save failed rather than
                    // pretending the meeting was filed away.
                    Err(e) => events.push(Event::error(&e)),
                }
            }
            engine.end();
            events.push(Event::info("session stopped"));
            (events, None)
        }
        other => {
            let events = handle_command(&serde_json::to_string(&other).unwrap_or_default(), engine);
            (events, session)
        }
    }
}

/// Load the models and open the file this session writes into.
#[cfg(feature = "models")]
fn start_session(
    spec: &crate::protocol::SessionSpec,
    engine: &EngineState,
) -> summo_core::Result<ActiveSession> {
    use time::OffsetDateTime;

    let runner = crate::runner::SessionRunner::new(spec, &engine.store(), engine.hardware())?;

    // Local time rather than UTC: a meeting belongs to the day it happened on where the user was.
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    );
    let title = format!("Họp {:02}:{:02}", now.hour(), now.minute());

    let mut models = vec![("live".to_string(), spec.live_model.clone())];
    if let Some(refine) = &spec.refine_model {
        models.push(("refine".to_string(), refine.clone()));
    }

    let id = summo_core::MeetingId::new();

    // Read the setting at the start of the meeting rather than per frame: changing it mid-recording
    // would leave a file with a hole in it, which is worse than either answer.
    let keep_audio = summo_core::Settings::load(&engine.paths().settings())
        .map(|s| s.storage.keep_audio)
        .unwrap_or(true);
    let archive = crate::archive::AudioArchive::new(engine.paths(), &id, keep_audio);

    let recorder =
        crate::recorder::Recorder::start(engine.paths(), id, &title, &date, models)?;

    Ok(ActiveSession {
        runner,
        recorder,
        archive,
        started: std::time::Instant::now(),
    })
}

/// Audio handling when recognition is compiled in.
#[cfg(feature = "models")]
fn handle_audio_with_models(
    bytes: &[u8],
    engine: &EngineState,
    session: Option<&mut ActiveSession>,
) -> Vec<Event> {
    let (lane, samples) = match decode_frame(bytes) {
        Ok(frame) => frame,
        Err(e) => return vec![Event::error(&e)],
    };
    engine.advance(summo_core::audio::samples_to_secs(samples.len()), 0);

    let Some(active) = session else {
        // Audio before `session_start`. Saying so beats discarding it silently, which looks
        // identical to a microphone that is not working.
        return vec![Event::Error {
            message: "audio received before the session was started".into(),
            transient: true,
        }];
    };

    // Archive before decoding. If recognition fails or the model is wrong, the audio is still on
    // disk and the meeting can be transcribed again; the reverse is not true.
    if let Err(e) = active.archive.write(lane, &samples) {
        tracing::error!(error = %e, "could not archive audio");
    }

    match active.runner.accept(lane, &samples) {
        Ok(events) => {
            let finals = events
                .iter()
                .filter(|e| matches!(e, Event::Final(_)))
                .count();
            engine.advance(0.0, finals as u64);

            for event in &events {
                active.recorder.apply(event);
            }
            // Flushes on its own interval, so a crash costs seconds rather than the meeting.
            if let Err(e) = active.recorder.maybe_save() {
                tracing::error!(error = %e, "autosave failed");
            }
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
                allow_loopback_origins: false,
            },
        )
        .await
        .unwrap();
        (tmp, server)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    /// Put one unnamed voice in a meeting, the way a real session would have.
    fn seed_voice(tmp: &tempfile::TempDir, meeting: &str, label: &str, embedding: [f32; 4]) {
        let voices = Paths::at(tmp.path()).voices();
        let id = summo_core::MeetingId::from(meeting.to_string());
        let path = summo_diar::VoiceLog::path_for(&voices, &id);
        let mut log = summo_diar::VoiceLog::load(&path)
            .unwrap()
            .unwrap_or_else(|| summo_diar::VoiceLog::new(id, "campplus-sv"));
        log.samples.push(summo_diar::VoiceSample {
            seq: log.samples.len() as u64,
            t0: 0.0,
            duration: 30.0,
            label: summo_core::SpeakerId::from(label.to_string()),
            person: None,
            confirmed: false,
            embedding: embedding.to_vec(),
        });
        log.save(&path).unwrap();
    }

    /// Put a fake recording on disk. The bytes are not real Opus; nothing here decodes them.
    fn seed_audio(tmp: &tempfile::TempDir, meeting: &str, lane: &str, len: usize) {
        let paths = Paths::at(tmp.path());
        let dir = paths.audio_for(&summo_core::MeetingId::from(meeting.to_string()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{lane}.opus")), vec![9u8; len]).unwrap();
    }

    #[tokio::test]
    async fn audio_is_served_whole_when_no_range_is_asked_for() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 5_000);

        let resp = client()
            .get(format!("http://{}/meetings/01A/audio/mic", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["accept-ranges"], "bytes");
        assert_eq!(resp.headers()["content-type"], "audio/ogg");
        assert_eq!(resp.bytes().await.unwrap().len(), 5_000);
    }

    /// Seeking is the whole reason this route exists rather than a plain file read.
    #[tokio::test]
    async fn a_range_request_returns_only_that_span() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 5_000);

        let resp = client()
            .get(format!("http://{}/meetings/01A/audio/mic", server.addr()))
            .bearer_auth(server.token().as_str())
            .header("range", "bytes=1000-1999")
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers()["content-range"], "bytes 1000-1999/5000");
        assert_eq!(resp.bytes().await.unwrap().len(), 1_000);
    }

    #[tokio::test]
    async fn an_unsatisfiable_range_says_how_long_the_file_is() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 5_000);

        let resp = client()
            .get(format!("http://{}/meetings/01A/audio/mic", server.addr()))
            .bearer_auth(server.token().as_str())
            .header("range", "bytes=9000-9999")
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(resp.headers()["content-range"], "bytes */5000");
    }

    #[tokio::test]
    async fn a_lane_that_was_never_recorded_is_a_404() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 100);

        let resp = client()
            .get(format!("http://{}/meetings/01A/audio/system", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// The lane name comes from a URL, so it must never reach a path join.
    #[tokio::test]
    async fn audio_refuses_a_traversal_attempt() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 100);
        std::fs::write(tmp.path().join("secret.opus"), b"do not serve me").unwrap();

        for lane in ["..%2F..%2Fsecret", "..", "mic%2F..%2F..%2Fsecret"] {
            let resp = client()
                .get(format!("http://{}/meetings/01A/audio/{lane}", server.addr()))
                .bearer_auth(server.token().as_str())
                .send()
                .await
                .unwrap();
            assert!(
                resp.status().is_client_error(),
                "lane {lane} returned {}",
                resp.status()
            );
            let body = resp.bytes().await.unwrap();
            assert!(
                !body.windows(15).any(|w| w == b"do not serve me"),
                "lane {lane} leaked a file outside the meeting"
            );
        }
    }

    #[tokio::test]
    async fn audio_needs_a_token() {
        let (tmp, server) = running().await;
        seed_audio(&tmp, "01A", "mic", 100);
        let resp = client()
            .get(format!("http://{}/meetings/01A/audio/mic", server.addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_board_groups_tasks_and_keeps_agent_work_apart() {
        let (tmp, server) = running().await;
        seed_with_body(
            &tmp,
            "01A",
            "2026-08-10T09:00:00+07:00",
            "Họp",
            concat!(
                "## Việc cần làm\n",
                "- [ ] @ngoc Chốt spec <!-- id:T1 status:doing -->\n",
                "- [ ] @binh Gọi khách <!-- id:T2 -->\n",
                "- [ ] @agent Tạo lịch <!-- id:T3 status:running -->\n",
                "  - [x] Quét ghi chú\n",
            ),
        );

        let body: serde_json::Value = client()
            .get(format!("http://{}/tasks", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body["doing"].as_array().unwrap().len(), 1);
        assert_eq!(body["todo"].as_array().unwrap().len(), 1);
        assert_eq!(body["agent"].as_array().unwrap().len(), 1);
        assert_eq!(body["agent"][0]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(body["owners"].as_array().unwrap().len(), 2, "the agent is not an owner");
    }

    #[tokio::test]
    async fn moving_a_task_persists_and_leaves_the_notes_alone() {
        let (tmp, server) = running().await;
        seed_with_body(
            &tmp,
            "01A",
            "2026-08-10T09:00:00+07:00",
            "Họp",
            "## Tóm tắt\nGiữ nguyên câu này.\n\n## Việc cần làm\n- [ ] @ngoc Chốt spec <!-- id:T1 -->\n",
        );

        let resp = client()
            .post(format!("http://{}/tasks/T1", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "status": "done" }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "got {}", resp.status());

        let board: serde_json::Value = client()
            .get(format!("http://{}/tasks", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(board["done"].as_array().unwrap().len(), 1);

        let path = Paths::at(tmp.path()).meetings().join("01A.md");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("Giữ nguyên câu này."), "the notes were rewritten: {body}");
    }

    #[tokio::test]
    async fn a_summary_bullet_can_become_a_task() {
        let (tmp, server) = running().await;
        seed_with_body(&tmp, "01A", "2026-08-10T09:00:00+07:00", "Họp", "## Tóm tắt\nX.\n");

        let created: serde_json::Value = client()
            .post(format!("http://{}/meetings/01A/tasks", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "text": "Gửi báo giá", "owner": "binh" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(created["text"], "Gửi báo giá");
        assert_eq!(created["owner"], "binh");

        let board: serde_json::Value = client()
            .get(format!("http://{}/tasks", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(board["todo"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_task_that_is_not_there_is_a_404() {
        let (_tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/tasks/NOPE", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "status": "done" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tasks_need_a_token() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/tasks", server.addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn templates_are_seeded_and_listed() {
        let (_tmp, server) = running().await;
        let body: serde_json::Value = client()
            .get(format!("http://{}/templates", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let ids: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"standard"), "got {ids:?}");
        assert!(ids.contains(&"standup"), "got {ids:?}");

        // Every template must describe at least one section, or it summarises nothing.
        for template in body.as_array().unwrap() {
            assert!(
                !template["sections"].as_array().unwrap().is_empty(),
                "template {} has no sections",
                template["id"]
            );
        }
    }

    #[tokio::test]
    async fn summarising_a_meeting_that_is_not_there_is_reported() {
        let (_tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/meetings/01NOPE/summarize", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        // Either no model is configured or the meeting is missing; both are the caller's problem
        // and both must come back as a 4xx with a message rather than a 500.
        assert!(resp.status().is_client_error(), "got {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["error"].is_string(), "expected an explanation, got {body}");
    }

    #[tokio::test]
    async fn templates_need_a_token() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/templates", server.addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn people_needs_a_token_like_everything_else() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/people", server.addr()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_fresh_vault_knows_nobody() {
        let (_tmp, server) = running().await;
        let body: serde_json::Value = client()
            .get(format!("http://{}/people", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["people"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn naming_a_voice_creates_a_person_and_stops_asking() {
        let (tmp, server) = running().await;
        seed_voice(&tmp, "01A", "S2", [0.0, 1.0, 0.0, 0.0]);
        let base = format!("http://{}", server.addr());

        let unknown: serde_json::Value = client()
            .get(format!("{base}/meetings/01A/voices"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(unknown.as_array().unwrap().len(), 1);
        assert_eq!(unknown[0]["label"], "S2");

        let named: serde_json::Value = client()
            .post(format!("{base}/meetings/01A/voices/S2"))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "name": "Bình" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(named["person"]["name"], "Bình");
        assert_eq!(named["relabelled_here"], 1);

        // The question is answered, so it is no longer asked.
        let after: serde_json::Value = client()
            .get(format!("{base}/meetings/01A/voices"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(after.as_array().unwrap().len(), 0);

        // And they are somebody the app can now list and rename.
        let renamed: serde_json::Value = client()
            .post(format!("{base}/people/binh/name"))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "name": "Bình Trần" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(renamed["name"], "Bình Trần");
    }

    #[tokio::test]
    async fn a_person_who_is_not_there_is_a_404_not_a_400() {
        let (_tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/people/nobody/name", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "name": "X" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn forgetting_somebody_says_whether_they_were_there() {
        let (tmp, server) = running().await;
        seed_voice(&tmp, "01B", "S2", [0.0, 1.0, 0.0, 0.0]);
        let base = format!("http://{}", server.addr());
        client()
            .post(format!("{base}/meetings/01B/voices/S2"))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "name": "Bình" }))
            .send()
            .await
            .unwrap();

        let first: serde_json::Value = client()
            .delete(format!("{base}/people/binh"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(first["removed"], true);

        let second: serde_json::Value = client()
            .delete(format!("{base}/people/binh"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(second["removed"], false);
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
        for path in ["hw", "models", "status", "library", "library/search"] {
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
    async fn development_mode_admits_a_page_from_this_machine() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = EngineState::new(Paths::at(tmp.path())).unwrap();
        let server = Server::start(
            engine,
            ServerConfig {
                port: 0,
                write_token_file: false,
                allow_loopback_origins: true,
            },
        )
        .await
        .unwrap();

        let resp = client()
            .get(format!("http://{}/hw", server.addr()))
            .bearer_auth(server.token().as_str())
            .header("Origin", "http://localhost:5173")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // Even then, a remote page is refused: a developer with the daemon running must not be
        // recordable by any site they happen to visit.
        let remote = client()
            .get(format!("http://{}/hw", server.addr()))
            .bearer_auth(server.token().as_str())
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(remote.status(), 403);

        server.shutdown();
    }

    /// Write a meeting into a running server's vault.
    /// Seed a meeting whose body the test controls, for the parts that care about its contents.
    fn seed_with_body(tmp: &tempfile::TempDir, id: &str, date: &str, title: &str, body: &str) {
        let paths = Paths::at(tmp.path());
        let dir = paths.meetings();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.md")),
            format!(
                "---\nid: {id}\ndate: {date}\nduration: 600\ntags: []\n---\n# {title}\n\n{body}"
            ),
        )
        .unwrap();
    }

    fn seed(tmp: &tempfile::TempDir, id: &str, date: &str, title: &str) {
        let paths = Paths::at(tmp.path());
        let dir = paths.meetings();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.md")),
            format!(
                "---\nid: {id}\ndate: {date}\nduration: 600\ntags: [weekly]\n---\n\
                 # {title}\n\n## Tóm tắt\nChốt dùng Rust.\n\n## Transcript\n\
                 **[00:12:04] Bạn** — Mình họp về ngân sách nhé\n"
            ),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn the_library_lists_what_is_in_the_vault() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");

        let body: serde_json::Value = client()
            .get(format!("http://{}/library", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body["total"], 1);
        assert_eq!(body["groups"][0]["key"], "2026-08-09");
        assert_eq!(body["groups"][0]["meetings"][0]["title"], "Weekly Sync");
        assert_eq!(body["stats"]["meetings"], 1);
        server.shutdown();
    }

    #[tokio::test]
    async fn search_reaches_the_transcript_without_tone_marks() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");

        let body: serde_json::Value = client()
            .get(format!("http://{}/library/search?q=ngan+sach", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body[0]["meeting"]["id"], "01A");
        assert_eq!(body[0]["excerpts"][0]["t0"], 724.0);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_missing_meeting_is_a_404_not_a_500() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/meetings/nope", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_meeting_can_be_filed_renamed_and_trashed_over_http() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");
        let base = format!("http://{}/meetings/01A", server.addr());
        let token = server.token().as_str().to_string();

        let resp = client()
            .post(format!("{base}/folder"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "folder": "khach-hang" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let resp = client()
            .post(format!("{base}/title"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "title": "Họp sản phẩm" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let detail: serde_json::Value = client()
            .get(&base)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(detail["summary"]["title"], "Họp sản phẩm");
        assert_eq!(detail["summary"]["folder"], "khach-hang");
        assert_eq!(detail["transcript"].as_array().unwrap().len(), 1);

        let resp = client()
            .post(format!("{base}/trash"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = client()
            .get(format!("http://{}/library", server.addr()))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["total"], 0, "a trashed meeting must leave the library");
        server.shutdown();
    }

    #[tokio::test]
    async fn a_folder_that_escapes_the_vault_is_refused() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");

        let resp = client()
            .post(format!("http://{}/meetings/01A/folder", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "folder": "../../etc" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);

        // And it is still listed where it was.
        let body: serde_json::Value = client()
            .get(format!("http://{}/library", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["total"], 1);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_shipped_build_promises_nothing_to_a_web_page() {
        // No CORS headers, so even if a page somehow held the token the browser would block the
        // reply. This is the layer that must never quietly become permissive.
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/library", server.addr()))
            .header("Origin", "http://localhost:5199")
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        assert!(resp.headers().get("access-control-allow-origin").is_none());
        server.shutdown();
    }

    #[tokio::test]
    async fn development_mode_tells_the_browser_what_it_already_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = EngineState::new(Paths::at(tmp.path())).unwrap();
        let server = Server::start(
            engine,
            ServerConfig {
                port: 0,
                write_token_file: false,
                allow_loopback_origins: true,
            },
        )
        .await
        .unwrap();

        let resp = client()
            .get(format!("http://{}/library", server.addr()))
            .header("Origin", "http://localhost:5199")
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("access-control-allow-origin").unwrap(),
            "http://localhost:5199"
        );

        // A POST with a JSON body is preflighted, and the preflight carries no token.
        let resp = client()
            .request(
                reqwest::Method::OPTIONS,
                format!("http://{}/meetings/01A/title", server.addr()),
            )
            .header("Origin", "http://localhost:5199")
            .header("Access-Control-Request-Method", "POST")
            .header("Access-Control-Request-Headers", "content-type")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert!(
            resp.headers()
                .get("access-control-allow-headers")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("content-type")
        );

        // A remote origin is still refused, dev mode or not.
        let resp = client()
            .get(format!("http://{}/library", server.addr()))
            .header("Origin", "https://evil.example")
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        assert!(resp.headers().get("access-control-allow-origin").is_none());
        server.shutdown();
    }

    #[tokio::test]
    async fn settings_are_readable_and_carry_no_secret() {
        let (_tmp, server) = running().await;
        let body: serde_json::Value = client()
            .get(format!("http://{}/settings", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body["settings"]["llm"]["provider"], "ollama");
        // The key is reported as present or absent, never returned. If this ever starts serialising
        // a key, the settings file has grown a field it must not have.
        assert!(body["settings"]["llm"].get("api_key").is_none());
        assert!(body["api_key_present"].is_boolean());
        server.shutdown();
    }

    #[tokio::test]
    async fn the_llm_provider_can_be_changed_and_is_persisted() {
        let (tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/settings/llm", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({
                "provider": "http://127.0.0.1:1234/v1",
                "model": "qwen3-8b",
                "language": "Vietnamese",
                "summarize_on_stop": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let saved =
            summo_core::Settings::load(&Paths::at(tmp.path()).settings()).unwrap();
        assert_eq!(saved.llm.provider, "http://127.0.0.1:1234/v1");
        assert_eq!(saved.llm.model.as_deref(), Some("qwen3-8b"));
        assert!(saved.llm.summarize_on_stop);
        server.shutdown();
    }

    #[tokio::test]
    async fn an_unusable_provider_is_refused_before_it_is_written() {
        let (tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/settings/llm", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "provider": "wishful-thinking" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);

        let saved =
            summo_core::Settings::load(&Paths::at(tmp.path()).settings()).unwrap();
        assert_eq!(saved.llm.provider, "ollama", "a bad provider must not be written");
        server.shutdown();
    }

    #[tokio::test]
    async fn testing_a_provider_that_is_not_running_reports_it_rather_than_failing() {
        // Port 1 has nothing on it. A user pointing at a dead endpoint should see "cannot reach
        // it", not a 500 that looks like the app is broken.
        let (_tmp, server) = running().await;
        let body: serde_json::Value = client()
            .post(format!("http://{}/settings/llm/test", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "provider": "http://127.0.0.1:1/v1", "model": "x" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body["ok"], false);
        assert_eq!(body["local"], true, "a loopback endpoint is local");
        assert!(body["detail"].as_str().is_some_and(|d| !d.is_empty()));
        server.shutdown();
    }

    #[tokio::test]
    async fn storage_is_reported_and_pruning_defaults_to_a_dry_run() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2020-01-01T10:00:00+07:00", "Rất cũ");
        let audio = Paths::at(tmp.path()).audio_for(&summo_core::MeetingId::from("01A".to_string()));
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(audio.join("mic.opus"), vec![0u8; 2_048]).unwrap();

        let body: serde_json::Value = client()
            .get(format!("http://{}/storage", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["audio_bytes"], 2_048);
        assert_eq!(body["recordings"][0]["title"], "Rất cũ");

        // No `dry_run` parameter at all must not delete: a mistyped request is not consent.
        let body: serde_json::Value = client()
            .post(format!("http://{}/storage/prune", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["freed_bytes"], 2_048);
        assert!(audio.exists(), "a default prune deleted audio");

        let resp = client()
            .post(format!("http://{}/storage/prune?dry_run=false", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert!(!audio.exists());
        assert!(
            Paths::at(tmp.path()).meetings().join("01A.md").exists(),
            "pruning deleted a transcript"
        );
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
    fn a_download_request_points_at_the_command_that_performs_it() {
        // Accepting and doing nothing would look like a hung download.
        let (_tmp, engine) = engine();
        let events = handle_command(r#"{"cmd":"model_pull","id":"x"}"#, &engine);
        let Event::Error { message, .. } = &events[0] else {
            panic!("expected an error")
        };
        assert!(
            message.contains("summo pull x"),
            "the message should name the command that does work: {message}"
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
