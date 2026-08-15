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
    auth::{SessionToken, token_path},
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
    /// and driven in a browser test; see [`crate::auth::origin_is_allowed_from`] for why it is opt-in.
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
    /// The bound port, filled in after the listener exists.
    ///
    ///
    /// The router is built before the socket is bound — it has to be, since `axum::serve` takes
    /// both — and `port: 0` means the OS chooses. So the one handler that needs to *tell* the page
    /// which port it is on reads it from here rather than from a value that would have been zero.
    port: std::sync::Arc<std::sync::atomic::AtomicU16>,
    /// Notified when something asks the daemon to stop.
    ///
    /// A background daemon has no terminal to press Ctrl-C in, so `summo stop` has to reach it the
    /// only way anything reaches it: over HTTP, with the token. The process still decides to exit
    /// itself rather than being signalled, which is what lets it finish writing a recording first.
    stopping: std::sync::Arc<tokio::sync::Notify>,
}

impl AppState {
    /// The port this daemon actually bound. Zero until the listener exists.
    fn own_port(&self) -> u16 {
        self.port.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Refuse a request that has no business being served.
    ///
    /// One method rather than the same five arguments at sixty-odd call sites — which is how the
    /// port came to be missing from the check in the first place, and why the bundled interface
    /// could read and never write.
    fn guard(
        &self,
        headers: &HeaderMap,
        query_token: Option<&str>,
    ) -> std::result::Result<(), (StatusCode, &'static str)> {
        authorize_from(
            headers,
            query_token,
            &self.token,
            self.allow_loopback_origins,
            Some(self.own_port()),
        )
    }
}

/// A running daemon.
pub struct Server {
    addr: SocketAddr,
    token: SessionToken,
    handle: tokio::task::JoinHandle<()>,
    stopping: std::sync::Arc<tokio::sync::Notify>,
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
            port: std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0)),
            stopping: std::sync::Arc::new(tokio::sync::Notify::new()),
        };
        let port_slot = state.port.clone();
        let stopping = state.stopping.clone();

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
            .route("/catalogue", get(catalogue))
            .route("/models/{id}", axum::routing::delete(remove_model))
            .route("/settings/models", post(set_models))
            .route("/agent/run", post(run_errand))
            .route("/agent/habits", get(habits))
            .route("/agent/dream", get(dream_state).post(dream_now))
            .route("/status", get(status))
            .route("/shutdown", post(shutdown))
            .route("/storage", get(storage))
            .route("/storage/prune", post(prune_storage))
            .route("/meetings/{id}/audio", axum::routing::delete(forget_audio))
            .route("/settings", get(settings))
            .route("/settings/llm", post(set_llm))
            .route("/settings/llm/providers", get(llm_providers))
            .route("/settings/llm/test", post(test_llm))
            .route("/report", get(report))
            .route("/agents", get(agents))
            .route("/agents/{slug}", get(agent).post(set_agent))
            .route("/tasks", get(tasks))
            .route("/nudges", get(nudges))
            .route("/ask", post(ask))
            .route("/imports", get(list_imports).post(start_import))
            .route("/imports/clear", post(clear_imports))
            .route("/imports/{id}", get(get_import))
            .route("/meetings/{id}/draft", get(get_draft))
            .route("/meetings/{id}/draft/generate", post(generate_draft))
            .route("/meetings/{id}/draft/refine", post(refine_draft))
            .route("/meetings/{id}/draft/chat", post(chat_draft))
            .route("/meetings/{id}/draft/confirm", post(confirm_draft))
            .route("/meetings/{id}/draft", axum::routing::delete(discard_draft))
            .route("/tasks/{id}", post(update_task))
            .route("/tasks/{id}/run", post(run_task))
            .route("/meetings/{id}/tasks", post(create_task))
            .route("/templates", get(templates))
            .route("/locales", get(locales))
            .route("/notes", get(list_notes).post(create_note))
            .route("/notes/{id}", get(read_note).post(update_note))
            .route("/notes/{id}", axum::routing::delete(delete_note))
            .route("/meetings/{id}/comments", get(list_comments).post(add_comment))
            .route(
                "/meetings/{id}/comments/{comment}",
                axum::routing::delete(delete_comment),
            )
            .route("/meetings/{id}/comments/{comment}/react", post(react_comment))
            .route("/agenda", get(agenda))
            .route("/agenda/suggest", get(suggest_meeting))
            .route("/calendars", get(list_calendars).post(add_calendar))
            .route("/calendars/subscribe", post(subscribe_calendar))
            .route("/calendars/refresh", post(refresh_calendars))
            .route("/calendars/{name}", axum::routing::delete(remove_calendar))
            .route("/onboarding", get(onboarding))
            .route("/onboarding/complete", post(complete_onboarding))
            .route("/onboarding/recommend", get(recommend_models))
            .route("/languages", get(languages))
            .route("/settings/language", post(set_language))
            .route("/installs", get(list_installs).post(start_install))
            .route("/installs/{id}", get(get_install))
            .route("/meetings/{id}/summarize", post(summarize_meeting))
            .route("/meetings/{id}/translate", post(translate_meeting))
            .route("/meetings/{id}/translations", get(meeting_translations))
            .route(
                "/meetings/{id}/translations/{lang}",
                get(meeting_translation),
            )
            .route("/meetings/{id}/subtitles", get(meeting_subtitles))
            .route("/people", get(people))
            .route("/people/{id}/name", post(rename_person))
            .route("/people/{id}/avatar", post(set_person_avatar))
            .route("/people/{id}/merge", post(merge_person))
            .route("/people/{id}", axum::routing::delete(forget_person))
            .route("/meetings/{id}/audio/{lane}", get(meeting_audio))
            .route("/voices/unknown", get(unknown_voices_everywhere))
            .route("/meetings/{id}/voices", get(unknown_voices))
            .route("/meetings/{id}/voices/{label}", post(name_voice))
            .route("/library", get(library))
            .route("/library/search", get(search))
            .route("/meetings/{id}", get(meeting))
            .route("/meetings/{id}/folder", post(set_folder))
            .route("/meetings/{id}/tags", post(set_tags))
            .route("/meetings/{id}/colour", post(set_colour))
            .route("/meetings/{id}/title", post(set_title))
            .route("/meetings/{id}/trash", post(trash))
            .route("/mcp", post(mcp))
            .route("/ws", get(websocket))
            // Last: a fallback claims every path no route above matched, which is how a
            // single-page app's own routes reach it. Registering it earlier would shadow the API.
            .fallback(interface)
            .layer(middleware::from_fn_with_state(state.clone(), cors))
            .with_state(state.clone());

        // Added after the chain rather than inside it: the handler exists only in a build with
        // recognition, and a `#[cfg]` on one line of a long builder chain is a line every future
        // edit has to remember. A daemon without models simply does not have this path.
        #[cfg(feature = "models")]
        let app = app
            .route("/models/warm", post(warm_model))
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

        port_slot.store(addr.port(), std::sync::atomic::Ordering::Relaxed);
        tracing::info!(%addr, bundled = crate::assets::bundled(), "engine listening");
        // Calendars, kept current while the daemon runs.
        //
        // A subscription that only refreshed when somebody opened the agenda would be useless for
        // the one thing it is for: telling the user a meeting is starting before it starts. Fetched
        // on startup and then on a timer, and only when there is something to fetch — a user with
        // no subscriptions makes no requests to anybody.
        {
            let paths = engine.paths().clone();
            tokio::spawn(async move {
                loop {
                    match crate::calsync::list(&paths) {
                        Ok(subscriptions) if !subscriptions.is_empty() => {
                            if let Err(e) = crate::calsync::refresh(&paths, None).await {
                                tracing::warn!(error = %e, "calendar sync failed");
                            }
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "cannot read calendar subscriptions"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(crate::calsync::REFRESH_S))
                        .await;
                }
            });
        }

        // A night's sleep, if the user asked for one.
        //
        // Checked on a timer rather than scheduled for the hour: a laptop is asleep at three in the
        // morning, and a cron-shaped design would mean the feature works only for people who leave
        // a desktop running. This runs on the first check after the hour, which for most machines
        // is the moment the lid opens.
        {
            let engine = engine.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
                    let paths = engine.paths();
                    let settings =
                        summo_core::Settings::load(&paths.settings()).unwrap_or_default();
                    if !settings.agents.dream {
                        continue;
                    }
                    let now = time::OffsetDateTime::now_local()
                        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
                    let today = now.date().to_string();
                    if !crate::dream::due(
                        paths,
                        &today,
                        now.hour(),
                        settings.agents.dream_hour,
                        engine.status().is_recording(),
                    ) {
                        continue;
                    }
                    let Ok(client) = llm_for_engine(&engine) else {
                        // No model configured is not an error to report at three in the morning.
                        continue;
                    };
                    match crate::dream::run(paths, &client, None, &today).await {
                        Ok(dreamt) => {
                            tracing::info!(?dreamt, "agents slept on it");
                            crate::dream::mark(paths, &today, &dreamt);
                        }
                        Err(e) => tracing::warn!(error = %e, "a night's consolidation failed"),
                    }
                }
            });
        }

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "server stopped");
            }
        });

        Ok(Self {
            addr,
            token,
            handle,
            stopping,
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

    /// Resolves when something asked the daemon to stop over HTTP.
    ///
    /// Awaited beside Ctrl-C, so a daemon started in the background and one started in a terminal
    /// stop the same way and run the same cleanup.
    pub async fn stop_requested(&self) {
        self.stopping.notified().await;
    }
}

/// Tell the browser what the daemon already decided.
///
/// [`crate::auth::origin_is_allowed_from`] governs whether a request is served at all; this reports that same
/// decision back as CORS headers, because a browser blocks a cross-origin reply it was not
/// explicitly promised — the daemon would answer 200 and the app would still see a network error.
/// The two must never drift, so this asks the same function rather than keeping its own list.
///
/// In a shipped build the only origin allowed is this daemon's own, which is the page it served.
async fn cors(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let allowed = origin.as_deref().is_some_and(|o| {
        crate::auth::origin_is_allowed_from(
            Some(o),
            state.allow_loopback_origins,
            Some(state.own_port()),
        )
    });

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
/// The same check, told which port this daemon is on so it can recognise its own page.
fn authorize_from(
    headers: &HeaderMap,
    query_token: Option<&str>,
    expected: &SessionToken,
    allow_loopback_origins: bool,
    own_port: Option<u16>,
) -> std::result::Result<(), (StatusCode, &'static str)> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !crate::auth::origin_is_allowed_from(origin, allow_loopback_origins, own_port) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    Json(state.engine.hardware().clone()).into_response()
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    #[cfg(feature = "models")]
    {
        // `ready` is added *beside* the status fields, not wrapped around them. Every client reads
        // `state` and `live_model` at the top level, and moving them under a key would have been a
        // breaking change to save one line here.
        let mut body = serde_json::to_value(state.engine.status()).unwrap_or_default();
        if let Some(object) = body.as_object_mut() {
            // What is loaded and instant right now. `null` means the next recording pays about
            // three and a half seconds to build a decoder — worth saying rather than leaving as an
            // unexplained pause after pressing record.
            object.insert(
                "ready".into(),
                match state.engine.warm().ready() {
                    Some(key) => {
                        serde_json::json!({ "model": key.model, "language": key.language })
                    }
                    None => serde_json::Value::Null,
                },
            );
        }
        return Json(body).into_response();
    }
    #[cfg(not(feature = "models"))]
    Json(state.engine.status()).into_response()
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    Json(state.engine.store().list()).into_response()
}

/// Everything the registry offers, with what is already here marked.
///
/// Distinct from `/models`, which lists what is installed. This is the shop window: a user deciding
/// whether to spend 600 MB on a better translator needs to see the size, the licence, the languages
/// and what it is *for* before they commit, and none of that is on the machine yet.
///
/// The registry is remote and may be unreachable — on a plane, behind a proxy, or simply not
/// deployed yet. That is not an error here: the installed models are still returned, with
/// `reachable: false`, so the screen shows what the user has rather than a failure. A model
/// manager that goes blank without a network is worse than one that admits it is offline.
async fn catalogue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RecommendQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let installed: std::collections::HashMap<String, u64> = state
        .engine
        .store()
        .list()
        .into_iter()
        .map(|m| (m.id.to_string(), m.size_bytes))
        .collect();

    let mut reachable = true;
    let manifests = match candidates(&state, q.registry.as_deref()).await {
        Ok(manifests) => manifests,
        Err(e) => {
            tracing::warn!(error = %e, "registry unavailable; showing installed models only");
            reachable = false;
            state.engine.store().list()
        }
    };
    // `candidates` swallows a registry failure and returns the installed set, so "did the registry
    // answer" cannot be read from the result alone. Anything the user has not installed came from
    // the registry, which means it did.
    if reachable
        && manifests
            .iter()
            .all(|m| installed.contains_key(&m.id.to_string()))
    {
        reachable = !installed.is_empty() && manifests.len() > installed.len();
    }

    let hardware = state.engine.hardware();
    let models: Vec<_> = manifests
        .iter()
        .map(|m| {
            let id = m.id.to_string();
            serde_json::json!({
                "id": id,
                "name": m.name,
                "task": m.task,
                "mode": m.mode,
                "langs": m.langs,
                "license": m.license,
                "attribution": m.attribution,
                "redistributable": m.redistributable,
                "gated": m.gated,
                "description": m.description,
                "size_bytes": m.size_bytes,
                "installed": installed.contains_key(&id),
                // Whether this machine can run it at all, so a phone is not offered a model that
                // will be refused at load with an out-of-memory error.
                "fits": m.profile.min_ram_mb == 0
                    || m.profile.min_ram_mb <= hardware.total_ram_mb,
                "min_ram_mb": m.profile.min_ram_mb,
            })
        })
        .collect();

    // Which model each role points at. Installed and *chosen* are different states, and a screen
    // that cannot tell them apart is one where installing a Japanese model appears to do nothing.
    let settings = summo_core::Settings::load(&state.engine.paths().settings()).unwrap_or_default();
    let chosen = serde_json::json!({
        "live": settings.models.live,
        "refine": settings.models.refine,
        "vad": settings.models.vad,
        "speaker": settings.models.speaker,
        "translator": settings
            .llm
            .translator
            .as_ref()
            .filter(|mt| mt.is_local())
            .and_then(|mt| mt.model.clone()),
    });

    Json(serde_json::json!({
        "models": models,
        "reachable": reachable,
        "chosen": chosen,
    }))
    .into_response()
}

/// What this person keeps asking for.
///
/// Not a recommendation engine: it is the list of instructions they have typed more than once,
/// counted, so the interface can offer the words back instead of making them type them again. See
/// [`summo_agent::habits`] — the file is Markdown in the vault and deleting a line forgets it.
async fn habits(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let asks = summo_agent::habits::load(&state.engine.paths().agents());
    as_response(Ok(summo_agent::habits::habits(&asks)))
}

/// Whether the agents sleep on it, and what the last night did.
async fn dream_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    let settings = summo_core::Settings::load(&paths.settings()).unwrap_or_default();
    as_response(Ok(serde_json::json!({
        "dream": settings.agents.dream,
        "hour": settings.agents.dream_hour,
        "last": crate::dream::last(paths),
    })))
}

