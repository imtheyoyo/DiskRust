//! Point d'entrée de `diskrust-gui` (lecture seule).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    dr_gui::app::run()
}
