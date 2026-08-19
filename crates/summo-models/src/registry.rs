//! Manifest resolution.
//!
//! The registry is a set of static JSON files, and the app knows several places to look for them.
//! That ordering is the mechanism that keeps the open-source build independent of any one host:
//! every default source is a different view of the same git repository, so none of them is a
//! single point of failure and a user can always point at a directory of their own instead.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use summo_core::{Error, ModelId, Result, paths::ENV_REGISTRY};

use crate::manifest::{Manifest, Mode, Task};

/// The registry repository itself, read straight from the default branch.
///
/// First because it is the only source that cannot be stale: it *is* the repository. A CDN in
/// front of it would be faster for a crowd, and this is a few kilobytes fetched once per install.
pub const DEFAULT_GITHUB: &str = "https://raw.githubusercontent.com/Techainer/summo-registry/main";
/// The same files through jsDelivr, for when raw.githubusercontent is blocked or rate-limited.
///
/// jsDelivr serves any public GitHub repository with no account and no infrastructure of ours, and
/// it reaches networks where raw.githubusercontent does not. It caches a branch for up to twelve
/// hours, which is why it is second: a model published this morning must not be invisible until
/// tonight when the first source can already see it.
///
/// There used to be a `https://registry.summo.app` ahead of both. Nobody ever registered
/// `summo.app`, so every first run paid a failed DNS lookup — and on a network whose resolver
/// answers slowly rather than not at all, that is a pause before the app can list a single model.
/// A source that has never existed is not a fallback, it is latency with a comment above it.
pub const DEFAULT_JSDELIVR: &str = "https://cdn.jsdelivr.net/gh/Techainer/summo-registry@main";
/// The same files again, from our own domain, for the country this product is built for.
///
/// A user in Hanoi on a 12-core machine opened the app and was told "Could not reach the model
/// list. Check the network." Their network was fine. `raw.githubusercontent.com` is routinely
/// unreachable from Vietnamese consumer ISPs, and jsDelivr has been intermittently poisoned there
/// too — so the chain above is two sources with correlated failure, which is one source wearing a
/// hat.
///
/// `summo.techainer.com` is a Cloudflare Worker we already deploy, and the site's build copies the
/// registry into it. It is not a fallback for GitHub being *down*, which is rare; it is a fallback
/// for GitHub being *blocked*, which is Tuesday.
///
/// Last on purpose. It is a copy, and a copy refreshed when the site is rebuilt — the two sources
/// above are the repository itself. Somebody who can reach GitHub should read GitHub.
pub const DEFAULT_MIRROR: &str = "https://summo.techainer.com/registry";

/// One place manifests can be read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrySource {
    /// An `https://` base URL.
    Http(String),
    /// A local directory laid out like the registry repository.
    Dir(PathBuf),
}

impl RegistrySource {
    /// Parse a user-supplied value: `file://…` and bare paths become [`Self::Dir`].
    pub fn parse(value: &str) -> Result<Self> {
        if let Some(path) = value.strip_prefix("file://") {
            return Ok(Self::Dir(PathBuf::from(path)));
        }
        if value.starts_with("https://") {
            return Ok(Self::Http(value.trim_end_matches('/').to_string()));
        }
        if value.starts_with("http://") {
            return Err(Error::Registry(format!(
                "refusing plain http registry `{value}`: manifests must be tamper-evident"
            )));
        }
        Ok(Self::Dir(PathBuf::from(value)))
    }

    fn manifest_location(&self, id: &ModelId) -> String {
        match self {
            Self::Http(base) => format!("{base}/models/{id}.json"),
            Self::Dir(dir) => dir
                .join("models")
                .join(format!("{id}.json"))
                .display()
                .to_string(),
        }
    }

    fn index_location(&self) -> String {
        match self {
            Self::Http(base) => format!("{base}/index.json"),
            Self::Dir(dir) => dir.join("index.json").display().to_string(),
        }
    }
}

/// Summary row in `index.json`, enough to render the model picker without fetching every manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: ModelId,
    pub name: String,
    pub task: Task,
    pub mode: Mode,
    #[serde(default)]
    pub langs: Vec<String>,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    #[serde(default)]
    pub models: Vec<IndexEntry>,
}

/// Resolves model ids to manifests across an ordered list of sources.
pub struct Registry {
    sources: Vec<RegistrySource>,
    client: reqwest::Client,
    cache: tokio::sync::Mutex<HashMap<ModelId, Manifest>>,
}

impl Registry {
    /// Build the default chain: `SUMMO_REGISTRY` first if set, then GitHub, then jsDelivr.
    pub fn discover() -> Result<Self> {
        let mut sources = Vec::new();
        if let Ok(value) = std::env::var(ENV_REGISTRY)
            && !value.trim().is_empty()
        {
            sources.push(RegistrySource::parse(value.trim())?);
        }
        sources.push(RegistrySource::Http(DEFAULT_GITHUB.into()));
        sources.push(RegistrySource::Http(DEFAULT_JSDELIVR.into()));
        sources.push(RegistrySource::Http(DEFAULT_MIRROR.into()));
        Self::with_sources(sources)
    }

