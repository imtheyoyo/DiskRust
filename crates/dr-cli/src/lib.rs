//! Génération des rapports de la ligne de commande `diskrust` (strictement en lecture seule).
//!
//! À FAIRE (écriture) : toute future opération qui modifie un disque (écriture
//! d'une table de partitions, réparation de secteurs de démarrage) devra être
//! protégée par un drapeau de confirmation explicite et une implémentation
//! distincte de `WritableDevice`. Rien dans cette crate n'ouvre un périphérique
//! en écriture.

use dr_disk::{format_size, BlockDevice};
use dr_fs::{probe, FsInfo};
use dr_part::{read_table, type_label, Partition, TableType};
use dr_search::{check_structure_on_disk, quick_search, Progress, SearchOptions};

/// Ligne d'en-tête `Disque <nom> - <taille> - <taille de secteur>`.
pub fn disk_header(label: &str, dev: &dyn BlockDevice) -> String {
    format!(
        "Disque {label} - {} - {} octets/secteur",
        format_size(dev.size_bytes()),
        dev.sector_size()
    )
}

fn table_header() -> String {
    format!(
        "     {:<10} {:<30} {:>12} {:>12} {:>12}",
        "Genre", "Type", "Début", "Fin", "Taille (sect.)"
    )
}

fn partition_row(index: usize, p: &Partition, fs: Option<&FsInfo>) -> String {
    let mut row = format!(
        "{:>3} {} {:<10} {:<30} {:>12} {:>12} {:>12}",
        index + 1,
        if p.bootable { '*' } else { ' ' },
        p.kind.to_string(),
        type_label(p),
        p.start_lba,
        p.end_lba(),
        p.size_sectors
    );
    if let Some(fs) = fs {
        row.push_str(&format!("  [{}]", fs.kind));
        if let Some(l) = &fs.label {
            row.push_str(&format!(" {l}"));
        }
    } else if !p.name.is_empty() && p.name != type_label(p) {
        row.push_str(&format!("  {}", p.name));
    }
    row
}

/// Sortie de `diskrust list` pour un disque (liste des disques et partitions).
pub fn list_report(label: &str, dev: &dyn BlockDevice) -> Vec<String> {
    let mut out = vec![disk_header(label, dev)];
    let info = read_table(dev);
    out.push(format!("Type de table de partitions : {}", info.table_type));
    for w in &info.warnings {
        out.push(format!("Avertissement : {w}"));
    }
    if let Some(e) = &info.error {
        out.push(format!("Erreur : {e}"));
    }
    if !info.partitions.is_empty() {
        out.push(table_header());
        for (i, p) in info.partitions.iter().enumerate() {
            let fs = probe(dev, p.start_lba);
            out.push(partition_row(i, p, fs.as_ref()));
        }
    } else if matches!(info.table_type, TableType::Intel | TableType::Gpt) && info.error.is_none() {
        out.push("Aucune partition dans la table.".to_string());
    }
    out
}

/// Sortie de `diskrust analyse` : table + vérification de la structure + recherche rapide.
pub fn analyse_report(
    label: &str,
    dev: &dyn BlockDevice,
    progress: &mut dyn FnMut(&Progress),
) -> Vec<String> {
    let mut out = vec![disk_header(label, dev)];
    let info = read_table(dev);
    out.push(format!("Type de table de partitions : {}", info.table_type));
    for w in &info.warnings {
        out.push(format!("Avertissement : {w}"));
    }
    if let Some(e) = &info.error {
        out.push(format!("Erreur : {e}"));
    }
    if !info.partitions.is_empty() {
        out.push(table_header());
        for (i, p) in info.partitions.iter().enumerate() {
            out.push(partition_row(i, p, probe(dev, p.start_lba).as_ref()));
        }
    }
    let report = check_structure_on_disk(&info.partitions, Some(dev.sector_count()));
    if report.is_ok() {
        out.push("Structure : Ok".to_string());
    } else {
        out.push("Structure : Mauvaise".to_string());
        for i in &report.issues {
            out.push(format!("  {i}"));
        }
    }

    out.push("Analyse : recherche rapide".to_string());
    let found = quick_search(dev, &SearchOptions::default(), progress);
    if found.is_empty() {
        out.push("La recherche rapide n'a trouvé aucun système de fichiers.".to_string());
    } else {
        out.push(format!(
            "La recherche rapide a trouvé {} système(s) de fichiers :",
            found.len()
        ));
        out.push(table_header());
        for (i, p) in found.iter().enumerate() {
            let listed = info
                .partitions
                .iter()
                .any(|t| t.start_lba == p.start_lba && t.size_sectors == p.size_sectors);
            let mut row = partition_row(i, p, probe(dev, p.start_lba).as_ref());
            row.push_str(if listed {
                "  (dans la table de partitions)"
            } else {
                "  (ABSENT de la table de partitions : candidat à la récupération)"
            });
            out.push(row);
        }
    }
    out.push("Analyse en lecture seule : rien n'a été écrit sur le disque.".to_string());
    out
}
