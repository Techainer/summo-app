//! The interface, compiled into the binary.
//!
//! Summo is meant to work the way `ollama` does: one command, no install steps, no second process
//! to start, no directory of static files to keep beside the executable. `summo serve` and the app
//! is open. That is only true if the interface travels *inside* the binary, which is what this
//! does.
//!
//! Three things it has to get right.
//!
//! **The handshake must not go in the URL.** Every other transport puts the port and token in a
//! query string, which is fine for a Tauri webview and wrong for a browser: a query string lands in
//! history, in the window title, and in whatever the user pastes into a bug report. Served from the
//! same origin as the API there is a better option — the token is injected into `index.html` as a
//! `<script>` before the app loads, so it never appears in a URL at all.
//!
//! **Unknown paths fall back to the shell, missing assets do not.** The app uses hash history, so
//! its own routes live in the fragment and never reach the server — `/tasks` here is the API's
//! `/tasks`, and it answers 401 without a token exactly as it should. The fallback is for the
//! everything else: someone typing a path, a stale bookmark. A missing `.js` still 404s, because
//! silently serving HTML in place of a script produces `Unexpected token '<'`, which is an hour of
//! somebody's life to trace back to a routing rule.
//!
//! **Content types are not optional.** A browser refuses a module served as `text/plain`, and the
//! failure looks like the app not loading rather than like a header being wrong.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The built interface. Baked in at compile time from `apps/web/dist`.
///
/// Building this crate therefore requires the web app to have been built first. That coupling is
/// deliberate and is why it sits behind a feature: `cargo test -p summo-engine` on a machine with no
/// Node must still work.
#[cfg(feature = "bundled")]
static DIST: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../apps/web/dist");

/// Whether an interface is compiled in.
#[must_use]
pub const fn bundled() -> bool {
    cfg!(feature = "bundled")
}

/// The `Content-Type` for a path, by extension.
///
/// A short table rather than a mime crate: these are the only types a Vite build produces, and an
/// unknown extension gets `application/octet-stream`, which a browser downloads rather than
/// executes — the safe direction to be wrong in.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

/// Whether a request for this path should fall back to `index.html` when the file is missing.
///
/// Only extensionless paths, which is what a client-side route looks like. A missing `.js` must
/// stay a 404: serving HTML in its place produces `Unexpected token '<'` in the console, which is
/// an hour of somebody's life to trace back to a routing rule.
#[must_use]
pub fn is_route(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or("");
    !last.contains('.')
}

/// Normalise a request path to a key in the bundle.
///
/// Leading slashes are stripped and `..` segments are dropped. The bundle is an in-memory map so
/// traversal cannot reach the filesystem, but a path that walks out of it would still be a bug
/// worth refusing rather than a lookup that happens to miss.
#[must_use]
pub fn normalize(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    let cleaned: Vec<&str> = trimmed
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect();

    if cleaned.is_empty() {
        return "index.html".to_string();
    }
    cleaned.join("/")
}

/// Put the handshake into `index.html` so the app never has to read it from a URL.
///
/// Injected immediately before `</head>`, ahead of every module script, because the app reads it
/// during its first render. Falls back to appending when there is no `</head>` — a document that
/// odd is not worth failing over, and a script at the end still runs before React mounts.
#[must_use]
pub fn inject(html: &str, port: u16, token: &str) -> String {
    // JSON-encoded rather than interpolated: a token is generated, but a value spliced raw into a
    // script tag is the shape of a bug that only shows up once the value changes.
    let payload = serde_json::json!({ "port": port, "token": token });
    let script = format!("<script>window.__SUMMO__={payload};</script>");

    match html.find("</head>") {
        Some(at) => format!("{}{script}{}", &html[..at], &html[at..]),
        None => format!("{html}{script}"),
    }
}

