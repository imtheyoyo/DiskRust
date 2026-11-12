use std::fmt;
use std::io;

use crate::gpt::{format_guid, gpt_type_name, GptError};
use crate::mbr::mbr_type_name;

/// How a partition is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Primary MBR partition.
    Primary,
    /// Extended container partition (MBR).
    Extended,
    /// Logical partition inside an extended partition (EBR chain).
    Logical,
    /// GPT partition.
    Gpt,
}

impl fmt::Display for PartKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PartKind::Primary => "Primary",
            PartKind::Extended => "Extended",
            PartKind::Logical => "Logical",
            PartKind::Gpt => "GPT",
        })
    }
}

/// Partition type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeId {
    /// One-byte MBR system id.
    Mbr(u8),
    /// GPT type GUID, raw on-disk (mixed-endian) bytes.
    Gpt([u8; 16]),
}

/// A partition described by a table (or discovered by a search).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// First sector (absolute LBA).
    pub start_lba: u64,
    /// Length in sectors.
    pub size_sectors: u64,
    /// Storage kind.
    pub kind: PartKind,
    /// Type id (MBR byte or GPT GUID).
    pub type_id: TypeId,
    /// Human readable name (GPT name, or type description for MBR).
    pub name: String,
    /// Active/bootable flag (MBR 0x80, GPT legacy-BIOS-bootable attribute).
    pub bootable: bool,
}

impl Partition {
    /// Last sector (inclusive). Equal to `start_lba` for empty partitions.
    pub fn end_lba(&self) -> u64 {
        self.start_lba + self.size_sectors.saturating_sub(1)
    }
}

/// Partition table family found on a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableType {
    /// Intel/PC (MBR) partition table.
    Intel,
    /// EFI GPT.
    Gpt,
    /// Blank disk (no table).
    None,
    /// Something is there but it is not a recognised table.
    Unknown,
}

impl fmt::Display for TableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TableType::Intel => "Intel",
            TableType::Gpt => "EFI GPT",
            TableType::None => "None",
            TableType::Unknown => "Unknown",
        })
    }
}

/// Human readable type description of a partition.
pub fn type_label(p: &Partition) -> String {
    match p.type_id {
        TypeId::Mbr(t) => mbr_type_name(t).to_string(),
        TypeId::Gpt(g) => gpt_type_name(&g)
            .map(str::to_string)
            .unwrap_or_else(|| format_guid(&g)),
    }
}

/// Errors produced by the table parsers.
#[derive(Debug)]
pub enum PartError {
    /// Underlying I/O failure.
    Io(io::Error),
    /// Sector 0 does not end with 0x55AA.
    BadMbrSignature,
    /// No usable GPT (primary and backup both bad). Carries the diagnostic log.
    NoValidGpt(Vec<String>),
    /// A specific GPT problem.
    Gpt(GptError),
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartError::Io(e) => write!(f, "I/O error: {e}"),
            PartError::BadMbrSignature => f.write_str("Bad MBR: missing 0x55AA signature"),
            PartError::NoValidGpt(log) => {
                write!(f, "No valid GPT found ({})", log.join("; "))
            }
            PartError::Gpt(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PartError {}

impl From<io::Error> for PartError {
    fn from(e: io::Error) -> Self {
        PartError::Io(e)
    }
}

impl From<GptError> for PartError {
    fn from(e: GptError) -> Self {
        PartError::Gpt(e)
    }
}
