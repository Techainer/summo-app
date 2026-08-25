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
    /// The address that served the last file, tried first for the next one.
    ///
    /// A model is four or five files and they all live in the same three places. Without this,
    /// every file re-pays the connect timeout of the addresses this network cannot reach — five
    /// times the same wait, for a fact learned on the first file.
    working: std::sync::Mutex<Option<String>>,
}

impl Downloader {
    /// `staging` holds partial transfers. It must be on the same filesystem as the blob store so
    /// promotion is an atomic rename rather than a copy.
    pub fn new(staging: impl Into<PathBuf>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("summo/", env!("CARGO_PKG_VERSION")))
            // Five seconds to make a connection, not fifteen. A blocked address on a Vietnamese
            // ISP does not refuse the connection, it swallows it, so this timeout is the entire
            // cost of discovering that an address is unusable — paid once per address, per file.
            .connect_timeout(Duration::from_secs(5))
            // No overall request timeout: a multi-gigabyte model on a slow line is not an error.
            .build()
            .map_err(|e| Error::Registry(format!("cannot build http client: {e}")))?;
        Ok(Self {
            client,
            staging: staging.into(),
            max_retries: 3,
            credentials: Credentials::none(),
            working: std::sync::Mutex::new(None),
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
        let mut urls: Vec<&String> = std::iter::once(&entry.url).chain(&entry.mirror).collect();
        // Whatever served the previous file goes first. On a connection where the first two
        // addresses are black holes, this is the difference between paying ten seconds once and
        // paying it for every file in the model.
        if let Some(known) = self.working.lock().ok().and_then(|slot| slot.clone()) {
            urls.sort_by_key(|url| usize::from(**url != known));
        }
        // Addresses this round gave up on for good — a 404, a checksum mismatch, anything that
        // will still be true in ten seconds. Retrying those is time spent proving the same thing.
        let mut dead = vec![false; urls.len()];

        // Every address once, then every address again. This loop used to be the other way round:
        // four attempts at the first URL before the second was tried at all. Every manifest's `url`
        // is `huggingface.co`, which Vietnamese consumer ISPs block, so the common case was four
        // connect timeouts — a minute of nothing — before reaching a mirror that answers straight
        // away. The retries are for a flaky connection; the mirrors are for a blocked one, and
        // waiting out the first before using the second treats a block as if it were flakiness.
        for round in 0..=self.max_retries {
            for (index, url) in urls.iter().enumerate() {
                if dead[index] {
                    continue;
                }
                match self.try_one(url, entry, &partial, &mut on_progress).await {
                    Ok(()) => {
                        if let Ok(mut slot) = self.working.lock() {
                            *slot = Some((*url).clone());
                        }
                        self.promote(&partial, dest).await?;
                        return Ok(());
                    }
                    Err(e) => {
                        // A checksum mismatch means the partial file is poisoned; drop it so the
                        // next source starts clean instead of resuming onto bad bytes.
                        if matches!(e, Error::ChecksumMismatch { .. }) {
                            tokio::fs::remove_file(&partial).await.ok();
                        }
                        dead[index] = !e.is_transient();
                        tracing::warn!(url, round, error = %e, "source failed, trying the next one");
                        last_err = Some(e);
                    }
                }
            }
            if dead.iter().all(|&gone| gone) {
                break;
            }
            if round < self.max_retries {
                tokio::time::sleep(Duration::from_millis(500 * u64::from(round + 1))).await;
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
            variant: None,
            archive: None,
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

    /// A listener that accepts and hangs up, counting how many times it was asked.
    ///
    /// The closest a test can get to a blocked host without waiting for a real connect timeout: the
    /// connection is made and then broken, which reqwest reports the same way it reports the rest
    /// of a network going wrong.
    async fn refuses() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::{Arc, atomic::AtomicUsize};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                drop(stream);
            }
        });
        (url, hits)
    }

    /// A listener that answers one GET with `payload`.
    async fn serves(payload: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut scratch = [0_u8; 1024];
                let _ = stream.read(&mut scratch).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&payload).await;
                let _ = stream.flush().await;
            }
        });
        url
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_mirror_is_tried_before_any_url_is_retried() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = b"weights".repeat(64).to_vec();

        let (blocked, hits) = refuses().await;
        let working = serves(payload.clone()).await;

        // The shape of a real manifest on a Vietnamese connection: the canonical address is
        // unreachable and a mirror is fine.
        let mut entry = entry_for(&payload, blocked);
        entry.mirror = vec![working];

        let dl = Downloader::new(tmp.path().join("staging"))
            .unwrap()
            .with_max_retries(3);
        let dest = tmp.path().join("out.bin");
        dl.fetch(&entry, &dest, |_| {}).await.unwrap();

        assert_eq!(tokio::fs::read(&dest).await.unwrap(), payload);
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the blocked address should be tried once and then left alone, not retried four times \
             before the mirror is reached"
        );
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
