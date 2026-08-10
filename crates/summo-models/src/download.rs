//! Resumable, verified file transfer.
//!
//! Model files run from tens of megabytes to several gigabytes over connections that drop, so three
//! properties are non-negotiable:
//!
//! * **Resume.** Partial transfers land in a staging directory and continue with a `Range` request
//!   instead of starting over.
//! * **Verify.** The digest is computed over the bytes actually on disk, and a mismatch is fatal —
//!   a corrupt model produces silently wrong transcripts, which is worse than a failed download.
//! * **Mirrors.** Each file carries fallback URLs, so a model stays installable even if our CDN is
//!   unavailable. This is what keeps the open-source app independent of the paid service.

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    time::Duration,
};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use summo_core::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::credentials::Credentials;
use crate::manifest::FileEntry;

/// Bytes hashed per read when verifying an existing partial file.
const HASH_CHUNK: usize = 1024 * 1024;

/// Progress callback payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub done: u64,
    pub total: u64,
    /// True while re-hashing bytes already on disk rather than transferring new ones.
    pub resuming: bool,
}

impl DownloadProgress {
    #[must_use]
    pub fn pct(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        u8::try_from((self.done * 100 / self.total).min(100)).unwrap_or(100)
    }
}

/// Fetches manifest files into a staging directory, then promotes them to their final path.
pub struct Downloader {
    client: reqwest::Client,
    staging: PathBuf,
    max_retries: u32,
    credentials: Credentials,
}

impl Downloader {
    /// `staging` holds partial transfers. It must be on the same filesystem as the blob store so
    /// promotion is an atomic rename rather than a copy.
    pub fn new(staging: impl Into<PathBuf>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("summo/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            // No overall request timeout: a multi-gigabyte model on a slow line is not an error.
            .build()
            .map_err(|e| Error::Registry(format!("cannot build http client: {e}")))?;
        Ok(Self {
            client,
            staging: staging.into(),
            max_retries: 3,
            credentials: Credentials::none(),
        })
    }

    #[must_use]
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Supply tokens for gated hosts.
    ///
    /// The downloader does not decide which requests get a credential; [`Credentials::header_for`]
    /// does, per URL, against a host allowlist. A manifest naming an arbitrary mirror therefore
    /// cannot cause the token to be sent anywhere it does not belong.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = credentials;
        self
    }

    /// Fetch `entry` to `dest`, resuming and verifying.
    ///
    /// Returns immediately if `dest` already exists — the blob store is content-addressed, so an
    /// existing file at that path is already the right bytes.
    pub async fn fetch<F>(&self, entry: &FileEntry, dest: &Path, mut on_progress: F) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        if tokio::fs::try_exists(dest).await.unwrap_or(false) {
            on_progress(DownloadProgress {
                done: entry.size,
                total: entry.size,
                resuming: false,
            });
            return Ok(());
        }

        tokio::fs::create_dir_all(&self.staging)
            .await
            .map_err(|e| Error::io(&self.staging, e))?;
        let partial = self.staging.join(format!("{}.part", entry.sha256));

        let mut last_err = None;
        let urls: Vec<&String> = std::iter::once(&entry.url).chain(&entry.mirror).collect();

