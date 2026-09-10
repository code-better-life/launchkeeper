//! Thin wrapper: everything lives in the library so that the binding export
//! can be a `cargo test` target (see `lib.rs`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    launchkeeper_app_lib::run();
}
