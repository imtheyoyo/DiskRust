//! Modèle de l'interface : toute la logique hors affichage.
//!
//! Charge un disque (image ou périphérique, toujours en lecture seule) dans un
//! [`DiskView`], lance la recherche rapide et produit les rapports textuels en
//! réutilisant `dr-cli`. Aucune dépendance graphique : testable sans écran.

use std::io;
use std::path::PathBuf;

use dr_disk::{format_size, BlockDevice, DiskInfo, FileImage, PhysicalDisk};
use dr_fs::probe;
use dr_part::{read_table, type_label, Partition, TableType};
use dr_search::{check_structure_on_disk, quick_search, Progress, SearchOptions};

/// Bandeau affiché en permanence.
pub const BANDEAU_LECTURE_SEULE: &str =
    "Lecture seule : aucune écriture n'est effectuée sur le disque.";

/// Conseil affiché pour la liste des disques physiques.
pub const CONSEIL_ADMIN: &str =
    "Pour lister les disques physiques, lancez le programme en tant qu'administrateur";

/// Origine d'un disque : toujours ouvert en lecture seule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskSource {
    /// Périphérique physique (`\\.\PhysicalDrive0`, `/dev/sda`).
    Physical(String),
    /// Image brute `.dd` / `.img` / `.raw`.
    Image(PathBuf),
}

impl DiskSource {
    /// Nom affiché pour la source.
    pub fn label(&self) -> String {
        match self {
            DiskSource::Physical(p) => p.clone(),
            DiskSource::Image(p) => p.display().to_string(),
        }
    }

    /// Ouvre la source en lecture seule.
    pub fn open(&self) -> io::Result<Box<dyn BlockDevice>> {
        match self {
            DiskSource::Physical(p) => Ok(Box::new(PhysicalDisk::open(p)?)),
            DiskSource::Image(p) => Ok(Box::new(FileImage::open(p)?)),
        }
    }
}

/// Libellé d'un disque de la liste de gauche : chemin + taille.
pub fn disk_list_label(info: &DiskInfo) -> String {
    let mut s = format!("{} ({})", info.path, format_size(info.size_bytes));
    if let Some(m) = info.model.as_deref().filter(|m| !m.trim().is_empty()) {
        s.push_str(&format!(" - {}", m.trim()));
    }
    s
}

/// Une ligne de la table des partitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionRow {
    /// Numéro affiché (à partir de 1).
    pub number: usize,
    /// Partition amorçable.
    pub bootable: bool,
    /// Genre (Primaire, Logique, ...).
    pub kind: String,
    /// Description du type.
    pub type_label: String,
    /// Premier secteur.
    pub start_lba: u64,
    /// Dernier secteur.
    pub end_lba: u64,
    /// Taille en secteurs.
    pub size_sectors: u64,
    /// Taille lisible.
    pub size_text: String,
    /// Système de fichiers détecté (nom), le cas échéant.
    pub fs: Option<String>,
    /// Étiquette du volume, le cas échéant.
    pub fs_label: Option<String>,
}

impl PartitionRow {
    fn new(number: usize, p: &Partition, dev: &dyn BlockDevice) -> Self {
        let fs = probe(dev, p.start_lba);
        Self {
            number,
            bootable: p.bootable,
            kind: p.kind.to_string(),
            type_label: type_label(p),
            start_lba: p.start_lba,
            end_lba: p.end_lba(),
            size_sectors: p.size_sectors,
            size_text: format_size(p.size_sectors.saturating_mul(u64::from(dev.sector_size()))),
            fs: fs.as_ref().map(|f| f.kind.to_string()),
            fs_label: fs.and_then(|f| f.label),
        }
    }
}

/// Résultat de la vérification de la structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureView {
    /// Aucun problème détecté.
    pub ok: bool,
    /// Descriptions des problèmes.
    pub issues: Vec<String>,
}

impl StructureView {
    /// Texte « Structure : Ok » ou « Structure : Mauvaise ».
    pub fn title(&self) -> &'static str {
        if self.ok {
            "Structure : Ok"
        } else {
            "Structure : Mauvaise"
        }
    }
}

/// Tout ce que l'interface affiche pour un disque.
#[derive(Debug, Clone)]
pub struct DiskView {
    /// Nom du disque (chemin).
    pub label: String,
    /// Taille en octets.
    pub size_bytes: u64,
    /// Octets par secteur.
    pub sector_size: u32,
    /// Famille de la table de partitions.
    pub table_type: TableType,
    /// Avertissements de l'analyse.
    pub warnings: Vec<String>,
    /// Erreur fatale d'analyse.
    pub error: Option<String>,
    /// Partitions de la table.
    pub partitions: Vec<PartitionRow>,
    /// Partitions brutes (pour comparer avec la recherche).
    pub raw_partitions: Vec<Partition>,
    /// Vérification de structure.
    pub structure: StructureView,
    /// Rapport textuel de `diskrust list`.
    pub report: String,
}

