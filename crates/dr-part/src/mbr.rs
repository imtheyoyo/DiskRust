//! MBR + extended partition (EBR chain) parsing.

use std::collections::HashSet;
use std::fmt;

use dr_disk::BlockDevice;

use crate::types::{PartError, PartKind, Partition, TypeId};

/// Default translation geometry used for CHS consistency checks.
pub const DEFAULT_HEADS: u32 = 255;
/// Default sectors-per-track used for CHS consistency checks.
pub const DEFAULT_SECTORS_PER_TRACK: u32 = 63;
/// Upper bound on EBR links followed (loop / runaway protection).
const MAX_LOGICAL: usize = 256;

/// A decoded cylinder/head/sector triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chs {
    /// Cylinder (0..=1023).
    pub cylinder: u16,
    /// Head (0..=255).
    pub head: u8,
    /// Sector (1..=63, 0 means invalid).
    pub sector: u8,
}

impl Chs {
    /// Decode the 3-byte on-disk form.
    pub fn decode(b: [u8; 3]) -> Self {
        Chs {
            head: b[0],
            sector: b[1] & 0x3F,
            cylinder: (u16::from(b[1] & 0xC0) << 2) | u16::from(b[2]),
        }
    }

    /// Convert to an LBA under the given geometry.
    pub fn to_lba(self, heads: u32, spt: u32) -> Option<u64> {
        if self.sector == 0 {
            return None;
        }
        Some(
            (u64::from(self.cylinder) * u64::from(heads) + u64::from(self.head)) * u64::from(spt)
                + u64::from(self.sector)
                - 1,
        )
    }

    /// CHS values saturate at cylinder 1023 for large disks and are then meaningless.
    pub fn is_saturated(self) -> bool {
        self.cylinder >= 1023
    }
}

impl fmt::Display for Chs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.cylinder, self.head, self.sector)
    }
}

/// One raw 16-byte partition entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrEntry {
    /// Boot flag byte (0x80 active, 0x00 inactive).
    pub boot: u8,
    /// CHS of first sector.
    pub chs_start: Chs,
    /// System id.
    pub type_id: u8,
    /// CHS of last sector.
    pub chs_end: Chs,
    /// Start LBA (relative to the table's base).
    pub start_lba: u32,
    /// Size in sectors.
    pub size_sectors: u32,
}

impl MbrEntry {
    fn parse(b: &[u8]) -> Self {
        MbrEntry {
            boot: b[0],
            chs_start: Chs::decode([b[1], b[2], b[3]]),
            type_id: b[4],
            chs_end: Chs::decode([b[5], b[6], b[7]]),
            start_lba: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            size_sectors: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
        }
    }

    /// Unused slot.
    pub fn is_empty(&self) -> bool {
        self.type_id == 0 || self.size_sectors == 0
    }

    /// DOS/Win95/Linux extended container.
    pub fn is_extended(&self) -> bool {
        matches!(self.type_id, 0x05 | 0x0F | 0x85)
    }
}

/// Result of parsing an Intel partition table.
#[derive(Debug, Clone, Default)]
pub struct MbrTable {
    /// Primary, extended and logical partitions in discovery order.
    pub partitions: Vec<Partition>,
    /// Non-fatal diagnostics (CHS mismatches, loops, out-of-range values ...).
    pub warnings: Vec<String>,
}

/// Parse the four entries of a 512-byte boot sector; requires the 0x55AA signature.
pub fn parse_mbr_sector(sec: &[u8]) -> Result<[MbrEntry; 4], PartError> {
    if sec.len() < 512 || sec[510] != 0x55 || sec[511] != 0xAA {
        return Err(PartError::BadMbrSignature);
    }
    let e = |i: usize| MbrEntry::parse(&sec[446 + 16 * i..446 + 16 * (i + 1)]);
    Ok([e(0), e(1), e(2), e(3)])
}

/// Human readable MBR system id.
pub fn mbr_type_name(t: u8) -> &'static str {
    match t {
        0x00 => "Empty",
        0x01 => "FAT12",
        0x04 => "FAT16 <32M",
        0x05 => "Extended",
        0x06 => "FAT16",
        0x07 => "HPFS/NTFS/exFAT",
        0x0B => "FAT32",
        0x0C => "FAT32 LBA",
        0x0E => "FAT16 LBA",
        0x0F => "Extended LBA",
        0x11 => "Hidden FAT12",
        0x14 => "Hidden FAT16 <32M",
        0x16 => "Hidden FAT16",
        0x17 => "Hidden HPFS/NTFS",
        0x1B => "Hidden FAT32",
        0x1C => "Hidden FAT32 LBA",
        0x1E => "Hidden FAT16 LBA",
        0x27 => "Hidden NTFS (WinRE)",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x85 => "Linux extended",
        0x8E => "Linux LVM",
        0xA5 => "FreeBSD",
        0xA6 => "OpenBSD",
        0xA9 => "NetBSD",
        0xAF => "Apple HFS/HFS+",
        0xEE => "EFI GPT protective",
        0xEF => "EFI System",
        0xFD => "Linux RAID autodetect",
        _ => "Unknown",
    }
}

