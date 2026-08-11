//! Keeping other programs off the daemon.
//!
//! A daemon listening on loopback is not private. Any web page the user visits can open
//! `ws://127.0.0.1:<port>` from JavaScript, and browsers do not apply the same-origin policy to
//! WebSocket connections the way they do to `fetch`. Without a check, a page left open in a
//! background tab could start a recording and stream the user's meeting to itself — using exactly
//! the local-first architecture that was supposed to protect them.
//!
//! Three defences, each covering what the others miss:
//!
//! 1. **Bind to loopback only**, so nothing off the machine can reach the port at all.
//! 2. **A bearer token** generated per launch and written to a file only the user can read. A web
//!    page cannot read that file, so it cannot present the token.
//! 3. **Reject browser origins.** A page cannot forge the `Origin` header on a WebSocket, so
//!    refusing any request that carries a web origin blocks the attack even if a token leaks.

use std::path::{Path, PathBuf};

use summo_core::{Error, Result};

/// Bytes of entropy in a session token.
const TOKEN_BYTES: usize = 32;

/// A per-launch credential for the local socket.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Generate a fresh token from the operating system's cryptographic random source.
    ///
    /// An explicit CSPRNG rather than a convenience crate that happens to use one underneath: this
    /// is the only thing standing between a web page and the user's microphone, and "probably
    /// random enough" is not a property worth inheriting by accident.
    ///
    /// # Panics
    /// If the OS random source is unavailable. There is no safe way to continue — a predictable
    /// token would be worse than no daemon at all.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).expect("the operating system random source must be available");
        Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison.
    ///
    /// `==` on strings returns as soon as two bytes differ, which leaks how much of a guess was
    /// correct. Over a loopback socket an attacker can make a great many attempts.
    #[must_use]
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let actual = candidate.as_bytes();
        if expected.len() != actual.len() {
            return false;
        }
        let mut diff = 0_u8;
        for (a, b) in expected.iter().zip(actual) {
            diff |= a ^ b;
        }
        diff == 0
    }

    /// Write the token where the app can read it, readable only by this user.
    ///
    /// The permissions are the whole point: a web page cannot read a file, so a token on disk is
    /// safe from the browser in a way that a token on a well-known port is not.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(path, &self.0).map_err(|e| Error::io(path, e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| Error::io(path, e))?;
        }
        Ok(())
    }

    /// Read a token the daemon previously wrote.
    pub fn read_from(path: &Path) -> Result<Self> {
        let body = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let trimmed = body.trim();
        if trimmed.len() != TOKEN_BYTES * 2 {
            return Err(Error::Config(format!(
                "{} does not contain a valid session token",
                path.display()
            )));
        }
        Ok(Self(trimmed.to_string()))
    }
}

// Never print the token: it would end up in logs, crash reports and support bundles.
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(redacted)")
    }
}

/// Where the daemon publishes its port and token.
#[must_use]
pub fn token_path(root: &Path) -> PathBuf {
    root.join("engine.token")
}

/// Whether a request's `Origin` header should be refused.
///
/// A native app sends no origin. A browser always sends one and cannot forge it, so an origin at
/// all means a web page is calling — and the only reason a web page would talk to this daemon is to
/// do something the user did not ask for.
///
/// `allow_loopback` widens this to pages served from loopback, which is how the interface is
/// developed and how it is driven in a browser test. It is off unless the daemon was started with
/// `--dev`, because the protection it removes is the entire reason this check exists: a page on the
/// open internet must never reach the microphone, and "it is only for development" is how that ends
/// up shipped.
#[must_use]
pub fn origin_is_allowed(origin: Option<&str>, allow_loopback: bool) -> bool {
    let Some(origin) = origin else {
        return true;
    };

    // Tauri's webview identifies itself with these schemes rather than an http origin.
    if origin.starts_with("tauri://")
        || origin.starts_with("summo://")
        || origin.starts_with("http://tauri.localhost")
    {
        return true;
    }

    allow_loopback && is_loopback_origin(origin)
}