#[derive(Debug, Default, Deserialize)]
struct DreamBody {
    /// One agent, or all of them.
    #[serde(default)]
    agent: Option<String>,
    /// Turn the nightly pass on or off. Absent leaves the setting alone, so "do it now" and
    /// "do it every night" are separate decisions.
    #[serde(default)]
    dream: Option<bool>,
    #[serde(default)]
    hour: Option<u8>,
    /// Run one now. Off by default: this endpoint is also how the switch is set.
    #[serde(default)]
    now: bool,
}

/// Change the setting, run a night by hand, or both.
async fn dream_now(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Option<Json<DreamBody>>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let paths = state.engine.paths();

    if body.dream.is_some() || body.hour.is_some() {
        let path = paths.settings();
        let result = summo_core::Settings::load(&path).and_then(|mut settings| {
            if let Some(dream) = body.dream {
                settings.agents.dream = dream;
            }
            if let Some(hour) = body.hour {
                settings.agents.dream_hour = hour.min(23);
            }
            settings.save(&path)
        });
        if let Err(e) = result {
            return as_response(Err::<serde_json::Value, _>(e));
        }
    }

    if !body.now {
        return as_response(Ok(serde_json::json!({ "dreamt": [] })));
    }
    // Never during a meeting: it is a language-model call and a rewrite of a file the live pipeline
    // reads, at the one moment the machine is busiest.
    if state.engine.status().is_recording() {
        return as_response(Ok(
            serde_json::json!({ "dreamt": [], "skipped": "đang ghi âm" }),
        ));
    }
    let client = match llm_client(&state) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let done = crate::dream::run(paths, &client, body.agent.as_deref(), &summo_core::today()).await;
    as_response(done.map(|dreamt| {
        crate::dream::mark(paths, &summo_core::today(), &dreamt);
        serde_json::json!({ "dreamt": dreamt })
    }))
}

/// Hand an agent a sentence.
///
/// The instruction becomes a `- [ ] @agent …` checkbox in the day's scratch note and runs through
/// the same path a checkbox typed by hand does — see [`crate::errand`]. That is deliberate: the
/// agent writes its steps into the file as it works, so an errand started from a text box leaves
/// the same readable trace in Markdown as one started from a note.
///
/// Synchronous. An agent run is seconds, the panel shows a spinner for them, and streaming the
/// steps would mean a second delivery mechanism for something the vault is already recording.
async fn run_errand(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ErrandBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        crate::errand::run(
            state.engine.paths(),
            &body.instruction,
            body.agent.as_deref(),
            body.meeting.as_deref(),
        )
        .await
        .map(crate::errand::Errand::from),
    )
}

#[derive(Debug, Deserialize)]
struct ErrandBody {
    instruction: String,
    /// A slug from `vault/agents/`. Absent uses the coordinator, which is what the roster is for.
    #[serde(default)]
    agent: Option<String>,
    /// The note this was asked from, so a habit knows what it is usually asked *about*.
    #[serde(default)]
    meeting: Option<String>,
}

/// Choose which installed model a role uses.
///
/// The missing half of the catalogue: installing a model and then having no way to say "use this
/// one" made the whole screen decorative. Roles are named rather than inferred from the task,
/// because `asr` fills two of them — the live model and the slower one that re-decodes after it —
/// and which is wanted is the user's decision, not a property of the model.
async fn set_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ModelsBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    as_response((|| {
        let path = state.engine.paths().settings();
        let mut settings = summo_core::Settings::load(&path)?;
        let id = body.model.trim();

        // Only something that is here. A setting naming a model that was never installed fails at
        // the start of a recording, which is the worst moment to find out.
        let model_id = summo_core::ModelId::parse(id).map_err(Error::Config)?;
        let manifest = state.engine.store().installed(&model_id)?;

        let expected = match body.role.as_str() {
            "live" | "refine" => summo_models::Task::Asr,
            "vad" => summo_models::Task::Vad,
            "speaker" => summo_models::Task::SpeakerEmbed,
            "translator" => summo_models::Task::Translate,
            other => return Err(Error::Config(format!("no such model role: `{other}`"))),
        };
        if manifest.task != expected {
            return Err(Error::Config(format!(
                "`{id}` cannot be the {} model",
                body.role
            )));
        }

        match body.role.as_str() {
            "live" => {
                settings.models.live = Some(id.to_string());
                // A refinement model identical to the live one decodes everything twice for
                // nothing, and `SessionSpec::validate` refuses it — so choosing one as live clears
                // the other rather than leaving a session that cannot start.
                if settings.models.refine.as_deref() == Some(id) {
                    settings.models.refine = None;
                }
            }
            "refine" => settings.models.refine = Some(id.to_string()),
            "vad" => settings.models.vad = Some(id.to_string()),
            "speaker" => settings.models.speaker = Some(id.to_string()),
            "translator" => {
                settings.llm.translator = Some(summo_core::settings::Translator {
                    provider: summo_core::settings::LOCAL.to_string(),
                    model: Some(id.to_string()),
                });
            }
            _ => unreachable!("the role was checked above"),
        }

        settings.save(&path)?;
        Ok(settings)
    })())
}

#[derive(Deserialize)]
struct ModelsBody {
    /// `live`, `refine`, `vad`, `speaker` or `translator`.
    role: String,
    model: String,
}

#[derive(Deserialize)]
struct LanguageBody {
    /// ISO code, or empty for "let the model detect it".
    #[serde(default)]
    language: String,
}

/// Fill in the models a session did not name.
#[cfg(feature = "models")]
///
/// The interface used to send a hardcoded `gipformer-65m`, which made the whole model catalogue
/// decorative: installing SenseVoice for a Japanese meeting changed nothing, because recording
/// still reached for the Vietnamese transducer — and on a machine without it, recording failed with
/// a missing-model error naming a model the user never chose.
///
/// Order: what the session asked for, then what the settings say, then the only installed speech
/// model if there is exactly one. The last is the case that matters on a fresh install — one model
/// is there, it is obviously the one to use, and asking would be asking a question with one answer.
fn resolve_models(
    spec: &crate::protocol::SessionSpec,
    engine: &EngineState,
) -> crate::protocol::SessionSpec {
    let mut spec = spec.clone();
    if !spec.live_model.trim().is_empty() {
        return spec;
    }

    let settings = summo_core::Settings::load(&engine.paths().settings()).unwrap_or_default();
    if spec.language.is_none() {
        spec.language = settings.models.language.clone();
    }

    if let Some(chosen) = settings.models.live.filter(|m| !m.trim().is_empty()) {
        spec.live_model = chosen;
        return spec;
    }

    let speech: Vec<_> = engine
        .store()
        .list()
        .into_iter()
        .filter(|m| m.task == summo_models::Task::Asr)
        .collect();

    // One model is not a choice.
    if let [only] = speech.as_slice() {
        spec.live_model = only.id.to_string();
        return spec;
    }

    // More than one, and nothing in the settings says which. This used to give up — and giving up
    // means `session needs a live model`, so installing a *second* speech model broke recording
    // until the user went and picked one. The language is the thing that decides, and by now the
    // app knows it: rank the installed models for it and take the best, which is the same answer
    // the model picker would give.
    //
    // With no language either, the multilingual models are the honest default: a model that only
    // speaks Vietnamese is the wrong guess for a meeting nobody has described.
    let language = spec.language.clone().unwrap_or_else(|| "*".into());
    let ranked = summo_models::recommend(&speech, engine.hardware(), &language);
    if let Some(best) = ranked.best() {
        spec.live_model = best.id.clone();
    } else if let Some(first) = speech.first() {
        // Nothing covers the language. Recording in the wrong language beats refusing to record:
        // the transcript is visibly wrong and fixable, and the meeting is not repeatable.
        spec.live_model = first.id.to_string();
    }
    spec
}

/// Delete an installed model, reclaiming whatever nothing else references.
///
/// A model manager without this is half a manager: these are 73 MB to 2.5 GB each, and installing
/// the wrong one is the most likely mistake the catalogue screen invites. Until now the only way
/// back was `summo rm` on a command line.
///
/// Refuses to remove a model the settings currently point at. The alternative is a recording that
/// fails to start with a missing-file error, some time later, with nothing connecting the two —
/// and choosing a replacement first is one click on the screen this is called from.
async fn remove_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    as_response((|| {
        let model_id = summo_core::ModelId::parse(&id).map_err(Error::Config)?;
        let paths = state.engine.paths();

        if let Some(role) = in_use(
            &summo_core::settings::Settings::load(&paths.settings())?,
            &id,
        ) {
            return Err(Error::Config(format!(
                "`{id}` is in use as the {role} model; choose another one first"
            )));
        }

        // Before the blobs go: a warm decoder holding a removed model is a crash waiting for the
        // next recording.
        #[cfg(feature = "models")]
        state.engine.warm().clear();

        let freed = state.engine.store().remove(&model_id)?;
        Ok(serde_json::json!({ "removed": id, "freed_bytes": freed }))
    })())
}

/// Which setting names this model, if any.
///
/// Read from the settings file rather than from a running session: a model is "in use" if the next
/// recording would reach for it, not only if one is happening now.
fn in_use(settings: &summo_core::settings::Settings, id: &str) -> Option<&'static str> {
    let named = |value: &Option<String>| value.as_deref() == Some(id);
    if named(&settings.models.live) {
        return Some("speech");
    }
    if named(&settings.models.refine) {
        return Some("refinement");
    }
    if named(&settings.models.vad) {
        return Some("voice activity");
    }
    if named(&settings.models.speaker) {
        return Some("speaker");
    }
    if settings
        .llm
        .translator
        .as_ref()
        .is_some_and(|mt| mt.is_local() && mt.model.as_deref() == Some(id))
    {
        return Some("translation");
    }
    None
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
            // `code` when there is one, so the interface can say this in the user's language;
            // `error` always, so a client that does not know the code still shows something. See
            // `summo_core::Error::Message`.
            let mut body = serde_json::json!({ "error": message });
            if let Some(code) = e.code() {
                body["code"] = serde_json::Value::String(code.to_string());
            }
            (status, Json(body)).into_response()
        }
    }
}

