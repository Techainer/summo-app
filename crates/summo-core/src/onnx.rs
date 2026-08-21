//! Where ONNX Runtime comes from on the one platform that has to be told.
//!
//! Every target but Intel macOS links a runtime fetched at build time by `ort-sys`. Intel macOS has
//! no such build to fetch — Microsoft's last publication for it was ONNX Runtime 1.23.2 — so those
//! builds use `ort`'s `load-dynamic` feature, which opens a `libonnxruntime.dylib` at runtime and
//! needs to be told where it is.
//!
//! The release ships that dylib beside the executable, so "where is it" has a boring answer: next
//! to the program that is asking. `ORT_DYLIB_PATH` set by hand always wins, which is what makes a
//! developer's own build, a Homebrew runtime or a Nix store path usable without a rebuild.
//!
//! This is a no-op everywhere else, and deliberately so: one call at startup, no `cfg` at the call
//! site, and nothing to forget when a fourth binary starts loading models.

/// Point `ort` at the runtime shipped beside this executable, unless the environment already has.
///
/// Safe to call more than once and from any binary; the first call wins and later ones see the
/// variable already set. Returns the path it chose, for a log line.
#[allow(clippy::missing_const_for_fn)]
pub fn locate_runtime() -> Option<std::path::PathBuf> {
    if !cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        return None;
    }

    // Somebody has already decided. A build pointed at a runtime by hand is a build being debugged,
    // and overriding that from here would make the override look broken.
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return None;
    }

    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?;
    // Two places, because the app bundle and the tarball put it in different ones: `Summo.app`
    // keeps frameworks a level up in `Contents/Frameworks`, and the tarball is a flat directory.
    for candidate in [
        here.join("libonnxruntime.dylib"),
        here.join("../Frameworks/libonnxruntime.dylib"),
    ] {
        if candidate.is_file() {
            // SAFETY: called before any thread reads the environment — `main` does this first,
            // before the runtime is loaded and before the daemon spawns anything.
            unsafe { std::env::set_var("ORT_DYLIB_PATH", &candidate) };
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On every platform that fetches a runtime at build time this must do nothing at all — no
    /// environment variable, no filesystem poke, no surprise for a developer running the tests.
    #[test]
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    fn a_platform_with_a_prebuilt_runtime_is_left_alone() {
        assert!(locate_runtime().is_none());
        assert!(std::env::var_os("ORT_DYLIB_PATH").is_none());
    }
}