/// Whether an origin names this machine.
///
/// Matched on the host alone. A remote host is refused even in development, so a page at
/// `https://evil.example` cannot reach a developer's daemon while they have it open.
fn is_loopback_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(name, _)| name);

    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_unique() {
        let a = SessionToken::generate();
        let b = SessionToken::generate();
        assert_eq!(a.as_str().len(), TOKEN_BYTES * 2);
        assert_ne!(a.as_str(), b.as_str(), "each launch must get a fresh token");
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn matching_accepts_only_the_exact_token() {
        let token = SessionToken::generate();
        assert!(token.matches(token.as_str()));
        assert!(!token.matches("wrong"));
        assert!(!token.matches(""));

        // A correct prefix must not pass, or an attacker could extend a guess one byte at a time.
        //
        // The last character has to be replaced with a *different* one. Hard-coding a replacement
        // is wrong once every sixteen runs: when the token already ends in that character, the
        // "almost" token is the real one, and the test asserts a valid credential is rejected.
        let prefix = &token.as_str()[..TOKEN_BYTES * 2 - 1];
        let last = token
            .as_str()
            .chars()
            .next_back()
            .expect("a token is not empty");
        let almost = format!("{prefix}{}", if last == '0' { '1' } else { '0' });
        assert_ne!(almost, token.as_str(), "the near miss must actually differ");
        assert!(!token.matches(&almost));
    }

    #[test]
    fn the_token_never_appears_in_debug_output() {
        // Debug formatting reaches logs, panics and crash reports.
        let token = SessionToken::generate();
        let printed = format!("{token:?}");
        assert!(!printed.contains(token.as_str()), "token leaked: {printed}");
        assert!(printed.contains("redacted"));
    }

    #[test]
    fn tokens_round_trip_through_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = token_path(tmp.path());
        let token = SessionToken::generate();

        token.write_to(&path).unwrap();
        let read = SessionToken::read_from(&path).unwrap();
        assert!(token.matches(read.as_str()));
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_not_readable_by_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = token_path(tmp.path());
        SessionToken::generate().write_to(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "token file is group- or world-accessible: {mode:o}"
        );
    }

    #[test]
    fn a_corrupt_token_file_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = token_path(tmp.path());
        std::fs::write(&path, "short").unwrap();
        assert!(SessionToken::read_from(&path).is_err());
    }

    #[test]
    fn requests_from_a_web_page_are_refused_by_default() {
        // The attack this blocks: a page in a background tab opening ws://127.0.0.1 and recording.
        for origin in [
            "https://evil.example",
            "http://localhost:3000",
            "http://127.0.0.1:8080",
            "null",
        ] {
            assert!(
                !origin_is_allowed(Some(origin), false),
                "origin `{origin}` should have been refused"
            );
        }
    }

    #[test]
    fn native_and_webview_clients_are_allowed() {
        assert!(
            origin_is_allowed(None, false),
            "a native client sends no origin"
        );
        assert!(origin_is_allowed(Some("tauri://localhost"), false));
        assert!(origin_is_allowed(Some("http://tauri.localhost"), false));
    }

    #[test]
    fn development_mode_admits_a_page_served_from_this_machine() {
        for origin in [
            "http://localhost:5173",
            "http://127.0.0.1:8903",
            "http://[::1]:5173",
        ] {
            assert!(
                origin_is_allowed(Some(origin), true),
                "origin `{origin}` should be allowed in development"
            );
        }
    }

    #[test]
    fn development_mode_still_refuses_a_remote_page() {
        // Widening this to any origin would mean a developer with the daemon running could be
        // recorded by any site they visit — the exact attack the check exists for.
        for origin in [
            "https://evil.example",
            "http://evil.example:5173",
            "null",
            "http://127.0.0.1.evil.example",
            "http://notlocalhost",
        ] {
            assert!(
                !origin_is_allowed(Some(origin), true),
                "origin `{origin}` must be refused even in development"
            );
        }
    }
}
