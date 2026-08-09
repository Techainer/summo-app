//! On-disk model storage.
//!
//! Files live in a content-addressed blob store keyed by sha256, exactly like Ollama's. Two models
//! that reference the same tokenizer, VAD or multimodal projector share one copy on disk, and
//! deleting a model only reclaims the blobs no remaining manifest still references.

use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

use summo_core::{Error, ModelId, Result, paths::Paths};

use crate::{
    download::{DownloadProgress, Downloader},
    manifest::Manifest,
};

/// An installed model with its files resolved to concrete paths.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledModel {
    pub manifest: Manifest,
    /// Manifest file name → blob path.
    pub files: BTreeMap<String, PathBuf>,
}

impl InstalledModel {
    /// Resolve a `params` key that names a file, e.g. `params.encoder` → the encoder blob.
    pub fn param_path(&self, key: &str) -> Option<&PathBuf> {
        let name = self.manifest.params.get(key)?.as_str()?;
        self.files.get(name)
    }

    pub fn path(&self, file_name: &str) -> Option<&PathBuf> {
        self.files.get(file_name)
    }
}

/// Reads and writes the local model directory.
pub struct ModelStore {
    paths: Paths,
}

impl ModelStore {
    #[must_use]
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    #[must_use]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    fn manifest_path(&self, id: &ModelId) -> PathBuf {
        self.paths.manifests().join(format!("{id}.json"))
    }

    /// Whether every blob a manifest needs is already present.
    pub fn is_installed(&self, manifest: &Manifest) -> bool {
        manifest.platform_files().all(|f| {
            self.paths
                .blob(&f.sha256)
                .map(|p| p.is_file())
                .unwrap_or(false)
        })
    }

    /// Resolve an installed model's files. Fails if any blob is missing.
    pub fn resolve(&self, manifest: &Manifest) -> Result<InstalledModel> {
        let mut files = BTreeMap::new();
        for f in manifest.platform_files() {
            let path = self.paths.blob(&f.sha256)?;
            if !path.is_file() {
                return Err(Error::ModelNotFound(format!(
                    "{}: blob for `{}` is not installed",
                    manifest.id, f.name
                )));
            }
            files.insert(f.name.clone(), path);
        }
        Ok(InstalledModel {
            manifest: manifest.clone(),
            files,
        })
    }

    /// Download any missing blobs, then record the manifest as installed.
    ///
    /// `on_progress` receives aggregate progress across all files, which is what a progress bar
    /// actually wants.
    pub async fn install<F>(
        &self,
        manifest: &Manifest,
        downloader: &Downloader,
        mut on_progress: F,
    ) -> Result<InstalledModel>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        manifest.validate()?;
        let total = manifest.total_bytes();
        let mut completed = 0_u64;

        for entry in manifest.platform_files() {
            let dest = self.paths.blob(&entry.sha256)?;
            let base = completed;
            downloader
                .fetch(entry, &dest, |p| {
                    on_progress(DownloadProgress {
                        done: base + p.done,
                        total,
                        resuming: p.resuming,
                    });
                })
                .await?;
            completed += entry.size;
        }

        // Written last: the manifest's presence is the signal that the model is usable.
        let path = self.manifest_path(&manifest.id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::io(parent, e))?;
        }
        tokio::fs::write(&path, serde_json::to_vec_pretty(manifest)?)
            .await
            .map_err(|e| Error::io(&path, e))?;

        self.resolve(manifest)
    }

    /// Load a previously installed manifest.
    pub fn installed(&self, id: &ModelId) -> Result<Manifest> {
        let path = self.manifest_path(id);
        let body =
            std::fs::read_to_string(&path).map_err(|_| Error::ModelNotFound(id.to_string()))?;
        Manifest::parse(&body)
    }

    /// Every installed manifest. Unreadable entries are skipped with a warning rather than failing
    /// the whole listing — one corrupt file should not hide the rest of the library.
    pub fn list(&self) -> Vec<Manifest> {
        let dir = self.paths.manifests();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(Error::from)
                .and_then(|b| Manifest::parse(&b))
            {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping manifest"),
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Uninstall a model and reclaim blobs nothing else references.
    ///
    /// Returns the number of bytes freed.
    pub fn remove(&self, id: &ModelId) -> Result<u64> {
        let manifest = self.installed(id)?;
        std::fs::remove_file(self.manifest_path(id)).ok();

        // Anything still referenced by a surviving manifest must stay.
        let keep: HashSet<String> = self
            .list()
            .iter()
            .flat_map(|m| m.files.iter().map(|f| f.sha256.clone()))
            .collect();

        let mut freed = 0;
        for f in &manifest.files {
            if keep.contains(&f.sha256) {
                continue;
            }
            let path = self.paths.blob(&f.sha256)?;
            if let Ok(meta) = std::fs::metadata(&path) {
                freed += meta.len();
            }
            std::fs::remove_file(&path).ok();
        }
        Ok(freed)
    }

    /// Total bytes held by the blob store.
    #[must_use]
    pub fn disk_usage(&self) -> u64 {
        fn walk(dir: &std::path::Path) -> u64 {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return 0;
            };
            entries
                .flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => walk(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        }
        walk(&self.paths.blobs())
    }

    /// Delete blobs no installed manifest references. Returns bytes freed.
    ///
    /// These accumulate when a manifest is updated in place — the old weights stay behind.
    pub fn gc(&self) -> Result<u64> {
        let keep: HashSet<String> = self
            .list()
            .iter()
            .flat_map(|m| m.files.iter().map(|f| f.sha256.clone()))
            .collect();

        let mut freed = 0;
        let Ok(shards) = std::fs::read_dir(self.paths.blobs()) else {
            return Ok(0);
        };
        for shard in shards.flatten() {
            let Ok(blobs) = std::fs::read_dir(shard.path()) else {
                continue;
            };
            for blob in blobs.flatten() {
                let name = blob.file_name().to_string_lossy().into_owned();
                if keep.contains(&name) {
                    continue;
                }
                if let Ok(meta) = blob.metadata() {
                    freed += meta.len();
                }
                std::fs::remove_file(blob.path()).ok();
            }
        }
        Ok(freed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    struct Fixture {
        _tmp: tempfile::TempDir,
        store: ModelStore,
        src_dir: PathBuf,
    }

    /// Build a store plus a directory of source files served over `file://`.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        paths.ensure().unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        Fixture {
            store: ModelStore::new(paths),
            src_dir,
            _tmp: tmp,
        }
    }

    impl Fixture {
        /// Create a source file and return the manifest entry that points at it.
        fn file(&self, name: &str, contents: &[u8]) -> crate::manifest::FileEntry {
            let path = self.src_dir.join(name);
            std::fs::write(&path, contents).unwrap();
            crate::manifest::FileEntry {
                name: name.to_string(),
                sha256: hex::encode(Sha256::digest(contents)),
                size: contents.len() as u64,
                url: format!("file://{}", path.display()),
                mirror: Vec::new(),
                platform: None,
            }
        }

        fn manifest(&self, id: &str, files: Vec<crate::manifest::FileEntry>) -> Manifest {
            Manifest {
                schema: 1,
                id: ModelId::parse(id).unwrap(),
                name: format!("Model {id}"),
                task: crate::manifest::Task::Asr,
                mode: crate::manifest::Mode::Batch,
                runtime: "test/runtime".into(),
                langs: vec!["vi".into()],
                domains: Vec::new(),
                license: "Apache-2.0".into(),
                attribution: None,
                redistributable: true,
                size_bytes: files.iter().map(|f| f.size).sum(),
                profile: crate::manifest::Profile::default(),
                params: [("encoder".to_string(), serde_json::json!("encoder.onnx"))]
                    .into_iter()
                    .collect(),
                files,
                description: None,
            }
        }

        fn downloader(&self) -> Downloader {
            Downloader::new(self.store.paths().downloads()).unwrap()
        }
    }

    #[tokio::test]
    async fn install_then_resolve_yields_loadable_paths() {
        let fx = fixture();
        let encoder = fx.file("encoder.onnx", b"encoder weights");
        let m = fx.manifest("model-a", vec![encoder]);

        let installed = fx
            .store
            .install(&m, &fx.downloader(), |_| {})
            .await
            .unwrap();

        assert!(fx.store.is_installed(&m));
        let path = installed
            .param_path("encoder")
            .expect("params.encoder should resolve");
        assert_eq!(std::fs::read(path).unwrap(), b"encoder weights");
    }

    #[tokio::test]
    async fn identical_files_are_stored_once() {
        let fx = fixture();
        let shared = fx.file("tokens.txt", b"shared tokenizer");
        let a = fx.manifest("model-a", vec![shared.clone()]);
        let b = fx.manifest("model-b", vec![shared]);

        fx.store
            .install(&a, &fx.downloader(), |_| {})
            .await
            .unwrap();
        let after_first = fx.store.disk_usage();
        fx.store
            .install(&b, &fx.downloader(), |_| {})
            .await
            .unwrap();

        assert_eq!(
            fx.store.disk_usage(),
            after_first,
            "a shared blob must not be stored twice"
        );
        assert_eq!(fx.store.list().len(), 2);
    }

    #[tokio::test]
    async fn removing_one_model_keeps_blobs_another_still_needs() {
        let fx = fixture();
        let shared = fx.file("tokens.txt", b"shared tokenizer");
        let only_a = fx.file("a-only.onnx", b"weights for a");
        let a = fx.manifest("model-a", vec![shared.clone(), only_a.clone()]);
        let b = fx.manifest("model-b", vec![shared.clone()]);

        fx.store
            .install(&a, &fx.downloader(), |_| {})
            .await
            .unwrap();
        fx.store
            .install(&b, &fx.downloader(), |_| {})
            .await
            .unwrap();

        let freed = fx
            .store
            .remove(&ModelId::parse("model-a").unwrap())
            .unwrap();

        assert_eq!(
            freed, only_a.size,
            "only the exclusive blob should be reclaimed"
        );
        assert!(fx.store.paths().blob(&shared.sha256).unwrap().is_file());
        assert!(fx.store.is_installed(&b), "model-b must remain usable");
    }

    #[tokio::test]
    async fn gc_reclaims_orphans_and_spares_referenced_blobs() {
        let fx = fixture();
        let kept = fx.file("encoder.onnx", b"live weights");
        let m = fx.manifest("model-a", vec![kept.clone()]);
        fx.store
            .install(&m, &fx.downloader(), |_| {})
            .await
            .unwrap();

        // Simulate a stale blob left behind by an in-place manifest update.
        let orphan_digest = hex::encode(Sha256::digest(b"orphaned weights"));
        let orphan = fx.store.paths().blob(&orphan_digest).unwrap();
        std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        std::fs::write(&orphan, b"orphaned weights").unwrap();

        let freed = fx.store.gc().unwrap();

        assert_eq!(freed, b"orphaned weights".len() as u64);
        assert!(!orphan.exists());
        assert!(
            fx.store.is_installed(&m),
            "referenced blobs must survive gc"
        );
    }

    #[tokio::test]
    async fn progress_is_aggregated_across_files() {
        let fx = fixture();
        let f1 = fx.file("a.bin", &b"x".repeat(1000));
        let f2 = fx.file("b.bin", &b"y".repeat(500));
        let total = f1.size + f2.size;
        let m = fx.manifest("model-a", vec![f1, f2]);

        let mut max_done = 0;
        let mut over_total = false;
        fx.store
            .install(&m, &fx.downloader(), |p| {
                max_done = max_done.max(p.done);
                over_total |= p.done > p.total;
                assert_eq!(p.total, total);
            })
            .await
            .unwrap();

        assert!(
            !over_total,
            "aggregate progress must never exceed the total"
        );
        assert_eq!(max_done, total);
    }

    #[test]
    fn resolve_fails_when_a_blob_is_missing() {
        let fx = fixture();
        let m = fx.manifest("model-a", vec![fx.file("encoder.onnx", b"never installed")]);
        assert!(!fx.store.is_installed(&m));
        assert!(fx.store.resolve(&m).is_err());
    }

    #[test]
    fn listing_an_empty_store_is_not_an_error() {
        let fx = fixture();
        assert!(fx.store.list().is_empty());
        assert_eq!(fx.store.disk_usage(), 0);
    }

    #[test]
    fn corrupt_manifest_is_skipped_not_fatal() {
        let fx = fixture();
        std::fs::write(
            fx.store.paths().manifests().join("broken.json"),
            "{ not json",
        )
        .unwrap();
        assert!(
            fx.store.list().is_empty(),
            "listing should survive a bad file"
        );
    }
}