async fn library(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<LibraryQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    // The clock is read here rather than inside the vault so "the last seven days" is anchored to
    // the machine the user is looking at, in its own offset.
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
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
    if let Err(rejection) = state.guard(&headers, query.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
struct ColourBody {
    /// Absent or `null` clears the colour. A client that wants no colour says so by omitting it,
    /// rather than by sending an empty string that would then have to mean two things.
    #[serde(default)]
    colour: Option<String>,
}

async fn set_colour(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ColourBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let id = summo_core::MeetingId::from(id);
    as_response(
        state
            .library
            .set_colour(&id, body.colour.as_deref())
            .map(|colour| serde_json::json!({ "colour": colour })),
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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

/// MCP over HTTP, on the daemon the app is already talking to.
///
/// `summo mcp` serves the same thing over stdio, which is what an editor spawns. This is for the
/// other case: an agent that is not in a position to spawn a process — a container, another
/// machine on the same host, a client that only speaks HTTP. Both call `summo_mcp::handle`, so
/// there is one implementation and two ways to reach it rather than two that drift.
///
/// It is behind the same token as every other route. That is the whole authorisation story: the
/// daemon is on loopback, the token is in a file readable by the user who started it, and an MCP
/// client is not more trusted than the app.
///
/// A notification — a request with no `id` — is answered with `202 Accepted` and no body, because
/// JSON-RPC says a notification gets no reply and a client that receives one may report it as a
/// protocol error.
async fn mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(request): Json<summo_mcp::Request>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    match summo_mcp::handle(state.engine.paths(), &request) {
        Some(response) => Json(response).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn storage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    as_response((|| {
        let settings = summo_core::Settings::load(&paths.settings())?;
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let meeting = summo_core::MeetingId::from(id);
    let path = match crate::audio_stream::locate(state.engine.paths(), &meeting, &lane) {
        Ok(path) => path,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let total = match std::fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(e) => {
            let message = format!("cannot read {}: {e}", path.display());
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response();
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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

#[derive(Debug, Deserialize)]
struct AskBody {
    question: String,
}

/// Answer a question from the vault, with citations.
async fn ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<AskBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let client = match llm_client(&state) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    as_response(crate::ask::ask(state.engine.paths(), &client, &body.question).await)
}

/// Without recognition the fields are still parsed — a caller should get "this build cannot
/// transcribe" rather than "malformed body" — but nothing reads them.
#[cfg_attr(not(feature = "models"), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct ImportBody {
    /// Absolute path to a media file on this machine.
    path: String,
    /// Model to decode with. Omitted means "whatever the app is configured to record with".
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    language: Option<String>,
    /// Attribute speakers. On by default: an imported recording is usually a room, and a wall of
    /// unattributed text is the thing people complain about first.
    #[serde(default = "yes")]
    diarize: bool,
}

/// Start importing a recording. Returns immediately with a job to poll.
async fn start_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ImportBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(spawn_import(&state, body))
}

#[cfg(feature = "models")]
fn spawn_import(state: &AppState, body: ImportBody) -> summo_core::Result<crate::imports::Job> {
    use summo_core::segment::Lane;

    let source = std::path::PathBuf::from(&body.path);
    crate::imports::check(&source)?;

    let model = match body.model {
        Some(model) => model,
        None => default_import_model(state)?,
    };
    let mut spec = crate::protocol::SessionSpec::new(model);
    // The offline path decodes on the system lane; the spec has to open the lane it will be fed.
    spec.lanes = vec![Lane::System];
    spec.language = body.language;
    spec.diarize = body.diarize;
    spec.validate()?;

    let title = summo_media::title_from(&source);
    let imports = state.engine.imports().clone();
    let id = imports.add(title, &source);
    let job = imports.get(&id).expect("just added");

    let paths = state.engine.paths().clone();
    let store = state.engine.store();
    let hw = state.engine.hardware().clone();

    // Its own thread, not a tokio task: decoding is CPU-bound for minutes at a time and would
    // otherwise starve the runtime that is serving this daemon's other requests.
    std::thread::spawn(move || {
        crate::imports::run(&imports, &id, &paths, &store, &hw, &spec, &source);
    });

    Ok(job)
}

/// Without recognition compiled in there is nothing to decode with, and pretending otherwise would
/// leave a job that sits at "queued" forever.
#[cfg(not(feature = "models"))]
fn spawn_import(_state: &AppState, _body: ImportBody) -> summo_core::Result<crate::imports::Job> {
    Err(summo_core::Error::Other(
        "bản build này không có nhận dạng giọng nói".into(),
    ))
}

/// The model an import should use when the caller did not name one.
///
/// The most recently installed transcription model, so "import this" works before the user has
/// opened settings — and fails with advice rather than a silent no-op when nothing is installed.
#[cfg(feature = "models")]
fn default_import_model(state: &AppState) -> summo_core::Result<String> {
    state
        .engine
        .store()
        .list()
        .into_iter()
        .find(|m| m.task == summo_models::Task::Asr)
        .map(|m| m.id.to_string())
        .ok_or_else(|| {
            summo_core::Error::Other(
                "chưa cài mô hình nhận dạng nào; chạy `summo setup` trước".into(),
            )
        })
}

/// Every import this daemon has run, newest first.
async fn list_imports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(Ok::<_, summo_core::Error>(state.engine.imports().list()))
}

/// One import's progress.
async fn get_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        state
            .engine
            .imports()
            .get(&id)
            .ok_or_else(|| summo_core::Error::Other(format!("không có lần nhập nào tên {id}"))),
    )
}

/// Forget the jobs that have finished, leaving the ones still running.
async fn clear_imports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let cleared = state.engine.imports().clear_finished();
    as_response(Ok::<_, summo_core::Error>(
        serde_json::json!({ "cleared": cleared }),
    ))
}

/// Hand an `@agent` task to the agent and wait for it.
///
/// Synchronous: the caller pressed "Run Task" and is watching the step list fill in, so a failure
/// belongs on their screen rather than in a log. The steps themselves are written to the vault as
/// they happen, so a client that navigates away still sees the trace when it comes back.
async fn run_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let paths = state.engine.paths().clone();
    let board = match crate::board::read(&paths) {
        Ok(board) => board,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let Some(task) = board.agent.into_iter().find(|t| t.id == id) else {
        return as_response(Err::<serde_json::Value, _>(summo_core::Error::Other(
            format!("no task with id {id} belongs to the agent"),
        )));
    };

    as_response(summo_agent::run::run(&paths, &task).await.map(|ran| {
        serde_json::json!({
            "task": ran.task,
            "status": ran.status.as_str(),
            "outcome": ran.outcome,
            "steps": ran.steps,
        })
    }))
}

/// Build an LLM client from settings, or say why not.
fn llm_client(state: &AppState) -> Result<summo_llm::LlmClient> {
    llm_for_engine(&state.engine)
}

/// The configured language model, built from settings plus the key in the environment.
///
/// Split from [`llm_client`] because the WebSocket session has an `EngineState` and no `AppState`,
/// and a second copy of provider resolution is how the socket and the HTTP routes end up talking to
/// different models.
fn llm_for_engine(engine: &EngineState) -> Result<summo_llm::LlmClient> {
    let settings = summo_core::settings::Settings::load(&engine.paths().settings())?;
    let provider = summo_llm::Provider::resolve_in(
        &summo_llm::provider::catalogue(&engine.paths().providers()),
        &settings.llm.provider,
        settings.llm.model.as_deref(),
        // Where a key comes from is `Provider::resolve`'s decision: SUMMO_API_KEY first, then
        // the provider's own variable. Passing one here would have been a second, narrower
        // copy of that policy.
        None,
    )?;
    summo_llm::LlmClient::new(provider)
}

/// The unapproved summary of a meeting, if there is one.
async fn get_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(crate::draft::load(
        state.engine.paths(),
        &summo_core::MeetingId::from(id),
    ))
}

#[derive(Debug, Deserialize)]
struct GenerateBody {
    #[serde(default)]
    template: Option<String>,
}

async fn generate_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<GenerateBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let client = match llm_client(&state) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    as_response(
        crate::draft::generate(
            state.engine.paths(),
            &client,
            &summo_core::MeetingId::from(id),
            body.template.as_deref(),
        )
        .await,
    )
}

#[derive(Debug, Deserialize)]
struct RefineBody {
    heading: String,
    /// The passage the user selected, verbatim.
    selection: String,
    instruction: String,
}

/// Rewrite one selected passage. Everything outside it stays byte-identical.
async fn refine_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<RefineBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let client = match llm_client(&state) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    as_response(
        crate::draft::refine(
            state.engine.paths(),
            &client,
            &summo_core::MeetingId::from(id),
            &body.heading,
            &body.selection,
            &body.instruction,
        )
        .await,
    )
}

#[derive(Debug, Deserialize)]
struct ChatBody {
    message: String,
}

async fn chat_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ChatBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let client = match llm_client(&state) {
        Ok(client) => client,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    as_response(
        crate::draft::chat(
            state.engine.paths(),
            &client,
            &summo_core::MeetingId::from(id),
            &body.message,
        )
        .await,
    )
}

async fn confirm_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        crate::draft::confirm(state.engine.paths(), &summo_core::MeetingId::from(id))
            .map(|sections| serde_json::json!({ "confirmed": sections })),
    )
}

async fn discard_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        crate::draft::discard(state.engine.paths(), &summo_core::MeetingId::from(id))
            .map(|removed| serde_json::json!({ "removed": removed })),
    )
}

/// What the agent wants to tell the user right now.
///
/// The daemon decides *whether* to speak; the shell decides how. Keeping the decision here means
/// the same rules apply to a desktop notification, a badge and a mobile push.
async fn nudges(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let today = now.date().to_string();
    let paths = state.engine.paths();

    let result = (|| {
        let recording = state.engine.status().is_recording();
        let mut seen = crate::nudge::load(paths)?;
        let mut due = crate::nudge::due(
            paths,
            &seen,
            &today,
            now.hour(),
            now.date().weekday().number_from_monday(),
            recording,
        )?;
        // The calendar prompt goes first: it is about the next five minutes, and everything else
        // here is about a day that has already happened.
        let suggest = summo_core::Settings::load(&paths.settings())
            .unwrap_or_default()
            .recording
            .suggest_on_meeting;
        let mut soon = crate::nudge::meeting_soon(
            paths,
            &seen,
            &today,
            now.unix_timestamp(),
            recording,
            suggest,
        );
        soon.append(&mut due);
        let due = soon;
        crate::nudge::record(paths, &mut seen, &due, &today)?;
        Ok(due)
    })();
    as_response(result)
}

/// The summary shapes installed, so the interface can offer a choice.
async fn templates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let settings = match summo_core::settings::Settings::load(&state.engine.paths().settings()) {
        Ok(settings) => settings,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let provider = match summo_llm::Provider::resolve_in(
        &summo_llm::provider::catalogue(&state.engine.paths().providers()),
        &settings.llm.provider,
        settings.llm.model.as_deref(),
        // Where a key comes from is `Provider::resolve`'s decision: SUMMO_API_KEY first, then
        // the provider's own variable. Passing one here would have been a second, narrower
        // copy of that policy.
        None,
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

#[derive(Debug, Deserialize)]
struct TranslateBody {
    /// Target language tag: `en`, `ja`, `vi`.
    lang: String,
    /// Re-translate lines that already have a translation. For when the glossary changed and the
    /// old output is wrong rather than merely missing.
    #[serde(default)]
    force: bool,
    /// Terms that must be translated a particular way, as `source => target` pairs.
    #[serde(default)]
    glossary: std::collections::BTreeMap<String, String>,
}

/// Translate a meeting into another language, writing a file beside it.
async fn translate_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<TranslateBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    // Not `llm_client`: translation may be pointed at a model of its own, and which model it is
    // decides the prompt as well as the endpoint. See `translate::Translator`.
    let translator = match summo_core::settings::Settings::load(&state.engine.paths().settings())
        .and_then(|settings| {
            crate::translate::Translator::from_settings(state.engine.paths(), &settings)
        }) {
        Ok(translator) => translator,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let meeting = summo_core::MeetingId::from(id);
    let doc = match load_meeting_doc(&state, &meeting) {
        Ok(doc) => doc,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let mut glossary = summo_llm::prompt::Glossary::default();
    for (from, to) in body.glossary {
        glossary.terms.push((from, to));
    }

    let result = crate::translate::translate(
        state.engine.paths(),
        &translator,
        &meeting,
        &doc,
        &body.lang,
        &glossary,
        body.force,
    )
    .await
    .map(|outcome| {
        serde_json::json!({
            "lang": outcome.lang,
            "translated": outcome.translated,
            "missing": outcome.missing,
            "requests": outcome.requests,
            "complete": outcome.complete(),
        })
    });
    as_response(result)
}

/// Which languages a meeting already exists in.
async fn meeting_translations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let langs =
        summo_vault::translation::languages(state.engine.paths(), &summo_core::MeetingId::from(id));
    as_response(Ok::<_, summo_core::Error>(langs))
}

/// One translation, as lines aligned to the transcript by `seq`.
async fn meeting_translation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, lang)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let meeting = summo_core::MeetingId::from(id);
    let result =
        summo_vault::translation::load(state.engine.paths(), &meeting, &lang).and_then(|found| {
            let translation = found.ok_or_else(|| {
                summo_core::Error::Other(format!("chưa dịch buổi họp này sang `{lang}`"))
            })?;
            Ok(serde_json::json!({
                "lang": translation.lang,
                "model": translation.model,
                "lines": translation.lines.iter().map(|l| serde_json::json!({
                    "seq": l.seq,
                    "t0": l.t0,
                    "text": l.text,
                })).collect::<Vec<_>>(),
            }))
        });
    as_response(result)
}

