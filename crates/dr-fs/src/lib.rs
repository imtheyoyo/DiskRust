//! Détection de signatures de systèmes de fichiers à partir des secteurs de démarrage et des superblocs.
//!
//! Seuls les *signatures et champs de géométrie* des structures sur disque sont
//! inspectés ; aucun système de fichiers n'est monté ni parcouru. Implémenté à
//! partir des spécifications publiques Microsoft FAT/exFAT, de la disposition du
//! secteur de démarrage NTFS et de la documentation du format sur disque ext2/3/4.

use std::fmt;

use dr_disk::BlockDevice;

#[doc(hidden)]
pub mod synth;

/// Famille de système de fichiers détectée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    /// FAT12
    Fat12,
    /// FAT16
    Fat16,
    /// FAT32
    Fat32,
    /// NTFS
    Ntfs,
    /// exFAT
    ExFat,
    /// ext2
    Ext2,
    /// ext3
    Ext3,
    /// ext4
    Ext4,
}

impl fmt::Display for FsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FsKind::Fat12 => "FAT12",
            FsKind::Fat16 => "FAT16",
            FsKind::Fat32 => "FAT32",
            FsKind::Ntfs => "NTFS",
            FsKind::ExFat => "exFAT",
            FsKind::Ext2 => "ext2",
            FsKind::Ext3 => "ext3",
            FsKind::Ext4 => "ext4",
        })
    }
}

impl FsKind {
    /// Identifiant système MBR raisonnable pour ce système de fichiers.
    pub fn mbr_type(self) -> u8 {
        match self {
            FsKind::Fat12 => 0x01,
            FsKind::Fat16 => 0x06,
            FsKind::Fat32 => 0x0C,
            FsKind::Ntfs | FsKind::ExFat => 0x07,
            FsKind::Ext2 | FsKind::Ext3 | FsKind::Ext4 => 0x83,
        }
    }
}

/// Informations obtenues sur un système de fichiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInfo {
    /// Famille de système de fichiers.
    pub kind: FsKind,
    /// Unité d'allocation (cluster / bloc) en octets.
    pub block_size: u32,
    /// Taille du volume en secteurs *du périphérique*.
    pub size_sectors: u64,
    /// Étiquette du volume, si présente dans le secteur de démarrage / superbloc.
    pub label: Option<String>,
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn has_boot_sig(b: &[u8]) -> bool {
    b.len() >= 512 && b[510] == 0x55 && b[511] == 0xAA
}

fn label_from(raw: &[u8]) -> Option<String> {
    let s: String = raw
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| {
            if c.is_ascii_graphic() || c == b' ' {
                c as char
            } else {
                '?'
            }
        })
        .collect();
    let s = s.trim_end().to_string();
    if s.is_empty() || s == "NO NAME" {
        None
    } else {
        Some(s)
    }
}

fn to_dev_sectors(bytes: u64, dev_ss: u32) -> u64 {
    bytes / u64::from(dev_ss.max(1))
}

