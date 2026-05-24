// Prevent additional console window on Windows in release. macOS only build
// for this iteration, but we keep the attribute for parity with the Tauri
// template.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nightowl_lib::run();
}