impl DiskView {
    /// Ligne d'en-tête : taille et octets par secteur.
    pub fn header_text(&self) -> String {
        format!(
            "{} - {} - {} octets/secteur",
            self.label,
            format_size(self.size_bytes),
            self.sector_size
        )
    }

    /// Vrai si la table est vide alors qu'elle est reconnue (aucune partition).
    pub fn table_is_empty(&self) -> bool {
        self.partitions.is_empty()
            && matches!(self.table_type, TableType::Intel | TableType::Gpt)
            && self.error.is_none()
    }
}

/// Charge un disque déjà ouvert dans un [`DiskView`].
pub fn load_view(label: &str, dev: &dyn BlockDevice) -> DiskView {
    let info = read_table(dev);
    let structure = check_structure_on_disk(&info.partitions, Some(dev.sector_count()));
    let partitions = info
        .partitions
        .iter()
        .enumerate()
        .map(|(i, p)| PartitionRow::new(i + 1, p, dev))
        .collect();
    DiskView {
        label: label.to_string(),
        size_bytes: dev.size_bytes(),
        sector_size: dev.sector_size(),
        table_type: info.table_type,
        warnings: info.warnings,
        error: info.error,
        partitions,
        raw_partitions: info.partitions,
        structure: StructureView {
            ok: structure.is_ok(),
            issues: structure.issues.iter().map(|i| i.to_string()).collect(),
        },
        report: dr_cli::list_report(label, dev).join("\n"),
    }
}

/// Ouvre une source (lecture seule) et la charge.
pub fn load_source(source: &DiskSource) -> io::Result<DiskView> {
    let dev = source.open()?;
    Ok(load_view(&source.label(), dev.as_ref()))
}

/// Un système de fichiers trouvé par la recherche rapide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Description de la partition trouvée.
    pub row: PartitionRow,
    /// Présente dans la table de partitions ; sinon candidate à la récupération.
    pub in_table: bool,
}

impl SearchHit {
    /// Mention « dans la table » ou « candidat à la récupération ».
    pub fn status_text(&self) -> &'static str {
        if self.in_table {
            "dans la table"
        } else {
            "candidat à la récupération"
        }
    }
}

/// Résultat complet d'une recherche rapide.
#[derive(Debug, Clone)]
pub struct SearchOutcome {
    /// Systèmes de fichiers trouvés.
    pub hits: Vec<SearchHit>,
    /// Rapport textuel complet (équivalent de `diskrust analyse`).
    pub report: String,
}

/// Lance la recherche rapide sur `dev` et la compare à la table de `view`.
///
/// Le rapport textuel est produit par `dr_cli::analyse_report` (un second
/// balayage, sans progression, garantit un texte identique à la ligne de commande).
pub fn run_search(
    dev: &dyn BlockDevice,
    view: &DiskView,
    progress: &mut dyn FnMut(&Progress),
) -> SearchOutcome {
    let found = quick_search(dev, &SearchOptions::default(), progress);
    let hits = found
        .iter()
        .enumerate()
        .map(|(i, p)| SearchHit {
            row: PartitionRow::new(i + 1, p, dev),
            in_table: view
                .raw_partitions
                .iter()
                .any(|t| t.start_lba == p.start_lba && t.size_sectors == p.size_sectors),
        })
        .collect();
    let report = dr_cli::analyse_report(&view.label, dev, &mut |_| {}).join("\n");
    SearchOutcome { hits, report }
}

/// Texte de progression, par exemple « 42 % - 2 trouvé(s) ».
pub fn progress_text(p: &Progress) -> String {
    format!("{} % - {} trouvé(s)", p.percent(), p.found)
}

/// Fraction 0.0..=1.0 pour la barre de progression.
pub fn progress_fraction(p: &Progress) -> f32 {
    p.percent() as f32 / 100.0
}

