#![windows_subsystem = "windows"] // no console window
mod annotation;
mod app;
mod capture;
mod clipboard;
mod hotkey;
mod image;
mod selection;
mod smooth;
mod storage;
mod tray;

fn main() {
    if let Err(e) = app::run() {
        app::show_error(&format!("shotr failed to start: {e}"));
    }
}
