//! Models that came with the app, installed on first run.
//!
//! The first thing a new install did was ask for the network. It measured the machine, listed the
//! models, and waited for a download of seventy megabytes before it could transcribe a word — and
//! when the list could not be fetched, which is a normal Tuesday on a Vietnamese consumer ISP, the
//! screen said "Could not reach the model list. Check the network" and there was nothing else to
//! press.
//!
//! An app you download should run. For one release the desktop installers carried the recogniser
//! and the voice detector, and this copied them into the vault the first time the daemon started.
//!
//! **The published installers no longer carry them.** That was the wrong place to fix the problem:
//! it charged everybody 76 MB on every download to route around two hosts being blocked, and it
//! left the actual failure — a catalogue you can read and cannot install from — in place for
//! anybody who deleted a model or wanted a different one. The weights now have a third address on
//! `summo.techainer.com`, which Vietnam reaches, so the download works and the installer does not
//! have to pre-empt it.
//!
//! This stays for the case it is genuinely the answer to: an air-gapped machine, or a build made
//! for one. `SUMMO_BUNDLE_MODELS=1` puts the models back in the bundle — see
//! `scripts/bundle-models.sh` — and everything below then does what it always did.
//!
//! **Copied, not read in place.** The store is content-addressed and things are removed from it —
//! the models screen has a delete button — and a store that silently repaired itself from a
//! read-only directory would make that button lie. After the first run these are ordinary
//! installed models: they can be removed, they are counted in the disk figure on the settings
//! screen, and nothing distinguishes them from a model somebody chose.
//!
//! **Verified, not trusted.** The digest is checked on the way in, the same as a download. The
//! bytes come from inside our own installer, so this should never fail; if it ever does, it means
//! something rewrote the bundle, and a silent copy would be the worst possible response.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use summo_core::{Error, ModelId, Result, paths::Paths};

use crate::manifest::Manifest;
use crate::store::ModelStore;

/// Where the app put the models it shipped with.
///
/// Set by the desktop shell, which is the only thing that knows where its own resources are. Unset
/// for a `cargo run`, a server install or the CLI tarball — all of which download the way they
/// always have.
pub const ENV_BUNDLED: &str = "SUMMO_BUNDLED_MODELS";

/// What a seeding run did, for the log line that explains a first start taking two seconds longer.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Seeded {
    pub installed: Vec<ModelId>,
    /// Models in the bundle that were already in the store, by digest.
    pub already: Vec<ModelId>,
}

/// The directory the app shipped, if there is one.
#[must_use]
pub fn bundled_dir() -> Option<PathBuf> {
    let raw = std::env::var_os(ENV_BUNDLED)?;
    let path = PathBuf::from(raw);
    path.is_dir().then_some(path)
}

/// Install everything in `dir` that is not already installed.
///
/// The layout matches the store's own, because `scripts/bundle-models.sh` writes it by asking the
/// same manifest the app would have downloaded:
///
/// ```text
/// models/
///   manifests/gipformer-65m.json
///   blobs/sha256/3c/3cc1a719…
/// ```
pub fn seed_from(dir: &Path, paths: &Paths) -> Result<Seeded> {
    let store = ModelStore::new(paths.clone());
    let manifests = dir.join("manifests");
    let mut out = Seeded::default();

    let entries = match std::fs::read_dir(&manifests) {
        Ok(entries) => entries,
        // No manifests is not a failure. A build that shipped no models is a build that shipped no
        // models, and the app downloads them like it always did.
        Err(_) => return Ok(out),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let manifest = Manifest::parse(&body)?;

        if store.is_installed(&manifest) {
            out.already.push(manifest.id.clone());
            continue;
        }

        for file in manifest.platform_files() {
            let target = paths.blob(&file.sha256)?;
            if target.is_file() {
                continue;
            }
            let source = dir
                .join("blobs")
                .join("sha256")
                .join(&file.sha256[..2])
                .join(&file.sha256);
            let bytes = std::fs::read(&source).map_err(|e| Error::io(&source, e))?;

            let digest = format!("{:x}", Sha256::digest(&bytes));
            if digest != file.sha256 {
                return Err(Error::Other(format!(
                    "{}: the bundled copy of `{}` does not match its manifest",
                    manifest.id, file.name
                )));
            }

            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::write(&target, &bytes).map_err(|e| Error::io(&target, e))?;
        }

        let installed_manifest = paths.manifests().join(format!("{}.json", manifest.id));
        if let Some(parent) = installed_manifest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&installed_manifest, &body)
            .map_err(|e| Error::io(&installed_manifest, e))?;
        out.installed.push(manifest.id.clone());
    }

    Ok(out)
}

