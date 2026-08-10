//! Tokens for the few model hosts that demand one.
//!
//! Almost nothing Summo downloads needs a credential. CAM++, ERes2NetV2, Silero and the ASR models
//! are all public. But pyannote's checkpoints are *gated*: MIT-licensed, yet served only to accounts
//! that have accepted their conditions, which means an access token.
//!
//! Two rules govern this module, and both exist because a token is not the same kind of data as a
//! setting.
//!
//! **A token is never written to a settings file.** It comes from the environment, or from a
//! `600`-mode file the user controls. `settings.json` ends up in backups, sync folders and support
//! bundles; a credential that lives there leaks by being copied, without anybody making a mistake.
//! This mirrors the rule already enforced for LLM API keys in `summo_core::settings`.
//!
//! **A token is only ever sent to a host on an allowlist.** Manifests carry mirror URLs, and a
//! manifest is data — it can come from a third-party registry. Attaching `Authorization` to whatever
//! URL a manifest names would turn "add a mirror" into "harvest the user's HuggingFace token". So
//! the header is attached per-host, by [`Credentials::header_for`], and an unknown host gets
//! nothing.

use std::path::{Path, PathBuf};

/// Hosts permitted to receive the HuggingFace token.
///
/// Deliberately exact matches, not suffixes: `huggingface.co.evil.test` must not qualify.
const HUGGINGFACE_HOSTS: [&str; 2] = ["huggingface.co", "cdn-lfs.huggingface.co"];

/// Environment variable read for the HuggingFace token, in order of preference.
///
/// `HF_TOKEN` is what the `huggingface_hub` tooling uses, so a machine already set up for Python
/// work needs no extra configuration.
const HF_ENV: [&str; 2] = ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"];

/// Resolved credentials for a download session.
#[derive(Clone, Default)]
pub struct Credentials {
    huggingface: Option<String>,
}

impl Credentials {
    /// Nothing configured: every download proceeds anonymously.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Resolve from the environment, then from files under `home`.
    ///
    /// Order, first hit wins:
    ///
    /// 1. `HF_TOKEN` / `HUGGING_FACE_HUB_TOKEN` in the environment
    /// 2. `<home>/hf_token` — Summo's own location
    /// 3. `~/.cache/huggingface/token` — written by `huggingface-cli login`
    ///
    /// A blank or whitespace-only value counts as absent, so `HF_TOKEN=` in a shell profile does not
    /// shadow a token that is present on disk.
    #[must_use]
    pub fn discover(home: &Path) -> Self {
        Self {
            huggingface: hf_from_env().or_else(|| hf_from_files(home)),
        }
    }

    /// Set the HuggingFace token explicitly. Blank input clears it.
    #[must_use]
    pub fn with_huggingface(mut self, token: impl Into<String>) -> Self {
        self.huggingface = non_blank(token.into());
        self
    }

    #[must_use]
    pub fn has_huggingface(&self) -> bool {
        self.huggingface.is_some()
    }

    /// The `Authorization` value to send to `url`, if any.
    ///
    /// Returns `None` for every host not on the allowlist, including when a token is configured.
    /// That is the point: a mirror URL cannot talk the downloader into revealing the credential.
    #[must_use]
    pub fn header_for(&self, url: &str) -> Option<String> {
        let host = host_of(url)?;
        if HUGGINGFACE_HOSTS.contains(&host.as_str()) {
            return self.huggingface.as_ref().map(|t| format!("Bearer {t}"));
        }
        None
    }
}

/// Deliberately opaque: a token must not reach a log line through a stray `{:?}`.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field(
                "huggingface",
                &if self.huggingface.is_some() {
                    "<set>"
                } else {
                    "<unset>"
                },
            )
            .finish()
    }
}

fn hf_from_env() -> Option<String> {
    HF_ENV
        .iter()
        .find_map(|key| std::env::var(key).ok().and_then(non_blank))
}

fn hf_from_files(home: &Path) -> Option<String> {
    let candidates = [
        home.join("hf_token"),
        dirs_cache_token().unwrap_or_else(|| PathBuf::from("/nonexistent")),
    ];
    candidates
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().and_then(non_blank))
}

fn dirs_cache_token() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("huggingface")
            .join("token"),
    )
}

fn non_blank(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Host of an absolute http(s) URL, lowercased, without port or userinfo.
///
/// Hand-rolled rather than pulling in a URL parser: the downloader only ever sees absolute URLs, and
/// the failure mode that matters is returning `None` — which denies the header — rather than
/// guessing a host.
fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // `user:pass@host` — the host is what follows the last `@`.
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials::none().with_huggingface("hf_secret")
    }

    #[test]
    fn allowlisted_hosts_get_the_header() {
        let c = creds();
        assert_eq!(
            c.header_for("https://huggingface.co/pyannote/segmentation-3.0/resolve/main/x.bin"),
            Some("Bearer hf_secret".to_string())
        );
        assert_eq!(
            c.header_for("https://cdn-lfs.huggingface.co/repos/ab/cd/blob"),
            Some("Bearer hf_secret".to_string())
        );
    }

    /// The reason this module exists: a manifest is data, and a mirror is a URL from that data.
    #[test]
    fn other_hosts_never_get_the_header() {
        let c = creds();
        for url in [
            "https://cdn.summo.app/blobs/sha256/abc",
            "https://evil.test/collect",
            "http://localhost:8080/model.onnx",
            "https://modelscope.cn/models/x",
        ] {
            assert_eq!(c.header_for(url), None, "leaked to {url}");
        }
    }

    /// A lookalike host must not pass. Suffix matching would have let this through.
    #[test]
    fn lookalike_hosts_are_rejected() {
        let c = creds();
        for url in [
            "https://huggingface.co.evil.test/x",
            "https://nothuggingface.co/x",
            "https://huggingface.com/x",
            "https://evil.test/https://huggingface.co/x",
        ] {
            assert_eq!(c.header_for(url), None, "leaked to {url}");
        }
    }

    /// `https://user:pass@evil.test/` parses to `evil.test`, not to whatever precedes the `@`.
    #[test]
    fn userinfo_does_not_forge_a_host() {
        let c = creds();
        assert_eq!(c.header_for("https://huggingface.co@evil.test/x"), None);
    }

    #[test]
    fn no_token_means_no_header_anywhere() {
        let c = Credentials::none();
        assert!(!c.has_huggingface());
        assert_eq!(c.header_for("https://huggingface.co/x"), None);
    }

    #[test]
    fn blank_tokens_count_as_absent() {
        assert!(!Credentials::none().with_huggingface("   ").has_huggingface());
        assert!(!Credentials::none().with_huggingface("").has_huggingface());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        // Tokens arrive from files, and a file written by `echo` ends in a newline.
        let c = Credentials::none().with_huggingface("hf_abc\n");
        assert_eq!(
            c.header_for("https://huggingface.co/x"),
            Some("Bearer hf_abc".to_string())
        );
    }

    #[test]
    fn a_token_file_under_home_is_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("hf_token"), "hf_from_file\n").expect("write");
        // Not asserting on env precedence here: the ambient environment is shared between tests.
        let found = hf_from_files(dir.path());
        assert_eq!(found, Some("hf_from_file".to_string()));
    }

    #[test]
    fn debug_never_prints_the_token() {
        let rendered = format!("{:?}", creds());
        assert!(!rendered.contains("hf_secret"), "token leaked: {rendered}");
        assert!(rendered.contains("<set>"));
    }
}
