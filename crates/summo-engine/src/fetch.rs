//! Getting media from a link.
//!
//! Two different problems wearing the same shape. `https://example.com/holp.mp4` is a file behind a
//! URL and needs nothing but a download. A YouTube or Drive page is not a media file at all — it is
//! a document that *describes* where some media is, in a format that changes whenever the site
//! feels like it, and reading it is a whole project that other people maintain.
//!
//! So: download it ourselves when it is a file, and hand it to `yt-dlp` when it is a page. That is
//! the same relationship this crate already has with ffmpeg, for the same three reasons — licensing
//! stays at arm's length, a malformed page crashes a subprocess rather than the daemon holding
//! somebody's meeting, and nobody's installer doubles for a feature they may never use.
//!
//! **What it cannot do, it says.** A page link on a machine with no `yt-dlp` fails with a message
//! naming `yt-dlp`, exactly the way a WebM full of Opus fails with a message naming ffmpeg. The
//! alternative — a generic "could not import" — is the error that makes somebody file a bug.
//!
//! ## Why it lives with the daemon rather than with the decoder
//!
//! `summo-media` decodes; it has no network and should keep none. This needs an HTTP client and
//! the daemon already has one, already async, already configured with the user agent and timeouts
//! that the model downloader learned the hard way on a Vietnamese ISP.
//!
//! ## What this deliberately does not decide
//!
//! Whether you may download a particular video. That is between the user and whoever publishes it,
//! and a tool that refuses on their behalf while another does not is not protecting anyone. What it
//! does do is keep the decision visible: a link import is something a person types, once, about a
//! recording they are looking at.

use std::path::{Path, PathBuf};
use std::process::Command;

use summo_core::{Error, Result};

/// Five seconds to make a connection, matching the model downloader.
///
/// A blocked address on a Vietnamese ISP does not refuse a connection, it swallows it. No overall
/// timeout: a two-hour recording on a slow line is not an error.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("summo/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| Error::Other(format!("cannot build http client: {e}")))
}

/// Whether a string is a link this module can be asked about at all.
///
/// Deliberately narrow. `http` and `https` only — a `file://` URL is a path with extra steps, and
/// every other scheme is either a local resource with its own rules or something that should not be
/// reached by pasting it into a text box.
///
/// One predicate, in `summo-core`, because the command line has to make the same call before it
/// can decide whether to make a path absolute.
pub use summo_core::media::looks_like_link as is_link;

/// A name for what came back, from the URL.
///
/// The last path segment without its query, cleaned of anything that is not a plain character —
/// this becomes a file name, and a URL is user input. Falls back to `download`, because a name is a
/// convenience and failing an import over one would not be.
#[must_use]
pub fn name_from(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    // From the path, which the host is not. `https://example.com` has nothing in it to name a
    // download after, and calling the file `example.com` would be naming it after the wrong thing.
    let path = without_query
        .split_once("://")
        .map_or(without_query, |(_, rest)| {
            rest.split_once('/').map_or("", |(_, path)| path)
        })
        .trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

/// `yt-dlp`, if this machine has one.
///
/// Located the same way ffmpeg is, with the same override, so a machine that keeps its tools
/// somewhere unusual configures both the same way.
fn yt_dlp() -> PathBuf {
    std::env::var_os("SUMMO_YT_DLP")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("yt-dlp"))
}

/// Whether `yt-dlp` can be run.
#[must_use]
pub fn has_yt_dlp() -> bool {
    Command::new(yt_dlp())
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// What a link turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A media file to download.
    Media,
    /// A page that describes where some media is.
    Page,
    /// The server says there is nothing there.
    Gone(u16),
}

/// Decide what a link is, from what the server says it is.
///
/// A `HEAD` rather than a guess from the extension: plenty of direct media URLs end in a hash, and
/// plenty of page URLs end in `.mp4` without being one. When the server will not answer a `HEAD` at
/// all — some will not — the extension is the fallback, because a wrong guess there costs one
/// attempt and not the import.
///
/// The status comes first, and it matters. A missing file answers 404 with an HTML error page, and
/// reading only the content type made that "this is a page, install yt-dlp" — sending somebody to
/// install a program because they mistyped a URL.
async fn classify(url: &str, client: &reqwest::Client) -> Kind {
    if let Ok(response) = client.head(url).send().await {
        let status = response.status();
        // Nothing there. Not a page, whatever the error page is made of.
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
            return Kind::Gone(status.as_u16());
        }
        if status.is_success() {
            let kind = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if kind.starts_with("video/") || kind.starts_with("audio/") {
                return Kind::Media;
            }
            // An answer, and it is not media. Trust it over the extension.
            if kind.starts_with("text/") || kind.contains("html") {
                return Kind::Page;
            }
        }
        // 403, 405, a redirect it would not follow: the server will not tell us, so guess.
    }
    if summo_media::looks_importable(Path::new(&name_from(url))) {
        Kind::Media
    } else {
        Kind::Page
    }
}

