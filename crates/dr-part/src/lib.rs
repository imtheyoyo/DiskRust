//! Analyseurs de tables de partitions : MBR/EBR (Intel) et GPT.
//!
//! Implémentés à partir des spécifications publiques des formats (spécification
//! UEFI pour GPT, disposition MBR classique des PC pour les tables Intel).

mod detect;
mod gpt;
mod mbr;
#[doc(hidden)]
pub mod synth;
mod types;

pub use detect::{detect_table_type, read_table, TableInfo};
pub use gpt::{
    format_guid, gpt_type_name, parse_gpt, parse_guid, GptError, GptHeader, GptTable, GPT_SIGNATURE,
};
pub use mbr::{
    mbr_type_name, parse_mbr, parse_mbr_sector, Chs, MbrEntry, MbrTable, DEFAULT_HEADS,
    DEFAULT_SECTORS_PER_TRACK,
};
pub use types::{type_label, PartError, PartKind, Partition, TableType, TypeId};

#[cfg(test)]
mod tests;
