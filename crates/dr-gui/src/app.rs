//! Fenêtre `eframe`/`egui` de `diskrust-gui`. Toute la logique métier est dans [`crate::model`].

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;

use dr_disk::{enumerate_disks, DiskInfo};
use dr_search::Progress;
use eframe::egui;

use crate::model::{
    disk_list_label, load_source, progress_fraction, progress_text, report_text, run_search,
    DiskSource, DiskView, SearchOutcome, BANDEAU_LECTURE_SEULE, CONSEIL_ADMIN,
};

/// Message envoyé par le fil d'exécution de la recherche.
enum Msg {
    Progress(Progress),
    Done(Result<SearchOutcome, String>),
}

/// État de la recherche rapide.
enum SearchState {
    Idle,
    Running {
        rx: Receiver<Msg>,
        progress: Option<Progress>,
    },
    Done(SearchOutcome),
    Failed(String),
}

/// Application principale.
pub struct DiskRustApp {
    disks: Vec<DiskInfo>,
    source: Option<DiskSource>,
    view: Option<DiskView>,
    search: SearchState,
    status: String,
    about_open: bool,
}

impl DiskRustApp {
    /// Crée l'application et énumère les disques.
    pub fn new() -> Self {
        let mut app = Self {
            disks: Vec::new(),
            source: None,
            view: None,
            search: SearchState::Idle,
            status: "Prêt.".to_string(),
            about_open: false,
        };
        app.refresh_disks();
        app
    }

    fn refresh_disks(&mut self) {
        self.disks = enumerate_disks();
        self.status = format!("{} disque(s) physique(s) trouvé(s).", self.disks.len());
    }

    fn open_source(&mut self, source: DiskSource) {
        self.search = SearchState::Idle;
        match load_source(&source) {
            Ok(view) => {
                self.status = format!("Disque chargé : {}", view.label);
                self.view = Some(view);
                self.source = Some(source);
            }
            Err(e) => {
                self.view = None;
                self.source = None;
                self.status = format!("Impossible d'ouvrir {} : {e}", source.label());
            }
        }
    }

    fn pick_image(&mut self) {
        let file = rfd::FileDialog::new()
            .set_title("Ouvrir une image disque")
            .add_filter("Images disque (.dd, .img, .raw)", &["dd", "img", "raw"])
            .add_filter("Tous les fichiers", &["*"])
            .pick_file();
        if let Some(path) = file {
            self.open_source(DiskSource::Image(path));
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let (Some(source), Some(view)) = (self.source.clone(), self.view.clone()) else {
            return;
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = match source.open() {
                Ok(dev) => {
                    let tx_progress = tx.clone();
                    let ctx_progress = ctx.clone();
                    Ok(run_search(dev.as_ref(), &view, &mut |p: &Progress| {
                        let _ = tx_progress.send(Msg::Progress(*p));
                        ctx_progress.request_repaint();
                    }))
                }
                Err(e) => Err(format!("Ouverture impossible : {e}")),
            };
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
        self.search = SearchState::Running { rx, progress: None };
        self.status = "Recherche rapide en cours...".to_string();
    }

    fn poll_search(&mut self) {
        let mut finished = None;
        if let SearchState::Running { rx, progress } = &mut self.search {
            loop {
                match rx.try_recv() {
                    Ok(Msg::Progress(p)) => *progress = Some(p),
                    Ok(Msg::Done(r)) => {
                        finished = Some(r);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = Some(Err("Le fil de recherche s'est arrêté.".to_string()));
                        break;
                    }
                }
            }
        }
        match finished {
            Some(Ok(outcome)) => {
                self.status = format!(
                    "Recherche terminée : {} système(s) de fichiers trouvé(s).",
                    outcome.hits.len()
                );
                self.search = SearchState::Done(outcome);
            }
            Some(Err(e)) => {
                self.status = format!("Recherche échouée : {e}");
                self.search = SearchState::Failed(e);
            }
            None => {}
        }
    }

    fn current_report(&self) -> Option<String> {
        let view = self.view.as_ref()?;
        let outcome = match &self.search {
            SearchState::Done(o) => Some(o),
            _ => None,
        };
        Some(report_text(view, outcome))
    }

    fn export_report(&mut self) {
        let Some(text) = self.current_report() else {
            return;
        };
        let file = rfd::FileDialog::new()
            .set_title("Exporter le rapport")
            .set_file_name("rapport-diskrust.txt")
            .add_filter("Texte (.txt)", &["txt"])
            .save_file();
        if let Some(path) = file {
            self.status = match std::fs::write(&path, text) {
                Ok(()) => format!("Rapport exporté : {}", path.display()),
                Err(e) => format!("Échec de l'export : {e}"),
            };
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Disques");
        ui.horizontal(|ui| {
            if ui.button("Actualiser").clicked() {
                self.refresh_disks();
            }
            if ui.button("Ouvrir une image…").clicked() {
                self.pick_image();
            }
        });
        ui.separator();
        let mut to_open = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.disks.is_empty() {
                ui.label("Aucun disque physique listé.");
            }
            for d in &self.disks {
                let src = DiskSource::Physical(d.path.clone());
                let selected = self.source.as_ref() == Some(&src);
                if ui.selectable_label(selected, disk_list_label(d)).clicked() {
                    to_open = Some(src);
                }
            }
        });
        if let Some(src) = to_open {
            self.open_source(src);
        }
        ui.separator();
        ui.weak(format!("{CONSEIL_ADMIN}."));
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.view.clone() else {
            ui.heading("Aucun disque sélectionné");
            ui.label("Choisissez un disque à gauche ou ouvrez une image.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(view.header_text());
            ui.label(format!("Type de table de partitions : {}", view.table_type));
            for w in &view.warnings {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 140, 0),
                    format!("Avertissement : {w}"),
                );
            }
            if let Some(e) = &view.error {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 50, 50),
                    format!("Erreur : {e}"),
                );
            }
            ui.add_space(6.0);
            if view.partitions.is_empty() {
                if view.table_is_empty() {
                    ui.label("Aucune partition dans la table.");
                }
            } else {
                partition_table(ui, "table_partitions", &view.partitions);
            }
            ui.add_space(6.0);
            let color = if view.structure.ok {
                egui::Color32::from_rgb(40, 150, 60)
            } else {
                egui::Color32::from_rgb(200, 50, 50)
            };
            ui.colored_label(color, view.structure.title());
            for i in &view.structure.issues {
                ui.label(format!("  {i}"));
            }
            ui.separator();
            self.search_section(ui);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Exporter le rapport…").clicked() {
                    self.export_report();
                }
                if ui.button("Copier le rapport").clicked() {
                    if let Some(t) = self.current_report() {
                        ui.ctx().copy_text(t);
                        self.status = "Rapport copié dans le presse-papiers.".to_string();
                    }
                }
            });
        });
    }

    fn search_section(&mut self, ui: &mut egui::Ui) {
        let running = matches!(self.search, SearchState::Running { .. });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Recherche rapide"))
                .clicked()
            {
                self.start_search(ui.ctx());
            }
            // L'API de recherche ne permet pas d'interrompre un balayage en cours :
            // il n'y a donc volontairement pas de bouton « Annuler ».
            if running {
                ui.spinner();
            }
        });
        match &self.search {
            SearchState::Idle => {}
            SearchState::Running { progress, .. } => {
                let (frac, text) = progress.as_ref().map_or_else(
                    || (0.0, "Démarrage...".to_string()),
                    |p| (progress_fraction(p), progress_text(p)),
                );
                ui.add(egui::ProgressBar::new(frac).text(text));
            }
            SearchState::Failed(e) => {
                ui.colored_label(egui::Color32::from_rgb(200, 50, 50), e);
            }
            SearchState::Done(outcome) => {
                if outcome.hits.is_empty() {
                    ui.label("La recherche rapide n'a trouvé aucun système de fichiers.");
                } else {
                    ui.label(format!(
                        "La recherche rapide a trouvé {} système(s) de fichiers :",
                        outcome.hits.len()
                    ));
                    let rows: Vec<_> = outcome.hits.iter().map(|h| h.row.clone()).collect();
                    egui::Grid::new("table_recherche")
                        .striped(true)
                        .num_columns(8)
                        .show(ui, |ui| {
                            partition_header(ui);
                            ui.strong("Statut");
                            ui.end_row();
                            for (row, hit) in rows.iter().zip(&outcome.hits) {
                                partition_cells(ui, row);
                                let color = if hit.in_table {
                                    egui::Color32::from_rgb(40, 150, 60)
                                } else {
                                    egui::Color32::from_rgb(200, 140, 0)
                                };
                                ui.colored_label(color, hit.status_text());
                                ui.end_row();
                            }
                        });
                }
            }
        }
    }
}

