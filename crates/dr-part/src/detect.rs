use dr_disk::BlockDevice;

use crate::gpt::{parse_gpt, GPT_SIGNATURE};
use crate::mbr::{parse_mbr, parse_mbr_sector};
use crate::types::{Partition, TableType};

/// Decide which partition table family is present.
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
        // A filesystem boot sector (superfloppy) also ends in 0x55AA.
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

/// Everything known about a disk's partition table.
#[derive(Debug, Clone)]
pub struct TableInfo {
    /// Detected family.
    pub table_type: TableType,
    /// Partitions (empty on failure or for blank disks).
    pub partitions: Vec<Partition>,
    /// Diagnostics (e.g. "Trying alternate GPT").
    pub warnings: Vec<String>,
    /// Fatal parse error, if any.
    pub error: Option<String>,
}

/// Detect and parse the partition table of a device.
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
                info.error = Some("No valid GPT found".to_string());
            }
            Err(e) => info.error = Some(e.to_string()),
        },
        TableType::None | TableType::Unknown => {}
    }
    info
}