/// Texte du rapport à exporter : celui de l'analyse si la recherche a eu lieu, sinon la liste.
pub fn report_text(view: &DiskView, outcome: Option<&SearchOutcome>) -> String {
    let mut s = outcome.map_or_else(|| view.report.clone(), |o| o.report.clone());
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use dr_disk::MemImage;
    use dr_fs::synth::{fat32_boot_sector, ntfs_boot_sector};
    use dr_part::synth::{gpt_image, mbr_entry, mbr_sector};

    const LINUX: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

    /// FAT32 à 2048 (dans la table) ; NTFS à 73728 absent de la table.
    fn disk_with_lost_partition() -> MemImage {
        let mut img = MemImage::zeroed(512 * 160_000);
        img.put(2048 * 512, &fat32_boot_sector(70_000));
        img.put(73_728 * 512, &ntfs_boot_sector(60_000, 4));
        img.put(0, &mbr_sector(&[mbr_entry(0x80, 0x0C, 2048, 70_000)]));
        img
    }

    #[test]
    fn view_of_mbr_disk() {
        let img = disk_with_lost_partition();
        let v = load_view("test.dd", &img);
        assert_eq!(v.table_type, TableType::Intel);
        assert_eq!(v.sector_size, 512);
        assert_eq!(v.partitions.len(), 1);
        let p = &v.partitions[0];
        assert_eq!((p.number, p.start_lba, p.size_sectors), (1, 2048, 70_000));
        assert_eq!(p.end_lba, 2048 + 70_000 - 1);
        assert!(p.bootable);
        assert_eq!(p.fs.as_deref(), Some("FAT32"));
        assert!(v.structure.ok);
        assert_eq!(v.structure.title(), "Structure : Ok");
        assert!(v.header_text().contains("512 octets/secteur"));
        assert!(v.report.contains("Type de table de partitions : Intel"));
    }

    #[test]
    fn view_of_gpt_disk() {
        let img = MemImage::new(gpt_image(4096, &[(LINUX, 2048, 4000, "donnees")]));
        let v = load_view("gpt.img", &img);
        assert_eq!(v.table_type, TableType::Gpt);
        assert_eq!(v.partitions.len(), 1);
        assert_eq!(v.partitions[0].start_lba, 2048);
        assert!(v.error.is_none());
    }

    #[test]
    fn empty_disk_has_no_partitions() {
        let v = load_view("vide.dd", &MemImage::zeroed(512 * 64));
        assert_eq!(v.table_type, TableType::None);
        assert!(v.partitions.is_empty());
        assert!(!v.table_is_empty());
    }

    #[test]
    fn bad_structure_is_reported() {
        let mut img = MemImage::zeroed(512 * 100_000);
        img.put(
            0,
            &mbr_sector(&[
                mbr_entry(0, 0x83, 2048, 20_000),
                mbr_entry(0, 0x83, 10_000, 20_000),
            ]),
        );
        let v = load_view("chevauche.dd", &img);
        assert!(!v.structure.ok);
        assert_eq!(v.structure.title(), "Structure : Mauvaise");
        assert!(!v.structure.issues.is_empty());
    }

    #[test]
    fn search_marks_table_and_candidates() {
        let img = disk_with_lost_partition();
        let v = load_view("test.dd", &img);
        let mut last = None;
        let out = run_search(&img, &v, &mut |p: &Progress| last = Some(*p));
        assert_eq!(out.hits.len(), 2, "{:?}", out.hits);
        assert!(out.hits[0].in_table);
        assert_eq!(out.hits[0].status_text(), "dans la table");
        assert!(!out.hits[1].in_table);
        assert_eq!(out.hits[1].status_text(), "candidat à la récupération");
        assert_eq!(out.hits[1].row.fs.as_deref(), Some("NTFS"));
        assert_eq!(last.map(|p| p.percent()), Some(100));
        assert!(out.report.contains("candidat à la récupération"));
        assert!(report_text(&v, Some(&out)).contains("Analyse"));
        assert!(!report_text(&v, None).contains("Analyse"));
    }

    #[test]
    fn progress_helpers() {
        let p = Progress {
            current_lba: 50,
            total_lba: 200,
            found: 3,
        };
        assert_eq!(progress_text(&p), "25 % - 3 trouvé(s)");
        assert!((progress_fraction(&p) - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn disk_label_has_path_and_size() {
        let info = DiskInfo {
            path: "/dev/sda".into(),
            size_bytes: 512 * 1000,
            sector_size: 512,
            model: Some("  Mon disque ".into()),
        };
        let l = disk_list_label(&info);
        assert!(l.starts_with("/dev/sda ("));
        assert!(l.ends_with("- Mon disque"));
    }

    #[test]
    fn image_source_opens_read_only() {
        let dir = std::env::temp_dir().join(format!("dr-gui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.dd");
        std::fs::write(&path, disk_with_lost_partition().bytes()).unwrap();
        let src = DiskSource::Image(path.clone());
        let v = load_source(&src).unwrap();
        assert_eq!(v.partitions.len(), 1);
        assert!(load_source(&DiskSource::Image(dir.join("absent.dd"))).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