fn check_entry(
    label: &str,
    e: &MbrEntry,
    abs_start: u64,
    disk_sectors: u64,
    warnings: &mut Vec<String>,
) {
    if e.boot != 0x00 && e.boot != 0x80 {
        warnings.push(format!(
            "{label}: invalid boot flag 0x{:02X} (expected 0x00 or 0x80)",
            e.boot
        ));
    }
    let (h, s) = (DEFAULT_HEADS, DEFAULT_SECTORS_PER_TRACK);
    if !e.chs_start.is_saturated() {
        if let Some(lba) = e.chs_start.to_lba(h, s) {
            if lba != abs_start {
                warnings.push(format!(
                    "{label}: CHS/LBA mismatch at start: CHS {} = LBA {lba}, but LBA field = {abs_start}",
                    e.chs_start
                ));
            }
        }
    }
    let abs_end = abs_start + u64::from(e.size_sectors) - 1;
    if !e.chs_end.is_saturated() {
        if let Some(lba) = e.chs_end.to_lba(h, s) {
            if lba != abs_end {
                warnings.push(format!(
                    "{label}: CHS/LBA mismatch at end: CHS {} = LBA {lba}, but LBA field = {abs_end}",
                    e.chs_end
                ));
            }
        }
    }
    if abs_end >= disk_sectors {
        warnings.push(format!(
            "{label}: partition ends at LBA {abs_end}, beyond end of disk ({disk_sectors} sectors)"
        ));
    }
}

/// Parse the MBR at LBA 0 and follow the extended partition's EBR chain.
pub fn parse_mbr(dev: &dyn BlockDevice) -> Result<MbrTable, PartError> {
    let sec0 = dev.read_sectors(0, 1)?;
    let entries = parse_mbr_sector(&sec0)?;
    let disk_sectors = dev.sector_count();
    let mut table = MbrTable::default();
    let mut extended: Option<(u64, u64)> = None;

    for (i, e) in entries.iter().enumerate() {
        if e.is_empty() {
            if e.type_id != 0 {
                table.warnings.push(format!(
                    "Partition {}: type 0x{:02X} with zero size",
                    i + 1,
                    e.type_id
                ));
            }
            continue;
        }
        let label = format!("Partition {}", i + 1);
        let start = u64::from(e.start_lba);
        check_entry(&label, e, start, disk_sectors, &mut table.warnings);
        let kind = if e.is_extended() {
            if extended.is_some() {
                table
                    .warnings
                    .push(format!("{label}: more than one extended partition"));
            } else {
                extended = Some((start, u64::from(e.size_sectors)));
            }
            PartKind::Extended
        } else {
            PartKind::Primary
        };
        table.partitions.push(Partition {
            start_lba: start,
            size_sectors: u64::from(e.size_sectors),
            kind,
            type_id: TypeId::Mbr(e.type_id),
            name: mbr_type_name(e.type_id).to_string(),
            bootable: e.boot == 0x80,
        });
    }

    if let Some((start, size)) = extended {
        walk_ebr_chain(dev, start, size, disk_sectors, &mut table);
    }
    Ok(table)
}

fn walk_ebr_chain(
    dev: &dyn BlockDevice,
    ext_start: u64,
    ext_size: u64,
    disk_sectors: u64,
    table: &mut MbrTable,
) {
    let mut visited = HashSet::new();
    let mut rel = 0u64; // link offsets are always relative to the extended partition start
    for n in 0..MAX_LOGICAL {
        let ebr_lba = ext_start + rel;
        if !visited.insert(ebr_lba) {
            table
                .warnings
                .push(format!("EBR chain loop detected at LBA {ebr_lba}"));
            return;
        }
        if ebr_lba >= ext_start + ext_size || ebr_lba >= disk_sectors {
            table.warnings.push(format!(
                "EBR at LBA {ebr_lba} lies outside the extended partition"
            ));
            return;
        }
        let sec = match dev.read_sectors(ebr_lba, 1) {
            Ok(s) => s,
            Err(e) => {
                table
                    .warnings
                    .push(format!("Cannot read EBR at LBA {ebr_lba}: {e}"));
                return;
            }
        };
        let entries = match parse_mbr_sector(&sec) {
            Ok(e) => e,
            Err(_) => {
                table
                    .warnings
                    .push(format!("EBR at LBA {ebr_lba}: missing 0x55AA signature"));
                return;
            }
        };
        let mut next = None;
        for e in entries.iter().filter(|e| !e.is_empty()) {
            if e.is_extended() {
                next.get_or_insert(u64::from(e.start_lba));
                continue;
            }
            let abs = ebr_lba + u64::from(e.start_lba);
            let label = format!("Logical {}", n + 5);
            check_entry(&label, e, abs, disk_sectors, &mut table.warnings);
            table.partitions.push(Partition {
                start_lba: abs,
                size_sectors: u64::from(e.size_sectors),
                kind: PartKind::Logical,
                type_id: TypeId::Mbr(e.type_id),
                name: mbr_type_name(e.type_id).to_string(),
                bootable: e.boot == 0x80,
            });
        }
        match next {
            Some(r) => rel = r,
            None => return,
        }
    }
    table.warnings.push(format!(
        "EBR chain longer than {MAX_LOGICAL} entries, giving up"
    ));
}