#[derive(Debug, Deserialize)]
struct SubtitleQuery {
    #[serde(default)]
    token: Option<String>,
    /// `srt` or `vtt`; anything `summo_vault::export` knows is accepted.
    #[serde(default = "default_subtitle_format")]
    format: String,
    /// Omit for the original language.
    #[serde(default)]
    lang: Option<String>,
}

fn default_subtitle_format() -> String {
    "srt".into()
}

/// A meeting as a subtitle file, optionally in a language it was translated into.
///
/// Served as text rather than JSON: the caller saves it or feeds it to a player, and wrapping a
/// subtitle file in a JSON string only means somebody has to unescape it again.
async fn meeting_subtitles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<SubtitleQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let meeting = summo_core::MeetingId::from(id);
    let doc = match load_meeting_doc(&state, &meeting) {
        Ok(doc) => doc,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let doc = match &q.lang {
        Some(lang) => match summo_vault::translation::load(state.engine.paths(), &meeting, lang) {
            Ok(Some(translation)) => crate::translate::applied(&doc, &translation),
            Ok(None) => {
                return as_response(Err::<serde_json::Value, _>(summo_core::Error::Other(
                    format!("chưa dịch buổi họp này sang `{lang}`"),
                )));
            }
            Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
        },
        None => doc,
    };

    let Some(format) = summo_vault::export::Format::parse(&q.format) else {
        return as_response(Err::<serde_json::Value, _>(summo_core::Error::Other(
            format!("không biết định dạng `{}`", q.format),
        )));
    };

    match summo_vault::export::export(&doc, format, summo_vault::export::Options::default()) {
        Ok(text) => (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            text,
        )
            .into_response(),
        Err(e) => as_response(Err::<serde_json::Value, _>(e)),
    }
}

/// Read one meeting's document off disk.
fn load_meeting_doc(
    state: &AppState,
    meeting: &summo_core::MeetingId,
) -> summo_core::Result<summo_vault::MeetingDoc> {
    let vault = state.engine.paths().vault();
    let path = crate::summarize::find_meeting_file(&vault, meeting)?;
    summo_vault::open(&vault, &path)
}

/// Interface translations the user dropped into `~/.summo/locales/`.
///
/// Unauthenticated content in the sense that anyone who can write to that directory can change it —
/// but so can they change the binary, so this adds no reach. It is still behind the token like
/// everything else.
async fn locales(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(Ok::<_, summo_core::Error>(crate::locales::load(
        state.engine.paths(),
    )))
}

/// What still stands between this install and a working recording.
async fn onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let status = crate::onboarding::status(state.engine.paths(), state.engine.hardware());
    let should_prompt = status.should_prompt();
    let needs_attention = status.needs_attention();
    as_response(Ok::<_, summo_core::Error>(serde_json::json!({
        "acknowledged": status.acknowledged,
        "can_record": status.can_record,
        "fresh": status.fresh,
        "should_prompt": should_prompt,
        "needs_attention": needs_attention,
        "checks": status.checks,
        "hardware": status.hardware,
    })))
}

/// Remember that the user has been through setup.
async fn complete_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        crate::onboarding::acknowledge(state.engine.paths())
            .map(|()| serde_json::json!({ "acknowledged": true })),
    )
}

#[derive(Debug, Deserialize)]
struct RecommendQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default = "default_lang")]
    lang: String,
    /// Registry to read from. Defaults to the built-in one.
    #[serde(default)]
    registry: Option<String>,
}

fn default_lang() -> String {
    "vi".into()
}

/// Rank the models available for a language on this machine, and say why.
///
/// The reasons matter more than the ranking. "Fits in 8 GB" and "Vietnamese word error rate 9.1%"
/// let a user disagree with the choice; a bare list asks them to trust it.
async fn recommend_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RecommendQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let manifests = match candidates(&state, q.registry.as_deref()).await {
        Ok(manifests) => manifests,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let installed: std::collections::HashSet<String> = state
        .engine
        .store()
        .list()
        .into_iter()
        .map(|m| m.id.to_string())
        .collect();

    let ranked = summo_models::recommend::recommend(&manifests, state.engine.hardware(), &q.lang);
    let models: Vec<_> = ranked
        .ranked
        .iter()
        .map(|scored| {
            let manifest = manifests.iter().find(|m| m.id.as_str() == scored.id);
            serde_json::json!({
                "id": scored.id,
                "name": scored.name,
                "score": scored.score,
                "reason": scored.reason,
                "live_capable": scored.live_capable,
                "expected_rtf": scored.expected_rtf,
                "accuracy": scored.accuracy,
                "installed": installed.contains(&scored.id),
                "size_bytes": manifest.map(|m| m.size_bytes),
                "license": manifest.map(|m| m.license.clone()),
                // Shown, not hidden: a non-redistributable or gated model is a real choice the user
                // is allowed to make, and they should know which one they are making.
                "redistributable": manifest.map(|m| m.redistributable),
                "gated": manifest.map(|m| m.gated),
            })
        })
        .collect();

    as_response(Ok::<_, summo_core::Error>(serde_json::json!({
        "lang": q.lang,
        "models": models,
        "rejected": ranked.rejected,
    })))
}

/// Remember which language is being spoken.
///
/// Stored in the daemon's settings rather than only in the browser, because it is a fact about this
/// installation and not about one browser profile. The record bar keeps its own copy — it has to
/// answer before any network call completes, and it is a per-meeting choice as often as a standing
/// one — but a first run that picks Japanese must still be recording Japanese in a different
/// browser, from the tray, or from `summo transcribe`, none of which can read `localStorage`.
///
/// An empty value clears it, which is not the same as never having chosen: cleared means the model
/// decides, and Whisper's own detection is what that resolves to.
async fn set_language(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<LanguageBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    as_response((|| {
        let path = state.engine.paths().settings();
        let mut settings = summo_core::Settings::load(&path)?;
        let code = body.language.trim().to_lowercase();
        settings.models.language = (!code.is_empty()).then_some(code);
        settings.save(&path)?;
        Ok(serde_json::json!({ "language": settings.models.language }))
    })())
}

/// Build a decoder now, so the next recording does not wait for one.
///
/// Over HTTP rather than only as the `model_load` socket command, because the socket exists only
/// while a session does — and the whole point of warming is to happen when nothing is recording.
/// The interface calls this when it opens and after a meeting ends.
///
/// Synchronous: it answers when the model is ready, which is what lets the caller show "ready"
/// rather than "asked for". Around three and a half seconds.
#[cfg(feature = "models")]
async fn warm_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    // Not while recording: the session owns the decoder it is using, and building a second one
    // would double resident memory in the middle of the meeting it would be trying to protect.
    if state.engine.status().is_recording() {
        return as_response(Ok::<_, summo_core::Error>(
            serde_json::json!({ "ready": null, "skipped": "recording" }),
        ));
    }

    let spec = resolve_models(&crate::protocol::SessionSpec::new(""), &state.engine);

    // Nothing installed yet, so nothing to warm. Not an error: warming is a nudge the interface
    // sends whenever it opens the record card, and answering 400 to it made a first run log a
    // failed request on a screen where nothing is wrong — the browser suites caught exactly that.
    if spec.live_model.trim().is_empty() {
        return as_response(Ok::<_, summo_core::Error>(
            serde_json::json!({ "ready": null, "skipped": "no model installed" }),
        ));
    }

    let engine = state.engine.clone();

    // On a blocking thread: building a decoder is seconds of CPU inside ONNX Runtime, and holding
    // an async worker for that long starves every other request the daemon is serving.
    let built = tokio::task::spawn_blocking(move || {
        crate::warm::build(&spec, &engine.store(), engine.hardware()).map(|(key, decoder)| {
            let described = serde_json::json!({ "model": key.model, "language": key.language });
            engine.warm().put(key, decoder);
            described
        })
    })
    .await;

    match built {
        Ok(Ok(ready)) => as_response(Ok::<_, summo_core::Error>(
            serde_json::json!({ "ready": ready }),
        )),
        Ok(Err(e)) => as_response(Err::<serde_json::Value, _>(e)),
        Err(e) => as_response(Err::<serde_json::Value, _>(summo_core::Error::Other(
            format!("warming panicked: {e}"),
        ))),
    }
}

/// Every language this registry can recognise, and what would serve each one.
///
/// The screen this feeds replaces a guess. Setup used to recommend a model for whatever language
/// the *interface* was in — right often enough that being wrong was invisible, and being wrong
/// means a download that cannot transcribe the meeting it was installed for.
///
/// Ranked per language rather than filtered: "covered" and "good" are different claims, and this
/// carries the measured accuracy so a picker can show that Whisper covers Vietnamese at 34 % and a
/// 73 MB transducer does it at 91 %.
async fn languages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RecommendQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let manifests = match candidates(&state, q.registry.as_deref()).await {
        Ok(manifests) => manifests,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };
    let installed: Vec<String> = state
        .engine
        .store()
        .list()
        .into_iter()
        .map(|m| m.id.to_string())
        .collect();

    let languages =
        summo_models::languages::available(&manifests, state.engine.hardware(), &installed);
    let settings = summo_core::Settings::load(&state.engine.paths().settings()).unwrap_or_default();

    as_response(Ok::<_, summo_core::Error>(serde_json::json!({
        // What the next recording would use, so a picker can open on the current answer rather than
        // on a default that silently disagrees with the settings file.
        "current": settings.models.language,
        "languages": languages,
    })))
}

/// Every manifest worth ranking: what is installed, plus the registry when it can be reached.
///
/// An unreachable registry ranks the installed models rather than failing. Offline is a supported
/// state for a local-first tool, and a setup screen that refuses to render without a network is the
/// opposite of the promise.
async fn candidates(
    state: &AppState,
    registry: Option<&str>,
) -> summo_core::Result<Vec<summo_models::Manifest>> {
    let mut manifests = state.engine.store().list();

    let reg = match registry {
        Some(spec) => {
            summo_models::Registry::with_sources(vec![summo_models::RegistrySource::parse(spec)?])?
        }
        None => summo_models::Registry::discover()?,
    };

    match reg.index().await {
        Ok(index) => {
            for entry in index.models {
                if manifests.iter().any(|m| m.id == entry.id) {
                    continue;
                }
                match reg.manifest(&entry.id).await {
                    Ok(m) => manifests.push(m),
                    Err(e) => tracing::warn!(id = %entry.id, error = %e, "skipping"),
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "registry unavailable; ranking installed models only"),
    }
    Ok(manifests)
}

#[derive(Debug, Deserialize)]
struct InstallBody {
    /// Model id, as it appears in the registry.
    id: String,
    #[serde(default)]
    registry: Option<String>,
}

/// Start downloading a model. Returns immediately; poll `/installs/{id}`.
async fn start_install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<InstallBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let id = match summo_core::ModelId::parse(&body.id) {
        Ok(id) => id,
        Err(e) => {
            return as_response(Err::<serde_json::Value, _>(summo_core::Error::Other(e)));
        }
    };

    let registry = match match &body.registry {
        Some(spec) => summo_models::RegistrySource::parse(spec)
            .and_then(|source| summo_models::Registry::with_sources(vec![source])),
        None => summo_models::Registry::discover(),
    } {
        Ok(registry) => registry,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let manifest = match registry.manifest(&id).await {
        Ok(manifest) => manifest,
        Err(e) => return as_response(Err::<serde_json::Value, _>(e)),
    };

    let installs = state.engine.installs().clone();
    let job = installs.claim(&id, &manifest.name);
    if !matches!(job.state, crate::install::State::Queued) {
        // Already running: hand back the job in flight rather than starting a second download that
        // would fight it for the same staging file.
        return as_response(Ok::<_, summo_core::Error>(job));
    }

    let store = state.engine.store();
    let downloads = state.engine.paths().downloads();
    let home = state.engine.paths().root().to_path_buf();
    let key = id.to_string();

    tokio::spawn(async move {
        let outcome = async {
            let downloader = summo_models::Downloader::new(downloads)?
                .with_credentials(summo_models::credentials::Credentials::discover(&home));
            installs.set(
                &key,
                crate::install::State::Downloading { done: 0, total: 0 },
            );

            let progress = installs.clone();
            let progress_key = key.clone();
            store
                .install(&manifest, &downloader, move |p| {
                    progress.set(
                        &progress_key,
                        crate::install::State::Downloading {
                            done: p.done,
                            total: p.total,
                        },
                    );
                })
                .await?;
            Ok::<_, summo_core::Error>(())
        }
        .await;

        match outcome {
            Ok(()) => installs.set(&key, crate::install::State::Done),
            Err(e) => installs.set(
                &key,
                crate::install::State::Failed {
                    // Resume is automatic, so a failure is not wasted work and the message says so.
                    error: format!("{e} — thử lại sẽ tiếp tục từ chỗ dừng"),
                },
            ),
        }
    });

    as_response(Ok::<_, summo_core::Error>(
        state.engine.installs().get(&body.id).unwrap_or(job),
    ))
}

async fn list_installs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(Ok::<_, summo_core::Error>(state.engine.installs().list()))
}

async fn get_install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        state
            .engine
            .installs()
            .get(&id)
            .ok_or_else(|| summo_core::Error::Other(format!("không có lần tải nào cho `{id}`"))),
    )
}

