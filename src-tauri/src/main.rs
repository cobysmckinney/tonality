// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // What this build reads and writes on disk, for the release workflow to put in the update
    // feed (see `tonality_core::format`). Before anything of the app starts.
    if std::env::args().nth(1).as_deref() == Some("--formats") {
        println!("{}", tonality_app::formats::json());
        return;
    }
    tonality_app::run()
}