/// Valide un BPB FAT12/16/32 (premier secteur du volume) et le classe selon
/// le nombre de clusters, comme le prescrit la spécification Microsoft.
pub fn detect_fat(sector: &[u8], dev_ss: u32) -> Option<FsInfo> {
    if !has_boot_sig(sector) {
        return None;
    }
    if !(sector[0] == 0xEB || sector[0] == 0xE9) {
        return None;
    }
    let bps = u32::from(le16(sector, 11));
    let spc = u32::from(sector[13]);
    let reserved = u32::from(le16(sector, 14));
    let nfats = u32::from(sector[16]);
    let root_entries = u32::from(le16(sector, 17));
    let total16 = u32::from(le16(sector, 19));
    let media = sector[21];
    let fat16 = u32::from(le16(sector, 22));
    let total32 = le32(sector, 32);
    let fat32 = le32(sector, 36);

    if !(512..=4096).contains(&bps) || !bps.is_power_of_two() {
        return None;
    }
    if spc == 0 || !spc.is_power_of_two() || spc * bps > 128 * 1024 {
        return None;
    }
    if reserved == 0 || !(1..=2).contains(&nfats) {
        return None;
    }
    if media != 0xF0 && media < 0xF8 {
        return None;
    }
    let total = if total16 != 0 { total16 } else { total32 };
    let fat_size = if fat16 != 0 { fat16 } else { fat32 };
    if total == 0 || fat_size == 0 {
        return None;
    }
    let is32_layout = fat16 == 0;
    if is32_layout {
        // FAT32 : pas de répertoire racine fixe, cluster racine >= 2.
        if root_entries != 0 || le32(sector, 44) < 2 {
            return None;
        }
    } else if root_entries == 0 {
        return None;
    }
    let root_dir_sectors = (root_entries * 32).div_ceil(bps);
    let overhead =
        u64::from(reserved) + u64::from(nfats) * u64::from(fat_size) + u64::from(root_dir_sectors);
    let data = u64::from(total).checked_sub(overhead)?;
    let clusters = data / u64::from(spc);
    if clusters == 0 {
        return None;
    }
    let kind = if clusters < 4085 {
        FsKind::Fat12
    } else if clusters < 65525 {
        FsKind::Fat16
    } else {
        FsKind::Fat32
    };
    // La disposition doit concorder avec le type déduit du nombre de clusters.
    if (kind == FsKind::Fat32) != is32_layout {
        return None;
    }
    let label = if is32_layout {
        (sector[66] == 0x29 || sector[66] == 0x28)
            .then(|| label_from(&sector[71..82]))
            .flatten()
    } else {
        (sector[38] == 0x29 || sector[38] == 0x28)
            .then(|| label_from(&sector[43..54]))
            .flatten()
    };
    Some(FsInfo {
        kind,
        block_size: bps * spc,
        size_sectors: to_dev_sectors(u64::from(total) * u64::from(bps), dev_ss),
        label,
    })
}

/// Valide un secteur de démarrage NTFS. La taille rapportée inclut le secteur de
/// démarrage de secours stocké dans le dernier secteur du volume (non compté par `$Boot`).
pub fn detect_ntfs(sector: &[u8], dev_ss: u32) -> Option<FsInfo> {
    if !has_boot_sig(sector) || &sector[3..11] != b"NTFS    " {
        return None;
    }
    let bps = u32::from(le16(sector, 11));
    let spc_raw = sector[13];
    let spc = if spc_raw <= 0x80 {
        u32::from(spc_raw)
    } else {
        // les valeurs > 0x80 codent 2^(256 - x)
        let shift = 256 - u32::from(spc_raw);
        if shift > 12 {
            return None;
        }
        1u32 << shift
    };
    if !(512..=4096).contains(&bps) || !bps.is_power_of_two() {
        return None;
    }
    if spc == 0 || !spc.is_power_of_two() {
        return None;
    }
    if le16(sector, 14) != 0 || sector[16] != 0 || le16(sector, 17) != 0 || le16(sector, 19) != 0 {
        return None; // doit être nul sur NTFS
    }
    let total = le64(sector, 40);
    let mft_lcn = le64(sector, 48);
    if total == 0 {
        return None;
    }
    let clusters = total / u64::from(spc);
    if mft_lcn == 0 || mft_lcn >= clusters {
        return None;
    }
    let block = bps * spc;
    Some(FsInfo {
        kind: FsKind::Ntfs,
        block_size: block,
        size_sectors: to_dev_sectors((total + 1) * u64::from(bps), dev_ss),
        label: None,
    })
}