#[derive(Debug, Deserialize)]
struct NoteBody {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    /// `YYYY-MM-DD`. Defaults to today, in the user's own timezone.
    #[serde(default)]
    day: Option<String>,
}

use summo_core::today;

/// Notes the user typed, newest first.
async fn list_notes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let result = summo_vault::note::list(state.engine.paths()).map(|entries| {
        entries
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id.to_string(),
                    "title": e.title,
                    "day": e.day,
                    // The colour and the tags a note carries. They were on the listing entry the
                    // whole time and this route dropped them, so the notes screen was the one place
                    // a colour somebody had set did not show — the mark is meant to be the same
                    // mark wherever the note appears.
                    "color": e.color,
                    "tags": e.tags,
                    "file": e.path.display().to_string(),
                })
            })
            .collect::<Vec<_>>()
    });
    as_response(result)
}

async fn create_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<NoteBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let day = body.day.unwrap_or_else(today);
    let result = summo_vault::note::create(state.engine.paths(), &body.title, &day, &body.body)
        .map(|(id, path)| {
            serde_json::json!({ "id": id.to_string(), "file": path.display().to_string() })
        });
    as_response(result)
}

async fn read_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let result = summo_vault::note::read(state.engine.paths(), &summo_core::MeetingId::from(id))
        .map(|doc| {
            serde_json::json!({
                "title": doc.title,
                "body": doc.body,
                // Everything under the title, which is what the editor edits. `body` alone stops
                // at the first `##`, so a note with headings — one written in Obsidian, one
                // started from a template, one saved out of a composed message — opened with most
                // of itself missing and was then saved back that way.
                "text": summo_vault::note::as_text(&doc),
                "frontmatter": doc.frontmatter,
                "sections": doc.sections.iter().map(|s| serde_json::json!({
                    "heading": s.heading,
                    "body": s.body,
                })).collect::<Vec<_>>(),
            })
        });
    as_response(result)
}

async fn update_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<NoteBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let title = body.title.trim();
    let result = summo_vault::note::set_body(
        state.engine.paths(),
        &summo_core::MeetingId::from(id),
        &body.body,
        (!title.is_empty()).then_some(title),
    )
    .map(|path| serde_json::json!({ "file": path.display().to_string() }));
    as_response(result)
}

async fn delete_note(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(
        summo_vault::note::remove(state.engine.paths(), &summo_core::MeetingId::from(id))
            .map(|removed| serde_json::json!({ "removed": removed })),
    )
}

/// The interface itself, when it is compiled in.
///
/// No token check. This is the page that *receives* the token — asking for one first would be a
/// chicken-and-egg, and the assets are the same public bundle anybody can download. Everything the
/// page then does is authenticated.
async fn interface(State(state): State<AppState>, uri: axum::http::Uri) -> impl IntoResponse {
    let port = state.port.load(std::sync::atomic::Ordering::Relaxed);
    crate::assets::serve(uri.path(), port, state.token.as_str())
}

/// Everything said about one note, and what the agent is still waiting on.
async fn list_comments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let result = summo_vault::annotate::load(state.engine.paths(), &id).map(|thread| {
        serde_json::json!({
            "annotations": thread.annotations,
            // Split out rather than left for the client to filter: "what is waiting on me" is the
            // question the panel opens with, and two clients filtering it two ways would disagree.
            "pending": thread.pending().len(),
        })
    });
    as_response(result)
}

#[derive(Debug, Deserialize)]
struct CommentBody {
    body: String,
    /// Display name of whoever is writing. Defaults to the local user.
    #[serde(default)]
    author: Option<String>,
    /// Pin to one utterance. Omit for a comment about the whole note.
    #[serde(default)]
    seq: Option<u64>,
    /// Pin to a `##` section instead.
    #[serde(default)]
    heading: Option<String>,
}

/// Say something about a note.
async fn add_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<CommentBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    // A segment anchor wins over a section one: it is the more specific of the two, and a client
    // sending both means the user selected a line inside a section.
    let anchor = match (body.seq, body.heading.as_deref().map(str::trim)) {
        (Some(seq), _) => summo_vault::annotate::Anchor::Segment { seq },
        (None, Some(heading)) if !heading.is_empty() => summo_vault::annotate::Anchor::Section {
            heading: heading.to_string(),
        },
        _ => summo_vault::annotate::Anchor::Note,
    };

    let author = body
        .author
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("Bạn")
        .to_string();

    let paths = state.engine.paths();
    let result = summo_vault::annotate::load(paths, &id).and_then(|mut thread| {
        let added = thread
            .comment(&author, &body.body, anchor, now_iso())?
            .clone();
        summo_vault::annotate::save(paths, &id, &thread)?;
        Ok(added)
    });
    as_response(result)
}

async fn delete_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, comment)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    let result = summo_vault::annotate::load(paths, &id).and_then(|mut thread| {
        let removed = thread.remove(&comment);
        if removed {
            summo_vault::annotate::save(paths, &id, &thread)?;
        }
        Ok(serde_json::json!({ "removed": removed }))
    });
    as_response(result)
}

#[derive(Debug, Deserialize)]
struct ReactBody {
    emoji: String,
    #[serde(default)]
    by: Option<String>,
}

/// Toggle a reaction. Reactions are on comments; a proposal is answered by accepting it.
async fn react_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, comment)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<ReactBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let who = body
        .by
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("Bạn");

    let paths = state.engine.paths();
    let result = summo_vault::annotate::load(paths, &id).and_then(|mut thread| {
        thread.react(&comment, &body.emoji, who)?;
        summo_vault::annotate::save(paths, &id, &thread)?;
        Ok(serde_json::json!({ "ok": true }))
    });
    as_response(result)
}

/// Now, in the offset the machine is in.
///
/// The offset is kept rather than normalised to UTC: a comment written at 18:00 in Hanoi reads as
/// 18:00 to the person who wrote it, and a thread that renders their own words in a different hour
/// than they typed them is a thread they distrust.
fn now_iso() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now.unix_timestamp().to_string())
}

/// Meetings from the user's calendars, in time order.
async fn agenda(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(Ok::<_, summo_core::Error>(crate::agenda::agenda(
        state.engine.paths(),
    )))
}

#[derive(Debug, Deserialize)]
struct SuggestQuery {
    #[serde(default)]
    token: Option<String>,
    /// When the recording started, as seconds since the epoch.
    started_epoch: i64,
}

/// What a recording that started at a given moment was probably for.
///
/// A suggestion, never an action. Nothing on this route starts, stops or renames anything — the app
/// offers the title and the user accepts it, because an app that titles a meeting from a calendar
/// without asking will eventually put a client's name on the wrong transcript.
async fn suggest_meeting(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SuggestQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(Ok::<_, summo_core::Error>(crate::agenda::suggest(
        state.engine.paths(),
        q.started_epoch,
    )))
}

#[derive(Debug, Deserialize)]
struct CalendarBody {
    /// Path to a `.ics` file on this machine.
    path: String,
    /// What to call it. Becomes the filename, so it is sanitised.
    name: String,
}

/// Install a calendar file the user picked.
async fn add_calendar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<CalendarBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let result = crate::agenda::install(
        state.engine.paths(),
        std::path::Path::new(&body.path),
        &body.name,
    )
    .map(|path| serde_json::json!({ "path": path.display().to_string() }));
    as_response(result)
}

async fn remove_calendar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    // A subscription owns its file, so unsubscribing removes both. A calendar that was copied in
    // has no subscription and only the file to remove — and asking in that order means a
    // subscription never leaves its `.ics` behind to keep filling the agenda.
    let result = crate::calsync::unsubscribe(paths, &name).and_then(|removed| {
        if removed {
            Ok(true)
        } else {
            crate::agenda::forget(paths, &name)
        }
    });
    as_response(result.map(|removed| serde_json::json!({ "removed": removed })))
}

#[derive(Debug, Default, Deserialize)]
struct ShutdownBody {
    /// Stop even though a recording is in progress.
    #[serde(default)]
    force: bool,
}

/// Ask the daemon to stop.
///
/// Refused while recording unless forced. `summo stop` in a terminal cannot see that a meeting is
/// being recorded in the tray, and a daemon that exits on request would end that meeting with no
/// question asked — the answer being "yes, obviously" often enough that it must still be asked.
async fn shutdown(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Option<Json<ShutdownBody>>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let force = body.is_some_and(|Json(b)| b.force);
    if state.engine.status().is_recording() && !force {
        let refused: Result<serde_json::Value> = Err(Error::msg(
            "daemon.recording",
            "đang ghi âm — dừng buổi ghi trước, hoặc dùng --force",
        ));
        return as_response(refused);
    }
    // `notify_one` rather than `notify_waiters`: it leaves a permit behind, so a request that
    // arrives in the instant before the waiter is registered still stops the daemon.
    state.stopping.notify_one();
    as_response(Ok(serde_json::json!({ "stopping": true })))
}

/// Every calendar the app knows about: subscriptions with their sync state, and files copied in.
async fn list_calendars(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths();
    let result = crate::calsync::list(paths).map(|subscriptions| {
        let subscribed: std::collections::HashSet<_> =
            subscriptions.iter().map(|s| s.name.clone()).collect();
        // Files the user copied in themselves, which have no URL and never refresh. Listed anyway:
        // an agenda showing meetings from a source with no row in Settings is a mystery.
        let files: Vec<_> = crate::agenda::load(paths)
            .into_iter()
            .filter(|(name, _)| !subscribed.contains(name))
            .map(|(name, events)| serde_json::json!({ "name": name, "events": events.len() }))
            .collect();
        serde_json::json!({ "subscriptions": subscriptions, "files": files })
    });
    as_response(result)
}

#[derive(Debug, Deserialize)]
struct SubscribeBody {
    title: String,
    url: String,
}

async fn subscribe_calendar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<SubscribeBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(crate::calsync::subscribe(state.engine.paths(), &body.title, &body.url).await)
}

#[derive(Debug, Default, Deserialize)]
struct RefreshBody {
    /// One calendar, or every one of them.
    #[serde(default)]
    name: Option<String>,
}

async fn refresh_calendars(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    body: Option<Json<RefreshBody>>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let name = body.and_then(|Json(b)| b.name);
    as_response(crate::calsync::refresh(state.engine.paths(), name.as_deref()).await)
}

/// Everyone Summo can recognise.
async fn people(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(crate::people::unknowns(
        &state.book,
        &state.engine.paths().voices(),
        &summo_core::MeetingId::from(id),
    ))
}

