//! `summo import` — hand recordings to the daemon and watch them land.
//!
//! An earlier version of this command did the work itself: it found ffmpeg, extracted a WAV under a
//! freshly minted meeting id, and stopped. That left a file in `~/.summo/audio/<id>/` belonging to
//! a meeting that was never written — audio the storage screen counts and nothing can play. The
//! job it told the user to run next did not exist either.
//!
//! So the CLI no longer transcribes. It finds the daemon, posts the file, and follows the job. One
//! implementation of import, shared with the app, and no half-imported state when it fails.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use summo_core::paths::Paths;

/// What the daemon publishes when it starts.
#[derive(Debug, Deserialize)]
pub struct Handshake {
    pub port: u16,
    pub token: String,
}

/// Find the running daemon, or explain how to start one.
pub fn handshake(paths: &Paths) -> Result<Handshake> {
    let path = paths.root().join("engine.json");
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "không thấy daemon đang chạy ({}). Mở app Summo, hoặc chạy `summo-engine`.",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).with_context(|| format!("{} hỏng", path.display()))
}

#[derive(Debug, Deserialize)]
pub struct Job {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub done_s: Option<f64>,
    #[serde(default)]
    pub total_s: Option<f64>,
    #[serde(default)]
    pub segments: Option<usize>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl Job {
    #[must_use]
    pub fn finished(&self) -> bool {
        self.state == "done" || self.state == "failed"
    }

    /// One line for a terminal that is being watched.
    #[must_use]
    pub fn line(&self) -> String {
        match self.state.as_str() {
            "queued" => "đang chờ".into(),
            "extracting" => "đang tách âm thanh".into(),
            "running" => match (self.done_s, self.total_s) {
                (Some(done), Some(total)) if total > 0.0 => {
                    let pct = ((done / total) * 100.0).clamp(0.0, 100.0);
                    format!("{pct:.0}% — {} câu", self.segments.unwrap_or(0))
                }
                _ => format!("đang nhận dạng — {} câu", self.segments.unwrap_or(0)),
            },
            "done" => format!(
                "xong — {} câu → {}",
                self.segments.unwrap_or(0),
                self.path.as_deref().unwrap_or("?")
            ),
            "failed" => format!("lỗi — {}", self.error.as_deref().unwrap_or("không rõ")),
            other => other.into(),
        }
    }
}

fn url(handshake: &Handshake, path: &str) -> String {
    format!("http://127.0.0.1:{}{path}", handshake.port)
}

async fn read_job(response: reqwest::Response) -> Result<Job> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        // The daemon's own message is far more useful than the status code — "chưa cài mô hình
        // nhận dạng nào" tells the user what to do; "400" does not.
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(body);
        bail!("{message}");
    }
    serde_json::from_str(&body).with_context(|| format!("phản hồi lạ từ daemon: {body}"))
}

/// Queue one file and return the job.
pub async fn start(
    client: &reqwest::Client,
    handshake: &Handshake,
    file: &Path,
    language: Option<&str>,
) -> Result<Job> {
    // An absolute path, because the daemon's working directory is not the shell's and a relative
    // one would silently resolve somewhere else.
    let absolute = std::fs::canonicalize(file)
        .with_context(|| format!("không thấy {}", file.display()))?;

    let mut body = serde_json::json!({ "path": absolute.to_string_lossy() });
    if let Some(language) = language {
        body["language"] = serde_json::Value::String(language.to_string());
    }

    let response = client
        .post(url(handshake, "/imports"))
        .bearer_auth(&handshake.token)
        .json(&body)
        .send()
        .await
        .context("không gọi được daemon")?;
    read_job(response).await
}

/// Ask how one job is doing.
pub async fn poll(client: &reqwest::Client, handshake: &Handshake, id: &str) -> Result<Job> {
    let response = client
        .get(url(handshake, &format!("/imports/{id}")))
        .bearer_auth(&handshake.token)
        .send()
        .await
        .context("không gọi được daemon")?;
    read_job(response).await
}

/// Files to import from a path that may be a file or a folder.
pub fn targets(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_dir() {
        Ok(summo_media::importable_in(path)?)
    } else {
        Ok(vec![path.to_path_buf()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: &str) -> Job {
        Job {
            id: "j".into(),
            title: "t".into(),
            state: state.into(),
            done_s: None,
            total_s: None,
            segments: None,
            path: None,
            error: None,
        }
    }

    #[test]
    fn a_failure_counts_as_finished_so_the_wait_loop_ends() {
        assert!(job("failed").finished());
        assert!(job("done").finished());
        assert!(!job("running").finished());
        assert!(!job("queued").finished());
    }

    #[test]
    fn progress_is_shown_as_a_percentage_once_the_length_is_known() {
        let mut j = job("running");
        j.done_s = Some(30.0);
        j.total_s = Some(120.0);
        j.segments = Some(7);
        assert_eq!(j.line(), "25% — 7 câu");
    }

    /// A file whose duration ffprobe could not report must still show movement, or a long import
    /// reads as a hang.
    #[test]
    fn an_unknown_length_still_reports_the_sentences_found() {
        let mut j = job("running");
        j.segments = Some(3);
        assert_eq!(j.line(), "đang nhận dạng — 3 câu");
    }

    #[test]
    fn a_failure_shows_the_daemon_s_message() {
        let mut j = job("failed");
        j.error = Some("không có âm thanh".into());
        assert!(j.line().contains("không có âm thanh"));
    }

    #[test]
    fn a_failure_with_no_message_still_says_it_failed() {
        assert!(job("failed").line().contains("lỗi"));
    }

    #[test]
    fn a_missing_daemon_says_how_to_start_one() {
        let dir = tempfile::tempdir().unwrap();
        let err = handshake(&Paths::at(dir.path())).unwrap_err().to_string();
        assert!(err.contains("Summo") || err.contains("summo-engine"), "{err}");
    }

    #[test]
    fn a_corrupt_handshake_is_reported_as_corrupt_rather_than_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("engine.json"), "{ not json").unwrap();
        let err = handshake(&Paths::at(dir.path())).unwrap_err().to_string();
        assert!(err.contains("hỏng"), "{err}");
    }

    #[test]
    fn a_single_file_is_its_own_target_list() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(targets(&file).unwrap(), vec![file]);
    }
}