/// Valide le secteur de démarrage principal exFAT.
pub fn detect_exfat(sector: &[u8], dev_ss: u32) -> Option<FsInfo> {
    if !has_boot_sig(sector) || &sector[3..11] != b"EXFAT   " {
        return None;
    }
    if sector[11..64].iter().any(|&b| b != 0) {
        return None; // région MustBeZero
    }
    let bps_shift = u32::from(sector[108]);
    let spc_shift = u32::from(sector[109]);
    if !(9..=12).contains(&bps_shift) || bps_shift + spc_shift > 25 {
        return None;
    }
    let volume_length = le64(sector, 72);
    let fat_offset = le32(sector, 80);
    let heap_offset = le32(sector, 88);
    let cluster_count = le32(sector, 92);
    let root = le32(sector, 96);
    if volume_length < (1 << 20) >> bps_shift || fat_offset < 24 || heap_offset <= fat_offset {
        return None;
    }
    if cluster_count == 0 || root < 2 || !(1..=2).contains(&sector[110]) {
        return None;
    }
    let bps = 1u64 << bps_shift;
    Some(FsInfo {
        kind: FsKind::ExFat,
        block_size: (bps << spc_shift) as u32,
        size_sectors: to_dev_sectors(volume_length * bps, dev_ss),
        label: None,
    })
}

/// Valide un superbloc ext2/3/4 (les 1024 octets situés 1024 octets après le début du volume).
pub fn detect_ext(sb: &[u8], dev_ss: u32) -> Option<FsInfo> {
    if sb.len() < 1024 || le16(sb, 56) != 0xEF53 {
        return None;
    }
    let log_block = le32(sb, 24);
    if log_block > 6 {
        return None;
    }
    let block_size = 1024u32 << log_block;
    let incompat = le32(sb, 96);
    let blocks_lo = u64::from(le32(sb, 4));
    let blocks = if incompat & 0x80 != 0 {
        blocks_lo | (u64::from(le32(sb, 336)) << 32)
    } else {
        blocks_lo
    };
    if blocks == 0 || le32(sb, 0) == 0 || le32(sb, 32) == 0 {
        return None; // aucun inode / aucun bloc par groupe
    }
    let compat = le32(sb, 92);
    let ro_compat = le32(sb, 100);
    // Même distinction par fonctionnalités que celle du noyau Linux.
    const EXT3_INCOMPAT: u32 = 0x4 | 0x8; // RECOVER | JOURNAL_DEV
    const EXT4_INCOMPAT: u32 = 0x40 | 0x80 | 0x200 | 0x400 | 0x1000 | 0x2000; // EXTENTS 64BIT FLEX_BG EA_INODE DIRDATA/MMP-class
    const EXT4_RO_COMPAT: u32 = 0x8 | 0x10 | 0x20 | 0x40; // HUGE_FILE GDT_CSUM DIR_NLINK EXTRA_ISIZE
    let kind = if incompat & EXT4_INCOMPAT != 0 || ro_compat & EXT4_RO_COMPAT != 0 {
        FsKind::Ext4
    } else if compat & 0x4 != 0 || incompat & EXT3_INCOMPAT != 0 {
        FsKind::Ext3
    } else {
        FsKind::Ext2
    };
    Some(FsInfo {
        kind,
        block_size,
        size_sectors: to_dev_sectors(blocks * u64::from(block_size), dev_ss),
        label: label_from(&sb[120..136]),
    })
}

/// Recherche un système de fichiers dont le premier secteur est `start_lba`.
pub fn probe(dev: &dyn BlockDevice, start_lba: u64) -> Option<FsInfo> {
    let ss = dev.sector_size();
    let start = start_lba.checked_mul(u64::from(ss))?;
    let len = ss.max(512) as usize;
    let mut boot = vec![0u8; len];
    dev.read_at(start, &mut boot).ok()?;
    if let Some(i) = detect_ntfs(&boot, ss)
        .or_else(|| detect_exfat(&boot, ss))
        .or_else(|| detect_fat(&boot, ss))
    {
        return Some(i);
    }
    let mut sb = vec![0u8; 1024];
    dev.read_at(start.checked_add(1024)?, &mut sb).ok()?;
    detect_ext(&sb, ss)
}

/// Recherche un secteur de démarrage NTFS *de secours* dans le dernier secteur d'un
/// volume candidat (`last_lba`). En cas de succès, `size_sectors` en tient déjà compte.
pub fn probe_ntfs_backup(dev: &dyn BlockDevice, last_lba: u64) -> Option<FsInfo> {
    let ss = dev.sector_size();
    let sec = dev.read_sectors(last_lba, 1).ok()?;
    detect_ntfs(&sec, ss)
}

#[cfg(test)]
mod tests;