impl Default for DiskRustApp {
    fn default() -> Self {
        Self::new()
    }
}

fn partition_header(ui: &mut egui::Ui) {
    for h in [
        "N°",
        "Amorçable",
        "Genre",
        "Type",
        "Début",
        "Fin",
        "Taille",
        "Système de fichiers",
    ] {
        ui.strong(h);
    }
}

fn partition_cells(ui: &mut egui::Ui, r: &crate::model::PartitionRow) {
    ui.label(r.number.to_string());
    ui.label(if r.bootable { "oui" } else { "" });
    ui.label(&r.kind);
    ui.label(&r.type_label);
    ui.label(r.start_lba.to_string());
    ui.label(r.end_lba.to_string());
    ui.label(&r.size_text);
    let fs = match (&r.fs, &r.fs_label) {
        (Some(f), Some(l)) => format!("{f} {l}"),
        (Some(f), None) => f.clone(),
        _ => String::new(),
    };
    ui.label(fs);
}

fn partition_table(ui: &mut egui::Ui, id: &str, rows: &[crate::model::PartitionRow]) {
    egui::Grid::new(id)
        .striped(true)
        .num_columns(8)
        .show(ui, |ui| {
            partition_header(ui);
            ui.end_row();
            for r in rows {
                partition_cells(ui, r);
                ui.end_row();
            }
        });
}

impl eframe::App for DiskRustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_search();
        egui::Panel::top("bandeau").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(40, 150, 60), BANDEAU_LECTURE_SEULE);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("À propos").clicked() {
                        self.about_open = true;
                    }
                });
            });
        });
        egui::Panel::bottom("statut").show(ui, |ui| {
            ui.label(&self.status);
        });
        egui::Panel::left("disques")
            .default_size(280.0)
            .show(ui, |ui| self.left_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.central_panel(ui));

        let ctx = ui.ctx().clone();
        egui::Window::new("À propos")
            .open(&mut self.about_open)
            .collapsible(false)
            .show(&ctx, |ui| {
                ui.heading("diskrust-gui");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("Licence : MIT OU Apache-2.0");
                ui.label("Analyse de disques strictement en lecture seule.");
            });
    }
}

/// Lance la fenêtre.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("diskrust - analyse de disques (lecture seule)")
            .with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "diskrust-gui",
        options,
        Box::new(|_cc| Ok(Box::new(DiskRustApp::new()))),
    )
}
