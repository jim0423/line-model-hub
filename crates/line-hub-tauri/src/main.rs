//! Tauri shell entry — desktop app that drives line-hub-core.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

fn main() {
    line_hub_tauri_lib::run();
}