//! Partition table parsers: MBR/EBR (Intel) and GPT.
//!
//! Implemented from the public format specifications (UEFI spec for GPT,
//! the classic PC MBR layout for Intel tables).

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