    pub fn with_sources(sources: Vec<RegistrySource>) -> Result<Self> {
        if sources.is_empty() {
            return Err(Error::Registry("no registry sources configured".into()));
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("summo/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Registry(format!("cannot build http client: {e}")))?;
        Ok(Self {
            sources,
            client,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn sources(&self) -> &[RegistrySource] {
        &self.sources
    }

    /// Fetch and validate a manifest, trying each source in order.
    pub async fn manifest(&self, id: &ModelId) -> Result<Manifest> {
        if let Some(hit) = self.cache.lock().await.get(id) {
            return Ok(hit.clone());
        }

        let mut last_err = None;
        for source in &self.sources {
            match self.read(&source.manifest_location(id), source).await {
                Ok(body) => match Manifest::parse(&body) {
                    Ok(manifest) => {
                        // A manifest that claims a different id than the file it was served as
                        // would let one model masquerade as another.
                        if &manifest.id != id {
                            last_err = Some(Error::InvalidManifest {
                                id: id.to_string(),
                                reason: format!("manifest declares id `{}`", manifest.id),
                            });
                            continue;
                        }
                        self.cache.lock().await.insert(id.clone(), manifest.clone());
                        return Ok(manifest);
                    }
                    Err(e) => {
                        tracing::warn!(%id, error = %e, "invalid manifest, trying next source");
                        last_err = Some(e);
                    }
                },
                Err(e) => {
                    tracing::debug!(%id, error = %e, "source miss");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::ModelNotFound(id.to_string())))
    }

    /// Fetch the catalogue from the first source that has one.
    pub async fn index(&self) -> Result<Index> {
        // Every source's failure, not the last one's.
        //
        // "Could not reach the model list. Check the network" is what a user in Hanoi was told
        // when two of three sources were blocked by their ISP — advice that sent them to look at
        // a router that was working. Which addresses were tried and what each answered is the
        // difference between that and a person who can see the shape of the problem.
        let mut failures = Vec::new();
        for source in &self.sources {
            let location = source.index_location();
            match self.read(&location, source).await {
                Ok(body) => match serde_json::from_str::<Index>(&body) {
                    Ok(index) => return Ok(index),
                    Err(e) => failures.push(format!("{location}: malformed index ({e})")),
                },
                Err(e) => failures.push(e.to_string()),
            }
        }
        Err(Error::Registry(format!(
            "no registry source answered:\n  {}",
            failures.join("\n  ")
        )))
    }

    async fn read(&self, location: &str, source: &RegistrySource) -> Result<String> {
        match source {
            RegistrySource::Dir(_) => tokio::fs::read_to_string(location)
                .await
                .map_err(|e| Error::io(location, e)),
            RegistrySource::Http(_) => {
                let resp = self
                    .client
                    .get(location)
                    .send()
                    .await
                    .map_err(|e| Error::Registry(format!("{location}: {e}")))?;
                if !resp.status().is_success() {
                    return Err(Error::Registry(format!(
                        "{location}: http {}",
                        resp.status()
                    )));
                }
                resp.text()
                    .await
                    .map_err(|e| Error::Registry(format!("{location}: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three sources that fail for different reasons, which is the only kind of fallback worth
    /// having.
    ///
    /// The chain was GitHub and a CDN in front of GitHub. Both are blocked from the same networks —
    /// a Vietnamese ISP does not distinguish them — so a user in the country this is built for had
    /// two sources and no fallback. The mirror is a different company, a different address, and a
    /// domain we control.
    #[test]
    fn the_defaults_do_not_all_fail_together() {
        // SAFETY: single-threaded test, and the variable is removed immediately after.
        unsafe { std::env::remove_var(ENV_REGISTRY) };
        let registry = Registry::discover().unwrap();
        let hosts: Vec<String> = registry
            .sources()
            .iter()
            .map(|source| match source {
                RegistrySource::Http(base) => base.clone(),
                RegistrySource::Dir(path) => path.display().to_string(),
            })
            .collect();
        assert!(
            hosts
                .iter()
                .any(|h| h.contains("raw.githubusercontent.com")),
            "the repository itself must be a source: {hosts:?}"
        );
        assert!(
            hosts.iter().any(|h| h.contains("summo.techainer.com")),
            "a source that is not GitHub must be in the chain: {hosts:?}"
        );
        assert_eq!(
            hosts.last().map(String::as_str),
            Some(DEFAULT_MIRROR),
            "the copy goes last; the repository is authoritative"
        );
    }

    /// The message a person actually reads when nothing answers.
    #[tokio::test]
    async fn a_dead_chain_names_every_address_it_tried() {
        let registry = Registry::with_sources(vec![
            RegistrySource::Http("https://first.invalid/registry".into()),
            RegistrySource::Http("https://second.invalid/registry".into()),
        ])
        .unwrap();
        let error = registry.index().await.unwrap_err().to_string();
        assert!(error.contains("first.invalid"), "{error}");
        assert!(error.contains("second.invalid"), "{error}");
    }

    fn sample_manifest(id: &str) -> String {
        serde_json::json!({
            "schema": 1,
            "id": id,
            "name": "Test model",
            "task": "vad",
            "mode": "live",
            "runtime": "test/runtime",
            "license": "Apache-2.0",
            "files": [{
                "name": "model.onnx",
                "sha256": "b".repeat(64),
                "size": 1024,
                "url": "https://cdn.summo.app/blobs/sha256/bbb"
            }]
        })
        .to_string()
    }

    fn dir_registry() -> (tempfile::TempDir, Registry) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(
            tmp.path().join("models/test-vad.json"),
            sample_manifest("test-vad"),
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("index.json"),
            serde_json::json!({
                "schema": 1,
                "models": [{
                    "id": "test-vad", "name": "Test VAD", "task": "vad",
                    "mode": "live", "langs": [], "size_bytes": 1024, "license": "Apache-2.0"
                }]
            })
            .to_string(),
        )
        .unwrap();
        let reg =
            Registry::with_sources(vec![RegistrySource::Dir(tmp.path().to_path_buf())]).unwrap();
        (tmp, reg)
    }

    #[test]
    fn plain_http_registries_are_refused() {
        assert!(RegistrySource::parse("http://example.com").is_err());
        assert!(RegistrySource::parse("https://example.com").is_ok());
    }

    #[test]
    fn file_urls_and_bare_paths_both_mean_a_directory() {
        assert_eq!(
            RegistrySource::parse("file:///srv/registry").unwrap(),
            RegistrySource::Dir(PathBuf::from("/srv/registry"))
        );
        assert_eq!(
            RegistrySource::parse("/srv/registry").unwrap(),
            RegistrySource::Dir(PathBuf::from("/srv/registry"))
        );
    }

    #[test]
    fn trailing_slashes_do_not_double_up() {
        let RegistrySource::Http(base) = RegistrySource::parse("https://x.dev/").unwrap() else {
            panic!("expected http source")
        };
        assert_eq!(base, "https://x.dev");
    }

    #[tokio::test]
    async fn resolves_from_a_local_directory() {
        let (_tmp, reg) = dir_registry();
        let m = reg
            .manifest(&ModelId::parse("test-vad").unwrap())
            .await
            .unwrap();
        assert_eq!(m.name, "Test model");
        assert_eq!(m.task, Task::Vad);
    }

    #[tokio::test]
    async fn falls_through_to_a_later_source() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(
            tmp.path().join("models/test-vad.json"),
            sample_manifest("test-vad"),
        )
        .unwrap();

        // First source is an empty directory; resolution must continue rather than fail.
        let empty = tempfile::tempdir().unwrap();
        let reg = Registry::with_sources(vec![
            RegistrySource::Dir(empty.path().to_path_buf()),
            RegistrySource::Dir(tmp.path().to_path_buf()),
        ])
        .unwrap();

        assert!(
            reg.manifest(&ModelId::parse("test-vad").unwrap())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn manifest_claiming_another_id_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        // Served as `test-vad` but declares itself `silero-vad`.
        std::fs::write(
            tmp.path().join("models/test-vad.json"),
            sample_manifest("silero-vad"),
        )
        .unwrap();
        let reg =
            Registry::with_sources(vec![RegistrySource::Dir(tmp.path().to_path_buf())]).unwrap();

        let err = reg
            .manifest(&ModelId::parse("test-vad").unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("declares id"), "got: {err}");
    }

    #[tokio::test]
    async fn second_lookup_is_served_from_cache() {
        let (tmp, reg) = dir_registry();
        let id = ModelId::parse("test-vad").unwrap();
        reg.manifest(&id).await.unwrap();
        // Remove the backing file: only a cache hit can succeed now.
        std::fs::remove_file(tmp.path().join("models/test-vad.json")).unwrap();
        assert!(reg.manifest(&id).await.is_ok());
    }

    #[tokio::test]
    async fn index_lists_available_models() {
        let (_tmp, reg) = dir_registry();
        let index = reg.index().await.unwrap();
        assert_eq!(index.models.len(), 1);
        assert_eq!(index.models[0].id.as_str(), "test-vad");
    }

    #[tokio::test]
    async fn missing_model_reports_not_found() {
        let (_tmp, reg) = dir_registry();
        let err = reg
            .manifest(&ModelId::parse("nope").unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nope"), "got: {err}");
    }

    #[test]
    fn empty_source_list_is_a_configuration_error() {
        assert!(Registry::with_sources(vec![]).is_err());
    }
}