/// Fetch whatever is at `url` into `dir`, and return the file.
///
/// # Errors
///
/// When the link cannot be reached, when it is a page and this machine has no `yt-dlp`, or when
/// either of those produces nothing.
pub async fn fetch(url: &str, dir: &Path) -> Result<PathBuf> {
    if !is_link(url) {
        return Err(Error::msg(
            "fetch.not_a_link",
            format!("`{url}` is not an http or https link"),
        ));
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;

    let client = client()?;
    match classify(url, &client).await {
        Kind::Media => direct(url, dir, &client).await,
        Kind::Page => page(url, dir),
        Kind::Gone(status) => Err(Error::msg(
            "fetch.gone",
            format!("{url} trả về {status} — không có gì ở địa chỉ đó"),
        )),
    }
}

/// A media file behind a URL: download it.
async fn direct(url: &str, dir: &Path, client: &reqwest::Client) -> Result<PathBuf> {
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| Error::msg("fetch.unreachable", format!("không tải được {url}: {e}")))?;

    // The extension the server reports, when the URL did not carry one. A file called `watch` is
    // one ffmpeg and the built-in decoder both have to sniff, and `looks_importable` would refuse
    // it before either got the chance.
    let mut name = name_from(url);
    if !name.contains('.') {
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if let Some(extension) = extension_for(&kind) {
            name.push('.');
            name.push_str(extension);
        }
    }

    let target = dir.join(&name);
    // Streamed to disk rather than buffered: a recording is the one thing being fetched here, and
    // holding a two-hour meeting in memory to write it out again is how a daemon gets killed.
    let mut file = tokio::fs::File::create(&target)
        .await
        .map_err(|e| Error::io(&target, e))?;
    let mut stream = response.bytes_stream();
    let mut size = 0u64;
    {
        use futures::StreamExt as _;
        use tokio::io::AsyncWriteExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| Error::msg("fetch.unreachable", format!("{url}: {e}")))?;
            size += chunk.len() as u64;
            file.write_all(&chunk)
                .await
                .map_err(|e| Error::io(&target, e))?;
        }
        file.flush().await.map_err(|e| Error::io(&target, e))?;
    }

    if size == 0 {
        let _ = std::fs::remove_file(&target);
        return Err(Error::msg(
            "fetch.empty",
            format!("{url} trả về một file rỗng"),
        ));
    }
    Ok(target)
}

/// The usual extension for a content type, for a URL that did not carry one.
fn extension_for(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/quicktime" => Some("mov"),
        "video/x-matroska" => Some("mkv"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/ogg" => Some("ogg"),
        "audio/flac" => Some("flac"),
        _ => None,
    }
}

/// A page that describes where some media is: ask `yt-dlp`.
fn page(url: &str, dir: &Path) -> Result<PathBuf> {
    if !has_yt_dlp() {
        return Err(Error::msg(
            "fetch.no_yt_dlp",
            format!(
                "{url} không phải là file media trực tiếp. Cài `yt-dlp` để nhập từ trang web, \
                 hoặc tải file về rồi nhập file đó."
            ),
        ));
    }

    // Before: nothing in `dir`. After: whatever `yt-dlp` wrote. Reading the directory is more
    // reliable than parsing the tool's output, which changes between versions and localises.
    let before = listing(dir);
    let output = Command::new(yt_dlp())
        .arg("--no-playlist")
        .arg("--no-progress")
        // Audio is all the recogniser needs and a fraction of the bytes, but a video is what
        // somebody wants to watch back — so take the smallest complete thing rather than the best.
        .arg("-f")
        .arg("best[height<=720]/best")
        .arg("-o")
        .arg(dir.join("%(title).80s.%(ext)s"))
        .arg(url)
        .output()
        .map_err(|e| Error::msg("fetch.yt_dlp", format!("không chạy được yt-dlp: {e}")))?;

    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        let why = why.lines().last().unwrap_or("yt-dlp failed").trim();
        return Err(Error::msg(
            "fetch.yt_dlp",
            format!("yt-dlp không tải được {url}: {why}"),
        ));
    }

    listing(dir)
        .into_iter()
        .find(|path| !before.contains(path))
        .ok_or_else(|| {
            Error::msg(
                "fetch.yt_dlp",
                format!("yt-dlp báo thành công nhưng không để lại file nào cho {url}"),
            )
        })
}

fn listing(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name becomes a file name, and a URL is user input.
    #[test]
    fn a_name_from_a_url_cannot_be_a_path() {
        assert_eq!(name_from("https://x.com/a/holp.mp4"), "holp.mp4");
        assert_eq!(name_from("https://x.com/a/holp.mp4?token=1"), "holp.mp4");
        assert_eq!(name_from("https://x.com/a/holp.mp4#t=10"), "holp.mp4");
        assert_eq!(name_from("https://x.com/..%2f..%2fetc/passwd"), "passwd");
        assert_eq!(name_from("https://x.com/a/../../etc/passwd"), "passwd");
        assert!(!name_from("https://x.com/a/../b.mp4").contains('/'));
    }

    #[test]
    fn a_url_with_nothing_to_name_it_after_still_gets_a_name() {
        assert_eq!(name_from("https://x.com/"), "download");
        assert_eq!(name_from("https://x.com"), "download");
        assert_eq!(name_from("https://x.com/watch?v=abc"), "watch");
    }

    #[test]
    fn a_content_type_supplies_the_extension_a_url_did_not_carry() {
        assert_eq!(extension_for("video/mp4"), Some("mp4"));
        assert_eq!(extension_for("video/mp4; codecs=avc1"), Some("mp4"));
        assert_eq!(extension_for("Audio/MPEG"), Some("mp3"));
        assert_eq!(extension_for("text/html"), None);
    }

    #[tokio::test]
    async fn a_path_is_refused_as_a_link_rather_than_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let err = fetch("/home/me/a.mp4", dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not an http or https link"), "{err}");
    }
}