/// Every unnamed voice in the vault, so the voice book has the work on it.
///
/// The per-meeting route above answers "who is speaking in the thing I am looking at". This one
/// answers "what is still unnamed anywhere", which is the question the voice book screen is for and
/// the one it could not previously ask.
async fn unknown_voices_everywhere(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(crate::people::unknowns_everywhere(
        &state.book,
        &state.engine.paths().voices(),
        &state.library,
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    as_response(crate::people::name_voice(
        &state.book,
        &state.engine.paths().voices(),
        &state.library,
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
    /// The dedicated translation model, or `Some(None)` to go back to using the general one.
    ///
    /// Doubly optional on purpose. Absent means "leave it alone", which is what every existing
    /// caller sends; present-and-null means "turn it off". Collapsing the two would make every
    /// save from an older client silently delete the user's translator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translator: Option<Option<summo_core::settings::Translator>>,
}

/// The API key, from the environment.
///
/// Read at the moment it is used rather than held, so a key rotated in the shell that launched the
/// daemon does not require a restart to take effect on the next call.
fn api_key() -> Option<String> {
    std::env::var("SUMMO_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

/// The roster: every agent, as the files describe them.
///
/// Read on every request rather than cached. An agent is a file the user can edit in Obsidian
/// while the app is open, and a cache would mean the screen showing something the vault no longer
/// says — which for a feature whose whole promise is "it is just files" would be the one bug that
/// undermines it.
async fn agents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths().clone();
    as_response(
        summo_agent::Roster::load_or_seed(&paths.agents()).map(|roster| {
            let base_tools = roster.base_tools().to_vec();
            let list: Vec<serde_json::Value> = roster
                .all()
                .map(|agent| {
                    let memory = summo_agent::memory::load(&agent.memory_path());
                    let tasks = summo_agent::memory::tasks(
                        &agent.tasks_path(),
                        &format!("agents/{}/TASKS.md", agent.slug),
                    );
                    serde_json::json!({
                        "slug": agent.slug,
                        "name": agent.head.name,
                        "description": agent.head.description,
                        "brief": agent.brief,
                        "provider": agent.head.provider,
                        "model": agent.head.model,
                        "spawns": agent.head.spawns,
                        // Resolved, not as written: an agent that lists nothing inherits the base's
                        // grant, and the screen should show what it can actually call.
                        "tools": if agent.head.tools.is_empty() {
                            base_tools.clone()
                        } else {
                            agent.head.tools.clone()
                        },
                        "memory": memory.iter().map(|f| serde_json::json!({
                            "learned": f.learned,
                            "text": f.text,
                        })).collect::<Vec<_>>(),
                        "tasks": tasks,
                        "open_tasks": tasks.iter().filter(|t| !t.status.is_finished()).count(),
                    })
                })
                .collect();

            serde_json::json!({
                "agents": list,
                "base": roster.base(),
                "base_tools": base_tools,
                // A `spawns` entry naming nothing is invisible until a run tries to delegate, at
                // which point the failure looks like the model's fault.
                "dangling": roster.dangling_spawns(),
                "skipped": roster.skipped().iter().map(|(path, reason)| serde_json::json!({
                    "path": path.display().to_string(),
                    "reason": reason,
                })).collect::<Vec<_>>(),
            })
        }),
    )
}

/// One agent's definition, as the text of its own file.
///
/// The raw Markdown, not a parsed shape. Editing an agent is editing a document — the frontmatter
/// carries settings a form could render, but the brief below it is prose, and a form that could
/// only express what the form knows about would make the file format the lesser thing.
async fn agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths().clone();
    as_response(agent_file(&paths, &slug).and_then(|path| {
        std::fs::read_to_string(&path)
            .map(|text| serde_json::json!({ "slug": slug, "definition": text }))
            .map_err(|e| summo_core::Error::io(&path, e))
    }))
}

#[derive(Debug, Deserialize)]
struct AgentBody {
    definition: String,
}

/// Replace an agent's definition.
///
/// Creates the directory when it is not there, so "add an agent" and "edit an agent" are the same
/// request — which is what it means for an agent to be a folder rather than a record.
async fn set_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(q): Query<TokenQuery>,
    Json(body): Json<AgentBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let paths = state.engine.paths().clone();
    as_response((|| {
        let dir = agent_dir(&paths, &slug)?;
        std::fs::create_dir_all(&dir).map_err(|e| summo_core::Error::io(&dir, e))?;
        let path = dir.join("AGENT.md");
        std::fs::write(&path, body.definition.as_bytes())
            .map_err(|e| summo_core::Error::io(&path, e))?;

        // Read it back through the parser rather than reporting success on a write. A definition
        // that will not parse is one the roster silently skips, and the moment to say so is now.
        let roster = summo_agent::Roster::load(&paths.agents())?;
        match roster.get(&slug) {
            Some(agent) => Ok(serde_json::json!({ "slug": slug, "name": agent.head.name })),
            None => Err(summo_core::Error::msg(
                "agent.unreadable",
                "đã lưu, nhưng file không đọc được — kiểm tra phần YAML ở đầu".to_string(),
            )),
        }
    })())
}

/// An agent's directory, refusing anything that is not a plain name.
///
/// A slug comes from a URL. Without this, `../../..` in a path segment is a write anywhere the
/// daemon can reach.
fn agent_dir(
    paths: &summo_core::paths::Paths,
    slug: &str,
) -> summo_core::Result<std::path::PathBuf> {
    let clean = slug.trim();
    let ok = !clean.is_empty()
        && clean.len() <= 64
        && clean
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(summo_core::Error::msg(
            "agent.bad_slug",
            format!("`{slug}` không phải tên agent hợp lệ"),
        ));
    }
    Ok(paths.agents().join(clean))
}

fn agent_file(
    paths: &summo_core::paths::Paths,
    slug: &str,
) -> summo_core::Result<std::path::PathBuf> {
    Ok(agent_dir(paths, slug)?.join("AGENT.md"))
}

/// Every endpoint Summo knows, and whether this machine can already reach it.
///
/// Served rather than hardcoded in the interface: the settings screen used to keep its own array of
/// four providers, which meant adding one was two edits in two languages and the two facts it did
/// not have — which variable holds the key, and whether that variable is set — could not be shown
/// at all. A user with `GEMINI_API_KEY` already exported now sees Gemini as ready.
///
/// `key_set` is a boolean. The key itself never crosses this boundary.
async fn llm_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }
    let presets: Vec<serde_json::Value> =
        summo_llm::provider::catalogue(&state.engine.paths().providers())
            .iter()
            .map(|endpoint| {
                serde_json::json!({
                    "id": endpoint.id,
                    "name": endpoint.name,
                    "base_url": endpoint.base_url,
                    "model": endpoint.model,
                    "local": endpoint.local(),
                    "key_env": endpoint.key_env,
                    "key_set": summo_llm::provider::key_for(Some(endpoint), &endpoint.id).is_some(),
                })
            })
            .collect();
    Json(serde_json::json!({ "providers": presets })).into_response()
}