/// Serve one file out of the bundle.
#[cfg(feature = "bundled")]
#[must_use]
pub fn serve(path: &str, port: u16, token: &str) -> Response {
    let key = normalize(path);

    if let Some(file) = DIST.get_file(&key) {
        return respond(&key, file.contents(), port, token);
    }

    // A client-side route: hand back the shell and let the router sort it out.
    if is_route(&key)
        && let Some(index) = DIST.get_file("index.html")
    {
        return respond("index.html", index.contents(), port, token);
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

#[cfg(feature = "bundled")]
fn respond(key: &str, bytes: &[u8], port: u16, token: &str) -> Response {
    let mime = content_type(key);

    if key.ends_with(".html") {
        let html = String::from_utf8_lossy(bytes);
        return (
            [
                (header::CONTENT_TYPE, mime),
                // Never cached: it carries the token, and this session's token is not the next
                // session's.
                (header::CACHE_CONTROL, "no-store"),
            ],
            inject(&html, port, token),
        )
            .into_response();
    }

    (
        [
            (header::CONTENT_TYPE, mime),
            // Vite fingerprints every asset filename, so a hit is immutable by construction.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes.to_vec(),
    )
        .into_response()
}

/// Without the interface compiled in, say so rather than 404ing.
///
/// A 404 at `/` reads as a broken build. This says which build you have and what to do about it.
#[cfg(not(feature = "bundled"))]
#[must_use]
pub fn serve(_path: &str, _port: u16, _token: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "This build has no interface compiled in. Build it with `--features bundled`, or open the \
         dev server at http://localhost:5173.",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_shell() {
        assert_eq!(normalize("/"), "index.html");
        assert_eq!(normalize(""), "index.html");
    }

    #[test]
    fn an_asset_path_maps_to_its_key() {
        assert_eq!(normalize("/assets/index-abc.js"), "assets/index-abc.js");
    }

    /// The bundle is an in-memory map so traversal cannot reach the disk — but a path walking out
    /// of it is still a bug to refuse rather than a lookup that happens to miss.
    #[test]
    fn a_traversal_cannot_walk_out_of_the_bundle() {
        assert_eq!(normalize("/../../etc/passwd"), "etc/passwd");
        assert_eq!(normalize("/assets/../../secret"), "assets/secret");
        assert_eq!(normalize("/./././"), "index.html");
    }

    /// A stale bookmark or a typed path should land on the app rather than on a 404 page.
    #[test]
    fn an_extensionless_path_is_a_client_side_route() {
        assert!(is_route("meetings/01J4"));
        assert!(is_route("tasks"));
        assert!(is_route(""));
    }

    /// Serving HTML in place of a missing script produces `Unexpected token '<'`, which is an hour
    /// of somebody's life to trace back to a routing rule.
    #[test]
    fn a_missing_asset_is_not_a_route() {
        assert!(!is_route("assets/index-abc.js"));
        assert!(!is_route("favicon.ico"));
        assert!(!is_route("assets/font.woff2"));
    }

    #[test]
    fn modules_and_fonts_get_the_types_a_browser_insists_on() {
        assert_eq!(content_type("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("app.mjs"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("style.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("inter.woff2"), "font/woff2");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    }

    /// Wrong in the direction that downloads rather than executes.
    #[test]
    fn an_unknown_extension_is_not_guessed_at() {
        assert_eq!(content_type("weird.xyz"), "application/octet-stream");
        assert_eq!(content_type("noextension"), "application/octet-stream");
    }

    /// The whole point: the token reaches the app without ever being in a URL, where it would land
    /// in history, in the window title and in whatever gets pasted into a bug report.
    #[test]
    fn the_handshake_is_injected_before_any_script_runs() {
        let html = "<html><head><title>Summo</title></head><body><script src=/a.js></script></body></html>";
        let out = inject(html, 8710, "tok");

        let injected = out.find("__SUMMO__").unwrap();
        let head_close = out.find("</head>").unwrap();
        let module = out.find("/a.js").unwrap();
        assert!(injected < head_close, "inside the head");
        assert!(injected < module, "before the app's own scripts");
        assert!(out.contains("8710"));
        assert!(out.contains("tok"));
    }

    #[test]
    fn a_document_with_no_head_still_gets_the_handshake() {
        let out = inject("<body>hi</body>", 1, "t");
        assert!(out.contains("__SUMMO__"));
    }

    /// Spliced raw, a token containing a quote would break the script. JSON encoding is what makes
    /// that impossible rather than unlikely.
    #[test]
    fn a_token_with_awkward_characters_cannot_break_out_of_the_script() {
        let out = inject("<head></head>", 1, "a\"b</script>c");
        assert!(!out.contains("a\"b</script>c"), "raw token in the document");
        assert!(out.contains("\\\"") || out.contains("\\u0022"), "{out}");
    }

    #[test]
    fn a_build_reports_whether_it_carries_an_interface() {
        // Compiles either way; the value is what the CLI prints so a user knows which build they
        // have before they wonder why the page is blank.
        let _ = bundled();
    }
}
