//! Links `libten_vad` when the `ten-vad` feature is enabled.
//!
//! The library is deliberately not vendored: its licence adds restrictions on top of Apache-2.0
//! that are incompatible with this project's AGPL-3.0 licence, so it can only be supplied by the
//! user. Point `SUMMO_TEN_VAD_LIB` at the directory containing `libten_vad.{so,dylib,dll}`.
fn main() {
    println!("cargo:rerun-if-env-changed=SUMMO_TEN_VAD_LIB");

    if std::env::var_os("CARGO_FEATURE_TEN_VAD").is_none() {
        return;
    }

    match std::env::var("SUMMO_TEN_VAD_LIB") {
        Ok(dir) => {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=dylib=ten_vad");
            // Let the built binary find the library without LD_LIBRARY_PATH.
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
        Err(_) => panic!(
            "feature `ten-vad` is enabled but SUMMO_TEN_VAD_LIB is not set.\n\
             TEN-VAD is not redistributed with Summo (see docs/adr/0001-vad-backend-licensing.md).\n\
             Fetch it yourself and set SUMMO_TEN_VAD_LIB to the directory holding libten_vad."
        ),
    }
}
