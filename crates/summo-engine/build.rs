//! Tell cargo that the interface is an input.
//!
//! `include_dir!` bakes `apps/web/dist` into the binary at compile time, and cargo has no idea it
//! did. Nothing in `src/` changes when the web app is rebuilt, so `cargo build` reports `Finished`
//! in two seconds and produces a binary serving the *previous* interface.
//!
//! That is a bad failure to have. It costs an afternoon rather than a minute, because everything
//! looks right: the web build succeeded, the daemon build succeeded, and the browser shows stale
//! markup with no indication that anything is out of date. It was found by writing a browser test
//! for a feature that was already working and watching it fail.
//!
//! One line of `rerun-if-changed` per file, because cargo watches files rather than trees.

use std::path::Path;

fn main() {
    // Only under the feature that embeds it. Without `bundled` there is nothing baked in, and a
    // missing `dist` must not be an error — `cargo test -p summo-engine` has to work on a machine
    // with no Node installed.
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_BUNDLED").is_none() {
        return;
    }

    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    watch(&dist);
}

/// Every file under `dist`, recursively.
///
/// Watching the directory alone is not enough: cargo notices a file being added or removed, and
/// misses one being rewritten in place — which is what Vite does to `index.html` on every build.
fn watch(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // No build yet. The compile itself will fail with a clearer message than anything printed
        // here, and a missing directory must not fail a build that does not need it.
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            watch(&path);
        }
    }
}
