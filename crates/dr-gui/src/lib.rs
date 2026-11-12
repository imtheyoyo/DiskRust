//! Interface graphique de `diskrust` (strictement en lecture seule).
//!
//! * [`model`] : toute la logique hors interface (chargement d'un disque dans un
//!   [`model::DiskView`], recherche rapide, rapports). Testable sans écran.
//! * `app` (fonctionnalité `ui`) : la fenêtre `eframe`/`egui`.
//!
//! Rien dans cette crate n'ouvre un périphérique en écriture.

#[cfg(feature = "ui")]
pub mod app;
pub mod model;
