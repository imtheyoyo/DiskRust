use dr_disk::BlockDevice;

use crate::gpt::{parse_gpt, GPT_SIGNATURE};
use crate::mbr::{parse_mbr, parse_mbr_sector};
use crate::types::{Partition, TableType};

/// Détermine quelle famille de table de partitions est présente.
pub fn detect_table_type(dev: &dyn BlockDevice) -> TableType {
    if dev.sector_size() < 512 {
        return TableType::Unknown;
    }
    let Ok(s0) = dev.read_sectors(0, 1) else {
        return TableType::Unknown;
    };
    let has_gpt_sig = dev
        .read_sectors(1, 1)
        .map(|s| &s[..8] == GPT_SIGNATURE)
        .unwrap_or(false);
    if let Ok(entries) = parse_mbr_sector(&s0) {
        // Un secteur de démarrage de système de fichiers (superfloppy) se termine aussi par 0x55AA.
        if &s0[3..11] == b"NTFS    " || &s0[3..11] == b"EXFAT   " {
            return TableType::Unknown;
        }
        if has_gpt_sig || entries.iter().any(|e| e.type_id == 0xEE) {
            return TableType::Gpt;
        }
        return TableType::Intel;
    }
    if has_gpt_sig {
        return TableType::Gpt;
    }
    if s0.iter().all(|&b| b == 0) {
        TableType::None
    } else {
        TableType::Unknown
    }
}

/// Tout ce qui est connu de la table de partitions d'un disque.
#[derive(Debug, Clone)]
pub struct TableInfo {
    /// Famille détectée.
    pub table_type: TableType,
    /// Partitions (vide en cas d'échec ou pour les disques vierges).
    pub partitions: Vec<Partition>,
    /// Diagnostics (par ex. « Tentative avec la GPT de secours »).
    pub warnings: Vec<String>,
    /// Erreur d'analyse fatale, le cas échéant.
    pub error: Option<String>,
}

/// Détecte et analyse la table de partitions d'un périphérique.
pub fn read_table(dev: &dyn BlockDevice) -> TableInfo {
    let table_type = detect_table_type(dev);
    let mut info = TableInfo {
        table_type,
        partitions: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };
    match table_type {
        TableType::Intel => match parse_mbr(dev) {
            Ok(t) => {
                info.partitions = t.partitions;
                info.warnings = t.warnings;
            }
            Err(e) => info.error = Some(e.to_string()),
        },
        TableType::Gpt => match parse_gpt(dev) {
            Ok(t) => {
                info.partitions = t.partitions;
                info.warnings = t.warnings;
            }
            Err(crate::PartError::NoValidGpt(log)) => {
                info.warnings = log;
                info.error = Some("Aucune GPT valide trouvée".to_string());
            }
            Err(e) => info.error = Some(e.to_string()),
        },
        TableType::None | TableType::Unknown => {}
    }
    info
}