        for url in urls {
            for attempt in 0..=self.max_retries {
                match self.try_one(url, entry, &partial, &mut on_progress).await {
                    Ok(()) => {
                        self.promote(&partial, dest).await?;
                        return Ok(());
                    }
                    Err(e) if e.is_transient() && attempt < self.max_retries => {
                        let backoff = Duration::from_millis(500 * u64::from(attempt + 1));
                        tracing::warn!(url, attempt, error = %e, "download failed, retrying");
                        tokio::time::sleep(backoff).await;
                    }
                    Err(e) => {
                        // A checksum mismatch means the partial file is poisoned; drop it so the
                        // next source starts clean instead of resuming onto bad bytes.
                        if matches!(e, Error::ChecksumMismatch { .. }) {
                            tokio::fs::remove_file(&partial).await.ok();
                        }
                        tracing::warn!(url, error = %e, "source failed, trying next mirror");
                        last_err = Some(e);
                        break;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| Error::Download {
            url: entry.url.clone(),
            reason: "no source succeeded".into(),
        }))
    }

    async fn try_one<F>(
        &self,
        url: &str,
        entry: &FileEntry,
        partial: &Path,
        on_progress: &mut F,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        if let Some(path) = url.strip_prefix("file://") {
            return self
                .copy_local(Path::new(path), entry, partial, on_progress)
                .await;
        }

        // Re-hash whatever survived a previous attempt so we can resume from the right offset.
        let (mut hasher, mut done) = resume_state(partial, on_progress, entry.size).await?;

        if done > entry.size {
            // Longer than declared: the partial cannot be a prefix of the real file.
            tokio::fs::remove_file(partial).await.ok();
            hasher = Sha256::new();
            done = 0;
        }

        if done < entry.size {
            let mut req = self.client.get(url);
            if done > 0 {
                req = req.header(reqwest::header::RANGE, format!("bytes={done}-"));
            }
            if let Some(auth) = self.credentials.header_for(url) {
                req = req.header(reqwest::header::AUTHORIZATION, auth);
            }
            let resp = req.send().await.map_err(|e| Error::Download {
                url: url.to_string(),
                reason: e.to_string(),
            })?;

            let status = resp.status();
            if !status.is_success() {
                return Err(Error::Download {
                    url: url.to_string(),
                    reason: format!("http {status}"),
                });
            }
            // A server that ignores `Range` replies 200 with the whole file; start over rather than
            // appending a second copy.
            if done > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
                tokio::fs::remove_file(partial).await.ok();
                hasher = Sha256::new();
                done = 0;
            }

            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                // Keep existing bytes: we seek to `done` and append to a resumed transfer.
                .truncate(false)
                .open(partial)
                .await
                .map_err(|e| Error::io(partial, e))?;
            file.seek(SeekFrom::Start(done))
                .await
                .map_err(|e| Error::io(partial, e))?;

            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| Error::Download {
                    url: url.to_string(),
                    reason: e.to_string(),
                })?;
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|e| Error::io(partial, e))?;
                done += chunk.len() as u64;
                on_progress(DownloadProgress {
                    done,
                    total: entry.size,
                    resuming: false,
                });
            }
            file.flush().await.map_err(|e| Error::io(partial, e))?;
        }

        verify(hasher, entry, partial).await
    }

    async fn copy_local<F>(
        &self,
        src: &Path,
        entry: &FileEntry,
        partial: &Path,
        on_progress: &mut F,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        tokio::fs::copy(src, partial)
            .await
            .map_err(|e| Error::io(src, e))?;
        let (hasher, done) = resume_state(partial, on_progress, entry.size).await?;
        on_progress(DownloadProgress {
            done,
            total: entry.size,
            resuming: false,
        });
        verify(hasher, entry, partial).await
    }

    async fn promote(&self, partial: &Path, dest: &Path) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::io(parent, e))?;
        }
        match tokio::fs::rename(partial, dest).await {
            Ok(()) => Ok(()),
            // Staging on a different filesystem: fall back to copy + remove.
            Err(_) => {
                tokio::fs::copy(partial, dest)
                    .await
                    .map_err(|e| Error::io(dest, e))?;
                tokio::fs::remove_file(partial).await.ok();
                Ok(())
            }
        }
    }
}