/// Seed from whatever the app shipped, once per vault.
///
/// The marker is the whole of the "once". Without it, a person who deleted the bundled model to
/// get seventy megabytes back would find it again on the next start, and the delete button on the
/// models screen would be a lie told politely. With it, seeding is a thing that happened to this
/// vault on the day it was created and never again.
pub fn seed(paths: &Paths) -> Result<Seeded> {
    let Some(dir) = bundled_dir() else {
        return Ok(Seeded::default());
    };
    let marker = paths.models().join(".seeded");
    if marker.exists() {
        return Ok(Seeded::default());
    }

    let seeded = seed_from(&dir, paths)?;

    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    // Written after the copy, so a crash half-way through means the next start tries again rather
    // than leaving a vault with three of four files.
    std::fs::write(&marker, "").map_err(|e| Error::io(&marker, e))?;
    Ok(seeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(dir: &Path, id: &str, bytes: &[u8]) -> String {
        let digest = format!("{:x}", Sha256::digest(bytes));
        let blob = dir.join("blobs").join("sha256").join(&digest[..2]);
        std::fs::create_dir_all(&blob).unwrap();
        std::fs::write(blob.join(&digest), bytes).unwrap();

        let manifest = serde_json::json!({
            "schema": 1,
            "id": id,
            "name": id,
            "task": "asr",
            "mode": "live",
            "runtime": "sherpa-onnx/transducer-offline",
            "license": "MIT",
            "langs": ["vi"],
            "files": [{
                "name": "model.onnx",
                "url": "https://example.invalid/model.onnx",
                "sha256": digest,
                "size": bytes.len(),
            }],
        })
        .to_string();
        std::fs::create_dir_all(dir.join("manifests")).unwrap();
        std::fs::write(dir.join("manifests").join(format!("{id}.json")), &manifest).unwrap();
        digest
    }

    #[test]
    fn a_bundled_model_is_an_installed_model_afterwards() {
        let home = tempfile::tempdir().unwrap();
        let shipped = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path());
        let digest = bundle(shipped.path(), "gipformer-65m", b"weights, notionally");

        let seeded = seed_from(shipped.path(), &paths).unwrap();
        assert_eq!(seeded.installed.len(), 1, "{seeded:?}");

        assert!(
            paths.blob(&digest).unwrap().is_file(),
            "the blob is in the store"
        );
        let store = ModelStore::new(paths.clone());
        assert!(
            store
                .list()
                .iter()
                .any(|m| m.id.as_str() == "gipformer-65m"),
            "and the model lists as installed"
        );
    }

    #[test]
    fn seeding_twice_installs_nothing_the_second_time() {
        let home = tempfile::tempdir().unwrap();
        let shipped = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path());
        bundle(shipped.path(), "gipformer-65m", b"weights, notionally");

        seed_from(shipped.path(), &paths).unwrap();
        let again = seed_from(shipped.path(), &paths).unwrap();
        assert!(again.installed.is_empty(), "{again:?}");
        assert_eq!(again.already.len(), 1);
    }

    /// A model somebody deleted stays deleted.
    ///
    /// Checked through `seed`, which is what the daemon calls, because the marker is where the
    /// promise lives — `seed_from` on its own would happily put it back.
    ///
    /// The models screen has a remove button, and it removes several hundred megabytes for a
    /// reason. An app that put its own copy back on the next start would be arguing with the user
    /// about their own disk.
    #[test]
    fn a_removed_model_is_not_reinstalled_by_a_later_start() {
        let home = tempfile::tempdir().unwrap();
        let shipped = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path());
        bundle(shipped.path(), "gipformer-65m", b"weights, notionally");
        seed_from(shipped.path(), &paths).unwrap();

        let store = ModelStore::new(paths.clone());
        let id = ModelId::parse("gipformer-65m").unwrap();
        store.remove(&id).unwrap();

        assert!(store.installed(&id).is_err(), "removal really removed it");

        // SAFETY: single-threaded test.
        unsafe { std::env::set_var(ENV_BUNDLED, shipped.path()) };
        // The marker was not written by `seed_from`, so write it the way `seed` would have.
        std::fs::write(paths.models().join(".seeded"), "").unwrap();
        let again = seed(&paths).unwrap();
        unsafe { std::env::remove_var(ENV_BUNDLED) };

        assert!(again.installed.is_empty(), "{again:?}");
        assert!(
            store.installed(&id).is_err(),
            "a start after a removal must not put it back"
        );
    }

    #[test]
    fn a_bundle_whose_bytes_were_swapped_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let shipped = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path());
        let digest = bundle(shipped.path(), "gipformer-65m", b"weights, notionally");

        // Somebody rewrote the file inside the installer.
        let blob = shipped
            .path()
            .join("blobs")
            .join("sha256")
            .join(&digest[..2])
            .join(&digest);
        std::fs::write(&blob, b"something else entirely").unwrap();

        let error = seed_from(shipped.path(), &paths).unwrap_err().to_string();
        assert!(error.contains("does not match its manifest"), "{error}");
        assert!(
            !paths.blob(&digest).unwrap().is_file(),
            "and nothing was written"
        );
    }

    #[test]
    fn no_bundle_is_not_an_error() {
        let home = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let paths = Paths::at(home.path());
        assert_eq!(seed_from(empty.path(), &paths).unwrap(), Seeded::default());
    }
}
