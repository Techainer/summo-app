//! Running the engine inside the app's own process.
//!
//! On desktop, Summo ships the daemon as a sidecar binary and the app talks to it over loopback.
//! That is the right shape there: a bad allocation inside an ONNX kernel takes down the daemon and
//! not the meeting, and the app reconnects to the transcript it already has.
//!
//! **On mobile it is not an option.** iOS does not let an app spawn another executable — there is no
//! `fork`, no `posix_spawn` an App Store build may use, and no way to ship a second binary the
//! system will run. Android permits it but frowns on it, and a second process there is the first
//! thing the OS kills under memory pressure, which on a phone is constantly.
//!
//! So mobile links the engine in. Same `Server`, same routes, same protocol — the app cannot tell
//! the difference beyond the handshake arriving from a function call instead of a file. What it
//! loses is crash isolation, and saying that out loud is better than discovering it: a panic in a
//! decoder now takes the app with it, which is why [`start`] is careful about what it does before
//! the recording exists on disk.
//!
//! This is deliberately not mobile-only. A desktop build can use it too — for a test that needs a
//! daemon without a subprocess, and for anyone who would rather ship one binary.

use summo_core::{Result, paths::Paths};

use crate::{EngineState, Server, ServerConfig};

/// Everything the interface needs to talk to an engine.
///
/// The same three fields the sidecar publishes in `engine.json`, so the webview's handshake code is
/// identical whether the engine is in this process or another one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Handshake {
    pub port: u16,
    pub token: String,
}

/// A running in-process engine.
///
/// Dropping it does not stop the server — [`Server`] owns a spawned task — so the caller holds this
/// for the life of the app and calls [`Embedded::shutdown`] when the app really is closing. A
/// recording in progress must survive the window being closed, which is exactly why stopping is an
/// explicit act rather than a destructor.
pub struct Embedded {
    server: Server,
    handshake: Handshake,
}

impl Embedded {
    #[must_use]
    pub fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    /// The query string the webview is loaded with.
    ///
    /// The token travels in the URL rather than in a header because the first request is a document
    /// load, which cannot carry one. It stays a real credential: the daemon binds loopback only, the
    /// origin check refuses browser pages, and on mobile nothing else on the device can read another
    /// app's webview URL.
    #[must_use]
    pub fn query(&self) -> String {
        format!(
            "port={}&token={}",
            self.handshake.port, self.handshake.token
        )
    }

    pub fn shutdown(self) {
        self.server.shutdown();
    }
}

/// Start an engine inside this process.
///
/// `home` is the app's data directory — on mobile that is a sandboxed path the OS hands the app,
/// not `~/.summo`, which does not exist there.
///
/// No token file is written. There is nobody to read it: the only client is the webview this
/// process is about to load, and a credential on disk that nothing needs is a credential that ends
/// up in a backup.
pub async fn start(home: impl Into<std::path::PathBuf>) -> Result<Embedded> {
    let paths = Paths::at(home);
    let engine = EngineState::new(paths)?;

    let server = Server::start(
        engine,
        ServerConfig {
            // Whatever the OS gives us. A fixed port would collide with whatever else the phone is
            // running, and there is no user to tell about it.
            port: 0,
            write_token_file: false,
            // The webview loads from a custom scheme, not from loopback http, so the origin check
            // stays strict — which is the check that stops a web page reaching the microphone.
            allow_loopback_origins: false,
        },
    )
    .await?;

    let handshake = Handshake {
        port: server.addr().port(),
        token: server.token().as_str().to_string(),
    };

    Ok(Embedded { server, handshake })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_embedded_engine_answers_on_the_port_it_reports() {
        let tmp = tempfile::tempdir().unwrap();
        let embedded = start(tmp.path()).await.unwrap();

        let response = reqwest::get(format!(
            "http://127.0.0.1:{}/health",
            embedded.handshake().port
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        embedded.shutdown();
    }

    /// The credential is still a credential. An embedded engine that skipped auth because "it is
    /// the same process" would be reachable by anything else on the device that can open a socket.
    #[tokio::test]
    async fn the_token_is_still_required() {
        let tmp = tempfile::tempdir().unwrap();
        let embedded = start(tmp.path()).await.unwrap();
        let port = embedded.handshake().port;

        let anonymous = reqwest::get(format!("http://127.0.0.1:{port}/status"))
            .await
            .unwrap();
        assert_eq!(anonymous.status(), 401);

        let authorised = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/status"))
            .bearer_auth(&embedded.handshake().token)
            .send()
            .await
            .unwrap();
        assert_eq!(authorised.status(), 200);
        embedded.shutdown();
    }

    /// Nothing reads a token file here, and a credential on disk that nothing needs is a credential
    /// that ends up in a phone backup.
    #[tokio::test]
    async fn no_token_file_is_left_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let embedded = start(tmp.path()).await.unwrap();

        assert!(!crate::auth::token_path(tmp.path()).exists());
        assert!(!tmp.path().join("engine.json").exists());
        embedded.shutdown();
    }

    #[tokio::test]
    async fn the_query_string_carries_what_the_webview_needs() {
        let tmp = tempfile::tempdir().unwrap();
        let embedded = start(tmp.path()).await.unwrap();

        let query = embedded.query();
        assert!(query.contains(&format!("port={}", embedded.handshake().port)));
        assert!(query.contains(&embedded.handshake().token));
        embedded.shutdown();
    }

    /// Two engines in one process would fight over the vault. The port being chosen by the OS is
    /// what keeps a second one from failing to bind instead of failing honestly later.
    #[tokio::test]
    async fn two_engines_get_different_ports() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let one = start(a.path()).await.unwrap();
        let two = start(b.path()).await.unwrap();

        assert_ne!(one.handshake().port, two.handshake().port);
        one.shutdown();
        two.shutdown();
    }

    #[tokio::test]
    async fn the_data_directory_is_created_where_it_was_asked_for() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("sandboxed").join("summo");
        let embedded = start(&home).await.unwrap();

        assert!(
            home.join("vault").exists(),
            "the vault lives under the app's own directory"
        );
        embedded.shutdown();
    }
}
