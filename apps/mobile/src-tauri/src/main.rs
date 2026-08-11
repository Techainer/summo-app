// Neither iOS nor Android calls `main` — the platform harness calls the library's entry point. This
// exists so `cargo run` works on a desktop for anyone debugging the mobile shell without a device.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    summo_mobile::run();
}