/// Hash the bytes already on disk, returning the hasher state and byte count.
async fn resume_state<F>(partial: &Path, on_progress: &mut F, total: u64) -> Result<(Sha256, u64)>
where
    F: FnMut(DownloadProgress) + Send,
{
    let mut hasher = Sha256::new();
    let mut done = 0_u64;

    let Ok(mut file) = tokio::fs::File::open(partial).await else {
        return Ok((hasher, 0));
    };
    let mut buf = vec![0_u8; HASH_CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| Error::io(partial, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        on_progress(DownloadProgress {
            done,
            total,
            resuming: true,
        });
    }
    Ok((hasher, done))
}

async fn verify(hasher: Sha256, entry: &FileEntry, partial: &Path) -> Result<()> {
    let actual = hex::encode(hasher.finalize());
    if actual != entry.sha256 {
        tokio::fs::remove_file(partial).await.ok();
        return Err(Error::ChecksumMismatch {
            file: entry.name.clone(),
            expected: entry.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_for(bytes: &[u8], url: String) -> FileEntry {
        FileEntry {
            name: "test.bin".into(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
            url,
            mirror: Vec::new(),
            platform: None,
        }
    }

    #[tokio::test]
    async fn local_file_urls_copy_and_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        let payload = b"summo model bytes".repeat(100);
        tokio::fs::write(&src, &payload).await.unwrap();

        let entry = entry_for(&payload, format!("file://{}", src.display()));
        let dl = Downloader::new(tmp.path().join("staging")).unwrap();
        let dest = tmp.path().join("out.bin");
        dl.fetch(&entry, &dest, |_| {}).await.unwrap();

        assert_eq!(tokio::fs::read(&dest).await.unwrap(), payload);
    }

    #[tokio::test]
    async fn corrupt_bytes_are_rejected_and_not_promoted() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        tokio::fs::write(&src, b"actual bytes").await.unwrap();

        // Manifest claims a different digest than the file really has.
        let mut entry = entry_for(b"expected bytes", format!("file://{}", src.display()));
        entry.size = 12;

        let dl = Downloader::new(tmp.path().join("staging")).unwrap();
        let dest = tmp.path().join("out.bin");
        let err = dl.fetch(&entry, &dest, |_| {}).await.unwrap_err();

        assert!(matches!(err, Error::ChecksumMismatch { .. }), "got {err:?}");
        assert!(
            !dest.exists(),
            "a failed verification must never be promoted"
        );
    }

    #[tokio::test]
    async fn existing_destination_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.bin");
        tokio::fs::write(&dest, b"already here").await.unwrap();

        // Unreachable URL: proves no request is attempted when the blob already exists.
        let entry = entry_for(b"already here", "https://127.0.0.1:1/nope".into());
        let dl = Downloader::new(tmp.path().join("staging")).unwrap();

        let mut calls = 0;
        dl.fetch(&entry, &dest, |_| calls += 1).await.unwrap();
        assert_eq!(calls, 1, "should report completion without transferring");
    }

    #[tokio::test]
    async fn resume_rehashes_existing_partial_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        tokio::fs::create_dir_all(&staging).await.unwrap();

        let payload = b"resume me".repeat(50);
        let entry = entry_for(&payload, "https://127.0.0.1:1/nope".into());

        // Simulate an interrupted transfer that actually completed on disk.
        tokio::fs::write(staging.join(format!("{}.part", entry.sha256)), &payload)
            .await
            .unwrap();

        let (hasher, done) = resume_state(
            &staging.join(format!("{}.part", entry.sha256)),
            &mut |_| {},
            entry.size,
        )
        .await
        .unwrap();

        assert_eq!(done, entry.size);
        assert_eq!(hex::encode(hasher.finalize()), entry.sha256);
    }

    #[test]
    fn progress_percentage_is_bounded() {
        assert_eq!(
            DownloadProgress {
                done: 0,
                total: 0,
                resuming: false
            }
            .pct(),
            0
        );
        assert_eq!(
            DownloadProgress {
                done: 5,
                total: 10,
                resuming: false
            }
            .pct(),
            50
        );
        assert_eq!(
            DownloadProgress {
                done: 99,
                total: 10,
                resuming: false
            }
            .pct(),
            100
        );
    }
}