async fn set_llm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TokenQuery>,
    Json(body): Json<LlmBody>,
) -> impl IntoResponse {
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let path = state.engine.paths().settings();
    as_response((|| {
        // Refuse a provider that cannot be resolved rather than writing it and failing later, when
        // the user has moved on and the error has nothing to do with what they are doing.
        summo_llm::Provider::resolve_in(
            &summo_llm::provider::catalogue(&state.engine.paths().providers()),
            &body.provider,
            body.model.as_deref(),
            Some("probe"),
        )?;

        let mut settings = summo_core::Settings::load(&path)?;
        settings.llm.provider = body.provider.trim().to_string();
        settings.llm.model = body.model.filter(|m| !m.trim().is_empty());
        if let Some(language) = body.language.filter(|l| !l.trim().is_empty()) {
            settings.llm.language = language;
        }
        if let Some(on_stop) = body.summarize_on_stop {
            settings.llm.summarize_on_stop = on_stop;
        }
        if let Some(translator) = body.translator {
            settings.llm.translator =
                translator
                    .filter(|t| !t.provider.trim().is_empty())
                    .map(|t| summo_core::settings::Translator {
                        provider: t.provider.trim().to_string(),
                        model: t.model.filter(|m| !m.trim().is_empty()),
                    });
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
        return rejection.into_response();
    }

    let provider = match summo_llm::Provider::resolve_in(
        &summo_llm::provider::catalogue(&state.engine.paths().providers()),
        &body.provider,
        body.model.as_deref(),
        // Where a key comes from is `Provider::resolve`'s decision: SUMMO_API_KEY first, then
        // the provider's own variable. Passing one here would have been a second, narrower
        // copy of that policy.
        None,
    ) {
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
    if let Err(rejection) = state.guard(&headers, q.token.as_deref()) {
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
        Command::ModelLoad { id } | Command::ModelSwap { id, .. } => vec![Event::Error {
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
    /// What this session was started with, so a mid-meeting change is an edit of it rather than a
    /// new set of assumptions.
    spec: crate::protocol::SessionSpec,
    runner: crate::runner::SessionRunner,
    recorder: crate::recorder::Recorder,
    archive: crate::archive::AudioArchive,
    started: std::time::Instant,
    /// Set when the session asked for live translation.
    live: Option<crate::live::LiveTranslator>,
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
            // Before `begin`, not inside `start_session`: `begin` validates the spec, and an empty
            // live model is exactly what validation refuses. Resolving afterwards would mean the
            // interface — which deliberately names no model — could never start a recording at all.
            let spec = resolve_models(&spec, engine);
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
            // Kept before the session is consumed, so the slot can be refilled for whatever the
            // *next* meeting will most likely be: the same model and language as this one.
            let finished = session.as_ref().map(|active| active.spec.clone());
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

            // Refill the slot for the next meeting, off the socket. Rebuilding takes about three
            // and a half seconds and this thread is the one carrying audio and events; doing it
            // here would stall the stop the user just asked for.
            if let Some(spec) = finished {
                let engine = engine.clone();
                std::thread::spawn(move || {
                    match crate::warm::build(&spec, &engine.store(), engine.hardware()) {
                        Ok((key, decoder)) => engine.warm().put(key, decoder),
                        // Nothing broken: the next recording loads its own decoder exactly as it
                        // did before this optimisation existed.
                        Err(e) => {
                            tracing::debug!(error = %e, "could not pre-load the next decoder")
                        }
                    }
                });
            }
            (events, None)
        }
        // Build a decoder now, so the next recording does not wait for one.
        //
        // Answered with an error in every build until today: this arm fell through to the handler
        // for daemons compiled *without* recognition, which says "rebuild with --features models"
        // — in a build that has them. The comment on the command has promised this since it was
        // written: "so the first recording is not delayed by it".
        Command::ModelLoad { id } => {
            let mut spec = crate::protocol::SessionSpec::new(id.trim());
            spec.language = None;
            let spec = resolve_models(&spec, engine);
            match crate::warm::build(&spec, &engine.store(), engine.hardware()) {
                Ok((key, decoder)) => {
                    let said = key.language.clone().unwrap_or_else(|| "auto".into());
                    let model = key.model.clone();
                    engine.warm().put(key, decoder);
                    (
                        vec![Event::info(format!("{model} ready ({said})"))],
                        session,
                    )
                }
                Err(e) => (vec![Event::error(&e)], session),
            }
        }

        // Change the language, or the model, without ending the meeting.
        //
        // The file, the utterances already committed and the audio archive all continue; only the
        // decoder is rebuilt, so the next utterance is heard by the new one. The open utterance is
        // lost rather than re-decoded — its audio lives inside the pipeline being replaced, and
        // half a sentence transcribed twice is worse than half a sentence missing.
        Command::ModelSwap { id, language } => {
            let Some(mut active) = session else {
                // Not an error worth failing on: a client that swaps before recording is asking for
                // the setting, and the setting is an HTTP call away.
                return (
                    vec![Event::error(&summo_core::Error::Config(
                        "no recording to change; set the model or language in settings instead"
                            .into(),
                    ))],
                    None,
                );
            };

            let mut spec = active.spec.clone();
            if !id.trim().is_empty() {
                spec.live_model = id.trim().to_string();
            }
            if let Some(language) = language {
                let language = language.trim().to_lowercase();
                spec.language = (!language.is_empty()).then_some(language);
            }
            // An empty model with a new language is the common case — the interface names a
            // language and lets the daemon pick what hears it.
            let spec = resolve_models(&spec, engine);

            match crate::runner::SessionRunner::with_warm(
                &spec,
                &engine.store(),
                engine.hardware(),
                Some(engine.warm()),
            ) {
                Ok(runner) => {
                    let said = spec.language.clone().unwrap_or_else(|| "auto".into());
                    active.runner = runner;
                    active.spec = spec.clone();
                    // So `/status` — and the banner reading it — says what is true now.
                    engine.retuned(&spec);
                    (
                        vec![Event::info(format!(
                            "now listening with {} in {said}",
                            spec.live_model
                        ))],
                        Some(active),
                    )
                }
                // The old pipeline is still in `active` and still working, so a failed swap leaves
                // the meeting recording rather than ending it. Losing a meeting because a model
                // would not load is the worst possible answer to "change the language".
                Err(e) => (vec![Event::error(&e)], Some(active)),
            }
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

    let runner = crate::runner::SessionRunner::with_warm(
        spec,
        &engine.store(),
        engine.hardware(),
        Some(engine.warm()),
    )?;

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

    let recorder = crate::recorder::Recorder::start(engine.paths(), id, &title, &date, models)?;

    // A translation the user asked for but cannot have — no provider configured — is reported when
    // the session starts rather than silently producing no subtitles for an hour.
    let live = match spec.translate_to.as_deref().map(str::trim) {
        Some(lang) if !lang.is_empty() => {
            match summo_core::settings::Settings::load(&engine.paths().settings()).and_then(
                |settings| crate::translate::Translator::from_settings(engine.paths(), &settings),
            ) {
                Ok(translator) => Some(crate::live::LiveTranslator::new(
                    translator,
                    crate::live::LiveConfig {
                        lang: lang.to_string(),
                        glossary: summo_llm::prompt::Glossary::default(),
                    },
                )),
                Err(e) => {
                    tracing::warn!(error = %e, "live translation asked for but no model is configured");
                    None
                }
            }
        }
        _ => None,
    };

    Ok(ActiveSession {
        spec: spec.clone(),
        runner,
        recorder,
        archive,
        live,
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

            // Live translation rides the same connection. It never blocks: `offer` queues the
            // finals, may spawn a request, and returns whatever earlier requests have already sent
            // back — so a slow model delays subtitles, never audio.
            let mut events = events;
            if let Some(live) = active.live.as_mut() {
                events.extend(live.offer(&events));
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

#[cfg(all(test, feature = "models"))]
mod resolve_tests {
    use super::*;
    use summo_core::paths::Paths;

    fn engine(home: &std::path::Path) -> EngineState {
        EngineState::new(Paths::at(home)).unwrap()
    }

    /// The bug this exists for: the interface sent a hardcoded `gipformer-65m`, so installing a
    /// model for another language changed nothing about what recording reached for — and on a
    /// machine without that model, recording failed naming one the user never chose.
    #[test]
    fn a_session_that_names_no_model_takes_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine(tmp.path());
        let path = engine.paths().settings();
        let mut settings = summo_core::Settings::default();
        settings.models.live = Some("sense-voice-small".into());
        settings.models.language = Some("ja".into());
        settings.save(&path).unwrap();

        let resolved = resolve_models(&crate::protocol::SessionSpec::new(""), &engine);
        assert_eq!(resolved.live_model, "sense-voice-small");
        assert_eq!(resolved.language.as_deref(), Some("ja"));
    }

    /// The hole the language picker left when it was first built: the choice lived in one browser's
    /// `localStorage`, so `/languages` always answered `current: null`, a second browser started
    /// over, and the tray and the CLI never learned it at all.
    #[test]
    fn the_spoken_language_survives_the_browser_that_chose_it() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine(tmp.path());
        let path = engine.paths().settings();

        let mut settings = summo_core::Settings::default();
        settings.models.language = Some("ja".into());
        settings.save(&path).unwrap();

        // A session that names no language takes it, which is what makes the setting worth writing.
        let resolved = resolve_models(&crate::protocol::SessionSpec::new(""), &engine);
        assert_eq!(resolved.language.as_deref(), Some("ja"));

        // And clearing it means detection, not the previous answer left behind.
        let mut settings = summo_core::Settings::load(&path).unwrap();
        settings.models.language = None;
        settings.save(&path).unwrap();
        let resolved = resolve_models(&crate::protocol::SessionSpec::new(""), &engine);
        assert_eq!(resolved.language, None);
    }

    /// Installing a second speech model used to break recording. With nothing named in the
    /// settings the resolver only handled "exactly one", so the moment a user added a model for
    /// another language, every recording failed with `session needs a live model` — and the
    /// interface, which deliberately names none, could not start one at all.
    #[test]
    fn a_second_installed_model_does_not_break_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine(tmp.path());

        // Two speech models on disk. `list()` reads the manifest directory, so writing manifests is
        // what "installed" means here — no blobs are needed to choose between them.
        std::fs::create_dir_all(engine.paths().manifests()).unwrap();
        for (id, langs) in [("gipformer-65m", r#"["vi"]"#), ("whisper-tiny", r#"["*"]"#)] {
            std::fs::write(
                engine.paths().manifests().join(format!("{id}.json")),
                format!(
                    r#"{{"schema":1,"id":"{id}","name":"{id}","task":"asr","mode":"live",
                        "runtime":"test","langs":{langs},"license":"MIT","size_bytes":1,
                        "profile":{{"rtf":{{"cpu_x86_avx512vnni_8t":0.02}},
                                   "quality":{{"wer_fleurs_vi":0.09}}}},
                        "files":[{{"name":"m.onnx","sha256":"{sha}","size":1,
                                  "url":"https://example.invalid/m"}}]}}"#,
                    sha = "a".repeat(64)
                ),
            )
            .unwrap();
        }

        let mut settings = summo_core::Settings::default();
        settings.models.language = Some("vi".into());
        settings.save(&engine.paths().settings()).unwrap();

        let resolved = resolve_models(&crate::protocol::SessionSpec::new(""), &engine);
        assert_eq!(
            resolved.live_model, "gipformer-65m",
            "the language decides between them"
        );
    }

    /// A client that knows which model it wants keeps it. The import job names one on purpose.
    #[test]
    fn a_named_model_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine(tmp.path());
        let mut settings = summo_core::Settings::default();
        settings.models.live = Some("sense-voice-small".into());
        settings.save(&engine.paths().settings()).unwrap();

        let resolved = resolve_models(&crate::protocol::SessionSpec::new("whisper-tiny"), &engine);
        assert_eq!(resolved.live_model, "whisper-tiny");
    }

    /// Nothing chosen and nothing installed is left empty, so `validate` refuses it with a message
    /// about a missing model rather than this inventing one.
    #[test]
    fn nothing_to_choose_from_stays_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = engine(tmp.path());
        assert!(
            resolve_models(&crate::protocol::SessionSpec::new(""), &engine)
                .live_model
                .is_empty()
        );
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
            .get(format!(
                "http://{}/meetings/01A/audio/system",
                server.addr()
            ))
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

        // A lane that is *only* dots — `..` or `%2E%2E` — is not in this list, and cannot be: the
        // URL parser in the client collapses both before a request is sent, so the daemon is asked
        // for `/meetings/01A/` and answers it correctly. What is tested here is the case that does
        // arrive intact, a single path segment with an encoded separator inside it, which is what
        // would reach the file system if the lane were joined onto a path unchecked.
        for lane in [
            "..%2F..%2Fsecret",
            "mic%2F..%2F..%2Fsecret",
            "%2E%2E%2Fsecret",
        ] {
            let resp = client()
                .get(format!(
                    "http://{}/meetings/01A/audio/{lane}",
                    server.addr()
                ))
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
        assert_eq!(
            body["owners"].as_array().unwrap().len(),
            2,
            "the agent is not an owner"
        );
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
        assert!(
            body.contains("Giữ nguyên câu này."),
            "the notes were rewritten: {body}"
        );
    }

    #[tokio::test]
    async fn a_summary_bullet_can_become_a_task() {
        let (tmp, server) = running().await;
        seed_with_body(
            &tmp,
            "01A",
            "2026-08-10T09:00:00+07:00",
            "Họp",
            "## Tóm tắt\nX.\n",
        );

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
    async fn running_a_task_that_is_not_the_agents_is_refused() {
        let (tmp, server) = running().await;
        seed_with_body(
            &tmp,
            "01A",
            "2026-08-10T09:00:00+07:00",
            "Họp",
            "## Việc cần làm\n- [ ] @ngoc Việc của người <!-- id:T1 -->\n",
        );

        let resp = client()
            .post(format!("http://{}/tasks/T1/run", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error(), "got {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("agent"),
            "expected an explanation, got {body}"
        );
    }

    #[tokio::test]
    async fn running_a_task_that_does_not_exist_is_a_404() {
        let (_tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/tasks/NOPE/run", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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
            .post(format!(
                "http://{}/meetings/01NOPE/summarize",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        // Either no model is configured or the meeting is missing; both are the caller's problem
        // and both must come back as a 4xx with a message rather than a 500.
        assert!(resp.status().is_client_error(), "got {}", resp.status());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(
            body["error"].is_string(),
            "expected an explanation, got {body}"
        );
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

    /// The voice book screen is "the questions first, then the people already known", and the
    /// questions half could only be reached from inside a meeting — so opening it from its own
    /// destination showed the second half and nothing else. This is the route that gives it work.
    #[tokio::test]
    async fn the_voice_book_can_ask_about_every_unnamed_voice_in_the_vault() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Họp đầu tuần");
        seed(&tmp, "01B", "2026-08-11T10:00:00+07:00", "Demo khách hàng");
        seed_voice(&tmp, "01A", "S2", [0.0, 1.0, 0.0, 0.0]);
        seed_voice(&tmp, "01B", "S4", [0.0, 0.0, 1.0, 0.0]);
        let base = format!("http://{}", server.addr());

        let asking: serde_json::Value = client()
            .get(format!("{base}/voices/unknown"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(asking.as_array().unwrap().len(), 2, "{asking}");
        // Newest first: a voice from this morning is one the user can still place from memory, and
        // one from last March is the one they will want the suggestions for. `ordering_key` negates
        // the timestamp, so the obvious `b.cmp(a)` sorts these backwards.
        assert_eq!(asking[0]["meeting"], "01B");
        assert_eq!(asking[1]["meeting"], "01A");
        // The meeting it came from, so the user knows which conversation they are naming somebody
        // in — a label like `S2` on its own is not a question anybody can answer.
        assert_eq!(asking[1]["title"], "Họp đầu tuần");
        assert_eq!(asking[1]["voices"][0]["label"], "S2");

        client()
            .post(format!("{base}/meetings/01A/voices/S2"))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "name": "Bình" }))
            .send()
            .await
            .unwrap();

        // Answered questions leave the list, rather than the meeting staying on it with no voices
        // under it.
        let after: serde_json::Value = client()
            .get(format!("{base}/voices/unknown"))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let left: Vec<&str> = after
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["meeting"].as_str().unwrap())
            .collect();
        assert_eq!(left, ["01B"], "{after}");
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

    /// Writes one meeting into the vault and returns its id.
    fn seed_meeting(paths: &Paths) -> summo_core::MeetingId {
        use summo_core::segment::{Lane, Segment};
        use summo_vault::meeting::{Frontmatter, MeetingDoc};

        let id = summo_core::MeetingId::new();
        let mut doc = MeetingDoc::new(Frontmatter::new(id.clone(), "2026-08-10"), "Họp ngân sách");
        doc.transcript
            .push(Segment::new(1, Lane::System, "xin chào", 0.0, 2.0));
        doc.transcript
            .push(Segment::new(2, Lane::System, "cảm ơn", 3.0, 4.0));

        let dir = paths.meetings();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hop.md"), doc.to_markdown().unwrap()).unwrap();
        id
    }

    #[tokio::test]
    async fn a_meeting_with_no_translation_lists_no_languages() {
        let (tmp, server) = running().await;
        let id = seed_meeting(&Paths::at(tmp.path()));

        let langs: Vec<String> = client()
            .get(format!(
                "http://{}/meetings/{id}/translations",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(langs.is_empty());
        server.shutdown();
    }

    /// Subtitles in a language nobody translated into must say so, not silently hand back the
    /// original — a user who asked for English and got Vietnamese has no way to tell it failed.
    #[tokio::test]
    async fn subtitles_in_an_untranslated_language_are_refused() {
        let (tmp, server) = running().await;
        let id = seed_meeting(&Paths::at(tmp.path()));

        let resp = client()
            .get(format!(
                "http://{}/meetings/{id}/subtitles?format=srt&lang=en",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error() || resp.status().is_server_error());
        server.shutdown();
    }

    #[tokio::test]
    async fn subtitles_come_back_as_a_subtitle_file_not_as_json() {
        let (tmp, server) = running().await;
        let paths = Paths::at(tmp.path());
        let id = seed_meeting(&paths);

        let mut translation = summo_vault::translation::Translation::new("en");
        translation.set(1, 0.0, "hello");
        summo_vault::translation::save(&paths, &id, &translation).unwrap();

        let resp = client()
            .get(format!(
                "http://{}/meetings/{id}/subtitles?format=srt&lang=en",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("-->"), "not an srt: {body}");
        assert!(body.contains("hello"), "{body}");
        // The line the model did not return keeps its original text rather than leaving a hole.
        assert!(body.contains("cảm ơn"), "{body}");
        server.shutdown();
    }

    #[tokio::test]
    async fn an_unknown_subtitle_format_is_named_in_the_error() {
        let (tmp, server) = running().await;
        let id = seed_meeting(&Paths::at(tmp.path()));

        let resp = client()
            .get(format!(
                "http://{}/meetings/{id}/subtitles?format=wat",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error() || resp.status().is_server_error());
        assert!(resp.text().await.unwrap().contains("wat"));
        server.shutdown();
    }

    /// A note is the same document as a meeting, so everything already built for meetings works on
    /// it. The round trip through HTTP is what proves it end to end.
    /// The bug this guards: a browser sends `Origin` on every write, even same-origin. With the
    /// interface served by the daemon itself, refusing that origin meant an app that could read and
    /// never write — every note, task and confirmation came back 403.
    #[tokio::test]
    async fn the_page_this_daemon_serves_may_write_back_to_it() {
        let (_tmp, server) = running().await;
        let origin = format!("http://127.0.0.1:{}", server.addr().port());

        let response = client()
            .post(format!("http://{}/notes", server.addr()))
            .header("origin", &origin)
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "title": "Ghi chú", "body": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "same-origin write must be allowed");
        server.shutdown();
    }

    /// Narrower than trusting loopback: another port on this machine is somebody else's server.
    #[tokio::test]
    async fn a_page_on_another_local_port_still_cannot_write() {
        let (_tmp, server) = running().await;
        let response = client()
            .post(format!("http://{}/notes", server.addr()))
            .header("origin", "http://127.0.0.1:3000")
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "title": "Ghi chú", "body": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_comment_can_be_written_reacted_to_and_removed_over_http() {
        let (_tmp, server) = running().await;
        let base = format!("http://{}", server.addr());
        let token = server.token().as_str().to_string();

        let added: serde_json::Value = client()
            .post(format!("{base}/meetings/01A1/comments"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "body": "Chỗ này sai", "author": "Ngọc", "seq": 12 }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = added["id"].as_str().expect("an id").to_string();
        assert_eq!(added["anchor"]["on"], "segment");
        assert_eq!(added["anchor"]["seq"], 12);

        let reacted = client()
            .post(format!("{base}/meetings/01A1/comments/{id}/react"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "emoji": "👍", "by": "Bình" }))
            .send()
            .await
            .unwrap();
        assert_eq!(reacted.status(), 200);

        let thread: serde_json::Value = client()
            .get(format!("{base}/meetings/01A1/comments"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(thread["annotations"].as_array().unwrap().len(), 1);
        assert_eq!(thread["annotations"][0]["reactions"][0]["emoji"], "👍");

        let removed: serde_json::Value = client()
            .delete(format!("{base}/meetings/01A1/comments/{id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(removed["removed"], true);
        server.shutdown();
    }

    #[tokio::test]
    async fn an_empty_comment_is_refused_over_http() {
        let (_tmp, server) = running().await;
        let response = client()
            .post(format!("http://{}/meetings/01A1/comments", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "body": "   " }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error() || response.status().is_server_error());
        server.shutdown();
    }

    #[tokio::test]
    async fn a_meeting_with_nothing_said_about_it_has_an_empty_thread() {
        let (_tmp, server) = running().await;
        let thread: serde_json::Value = client()
            .get(format!("http://{}/meetings/01A1/comments", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(thread["annotations"].as_array().unwrap().is_empty());
        assert_eq!(thread["pending"], 0);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_note_can_be_written_read_edited_and_deleted_over_http() {
        let (_tmp, server) = running().await;
        let base = format!("http://{}", server.addr());
        let token = server.token().as_str().to_string();

        let created: serde_json::Value = client()
            .post(format!("{base}/notes"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "title": "Ý tưởng", "body": "Ghi nhanh." }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = created["id"].as_str().expect("an id").to_string();

        let listed: Vec<serde_json::Value> = client()
            .get(format!("{base}/notes"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["title"], "Ý tưởng");

        let updated = client()
            .post(format!("{base}/notes/{id}"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "body": "Đã sửa." }))
            .send()
            .await
            .unwrap();
        assert_eq!(updated.status(), 200);

        let read: serde_json::Value = client()
            .get(format!("{base}/notes/{id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(read["body"], "Đã sửa.");
        // The title survives an edit to the body — it names the file, and renaming on every save
        // would break the user's links.
        assert_eq!(read["title"], "Ý tưởng");

        let deleted: serde_json::Value = client()
            .delete(format!("{base}/notes/{id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(deleted["removed"], true);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_note_without_a_title_is_refused_over_http() {
        let (_tmp, server) = running().await;
        let response = client()
            .post(format!("http://{}/notes", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "title": "  ", "body": "x" }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error() || response.status().is_server_error());
        server.shutdown();
    }

    #[tokio::test]
    async fn a_daemon_with_no_calendars_has_an_empty_agenda() {
        let (_tmp, server) = running().await;
        let entries: Vec<serde_json::Value> = client()
            .get(format!("http://{}/agenda", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(entries.is_empty());
        server.shutdown();
    }

    /// A suggestion is a suggestion. Nothing on this route may start, stop or rename anything — an
    /// app that titles a meeting from a calendar without asking eventually puts a client's name on
    /// the wrong transcript.
    #[tokio::test]
    async fn asking_for_a_suggestion_with_no_calendars_answers_nothing() {
        let (_tmp, server) = running().await;
        let response = client()
            .get(format!(
                "http://{}/agenda/suggest?started_epoch=1786000000",
                server.addr()
            ))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(
            response
                .json::<serde_json::Value>()
                .await
                .unwrap()
                .is_null()
        );
        server.shutdown();
    }

    #[tokio::test]
    async fn a_calendar_that_is_not_a_calendar_is_refused_when_it_is_added() {
        let (tmp, server) = running().await;
        let source = tmp.path().join("nope.ics");
        std::fs::write(&source, "<html>error</html>").unwrap();

        let response = client()
            .post(format!("http://{}/calendars", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "path": source, "name": "work" }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error() || response.status().is_server_error());
        server.shutdown();
    }

    #[tokio::test]
    async fn a_calendar_can_be_added_listed_and_removed_over_http() {
        let (tmp, server) = running().await;
        let source = tmp.path().join("work.ics");
        std::fs::write(
            &source,
            "BEGIN:VEVENT\r\nUID:standup\r\nDTSTART:20260810T090000Z\r\n\
DTEND:20260810T093000Z\r\nSUMMARY:Standup\r\nATTENDEE:mailto:a@x\r\n\
ATTENDEE:mailto:b@x\r\nEND:VEVENT\r\n",
        )
        .unwrap();

        let added = client()
            .post(format!("http://{}/calendars", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "path": source, "name": "work" }))
            .send()
            .await
            .unwrap();
        assert_eq!(added.status(), 200);

        let entries: Vec<serde_json::Value> = client()
            .get(format!("http://{}/agenda", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["summary"], "Standup");

        let removed: serde_json::Value = client()
            .delete(format!("http://{}/calendars/work", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(removed["removed"], true);
        server.shutdown();
    }

    #[tokio::test]
    async fn a_fresh_daemon_has_imported_nothing() {
        let (_tmp, server) = running().await;
        let jobs: Vec<serde_json::Value> = client()
            .get(format!("http://{}/imports", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(jobs.is_empty());
        server.shutdown();
    }

    /// A path that is not there has to fail at the request, not as a job that appears and then
    /// dies — otherwise a typo shows up in the UI as a broken import rather than as a typo.
    #[tokio::test]
    async fn importing_a_file_that_is_not_there_fails_without_creating_a_job() {
        let (_tmp, server) = running().await;
        let resp = client()
            .post(format!("http://{}/imports", server.addr()))
            .bearer_auth(server.token().as_str())
            .json(&serde_json::json!({ "path": "/definitely/not/here.mp4" }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error() || resp.status().is_server_error());

        let jobs: Vec<serde_json::Value> = client()
            .get(format!("http://{}/imports", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            jobs.is_empty(),
            "a rejected import must leave no job behind"
        );
        server.shutdown();
    }

    #[tokio::test]
    async fn asking_after_an_import_that_never_existed_says_so() {
        let (_tmp, server) = running().await;
        let resp = client()
            .get(format!("http://{}/imports/nope", server.addr()))
            .bearer_auth(server.token().as_str())
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_client_error() || resp.status().is_server_error());
        server.shutdown();
    }

    #[tokio::test]
    async fn everything_else_requires_the_token() {
        let (_tmp, server) = running().await;
        for path in [
            "hw",
            "models",
            "status",
            "library",
            "library/search",
            "imports",
        ] {
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
            .get(format!(
                "http://{}/library/search?q=ngan+sach",
                server.addr()
            ))
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

    /// A colour set over HTTP, found by filtering, and cleared again — and, in the middle, the one
    /// thing this route exists to make impossible: a colour that is really a stylesheet.
    #[tokio::test]
    async fn a_colour_is_set_filtered_by_and_refused_when_it_is_not_one() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");
        let base = format!("http://{}/meetings/01A", server.addr());
        let token = server.token().as_str().to_string();

        let colour = |body: serde_json::Value| {
            let base = base.clone();
            let token = token.clone();
            async move {
                client()
                    .post(format!("{base}/colour"))
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
            }
        };

        let resp = colour(serde_json::json!({ "colour": "teal" })).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.json::<serde_json::Value>().await.unwrap()["colour"],
            "teal"
        );

        let listed: serde_json::Value = client()
            .get(format!("http://{}/library?colour=teal", server.addr()))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed["total"], 1);
        assert_eq!(listed["colours"][0]["name"], "teal");
        assert_eq!(listed["colours"][0]["count"], 1);
        assert_eq!(
            listed["palette"].as_array().unwrap().len(),
            8,
            "the client takes its picker from this list rather than keeping a second copy"
        );

        // Refused at the edge, not sanitised somewhere later and half-written.
        let resp =
            colour(serde_json::json!({ "colour": "teal; background: url(https://x/)" })).await;
        assert_eq!(resp.status(), 400);
        let still: serde_json::Value = client()
            .get(&base)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            still["summary"]["color"], "teal",
            "the refused write changed nothing"
        );

        // No colour at all, which is how the picker's ninth option travels.
        let resp = colour(serde_json::json!({ "colour": null })).await;
        assert_eq!(resp.status(), 200);
        assert!(resp.json::<serde_json::Value>().await.unwrap()["colour"].is_null());
        server.shutdown();
    }

    /// The same server the editor plugin spawns, reachable over HTTP for an agent that cannot
    /// spawn a process — a container, a client that only speaks HTTP.
    #[tokio::test]
    async fn mcp_answers_over_http_behind_the_same_token() {
        let (tmp, server) = running().await;
        seed(&tmp, "01A", "2026-08-09T10:00:00+07:00", "Weekly Sync");
        let base = format!("http://{}/mcp", server.addr());
        let token = server.token().as_str().to_string();

        // No token, no answer. An MCP client is not more trusted than the app.
        let resp = client()
            .post(&base)
            .json(&serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);

        let handshake: serde_json::Value = client()
            .post(&base)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05" }
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(handshake["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(handshake["result"]["serverInfo"]["name"], "summo");

        // The vault the daemon is serving is the vault MCP reads — one machine, one set of files.
        let listed: serde_json::Value = client()
            .post(&base)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let resources = listed["result"]["resources"].as_array().unwrap();
        assert!(
            resources.iter().any(|r| r["name"] == "Weekly Sync"),
            "the seeded meeting must be listed: {resources:?}"
        );

        // A notification expects nothing back; answering one is a protocol error.
        let resp = client()
            .post(&base)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 202);
        assert!(resp.bytes().await.unwrap().is_empty());

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

        let saved = summo_core::Settings::load(&Paths::at(tmp.path()).settings()).unwrap();
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

        let saved = summo_core::Settings::load(&Paths::at(tmp.path()).settings()).unwrap();
        assert_eq!(
            saved.llm.provider, "ollama",
            "a bad provider must not be written"
        );
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
        let audio =
            Paths::at(tmp.path()).audio_for(&summo_core::MeetingId::from("01A".to_string()));
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
            .post(format!(
                "http://{}/storage/prune?dry_run=false",
                server.addr()
            ))
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

    /// Changing the language mid-meeting is only useful if the meeting survives it. Without a
    /// session there is nothing to change, and saying so beats loading a model nobody asked for.
    #[cfg(feature = "models")]
    #[test]
    fn a_swap_with_no_recording_says_where_the_setting_lives() {
        let (_tmp, engine) = engine();
        let swap = serde_json::to_string(&Command::ModelSwap {
            id: String::new(),
            language: Some("en".into()),
        })
        .unwrap();
        // `handle_command_with_models` and not `handle_command`: the swap belongs to the half of
        // the protocol that owns a pipeline, and the other half answers "built without recognition
        // support" for every model command, which is true of that build and not of this one.
        let (events, session) = handle_command_with_models(&swap, &engine, None);
        assert!(
            matches!(&events[0], Event::Error { message, .. } if message.contains("settings")),
            "{events:?}"
        );
        assert!(
            session.is_none(),
            "a failed swap must not invent a recording"
        );
    }

    /// The wire form matters as much as the behaviour: the interface sends a language and no model,
    /// and a `serde` default that made `id` mandatory would reject exactly that message.
    #[test]
    fn a_swap_may_name_a_language_without_naming_a_model() {
        let parsed: Command =
            serde_json::from_str(r#"{"cmd":"model_swap","language":"en"}"#).expect("parses");
        assert!(matches!(
            parsed,
            Command::ModelSwap { ref id, ref language }
                if id.is_empty() && language.as_deref() == Some("en")
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
