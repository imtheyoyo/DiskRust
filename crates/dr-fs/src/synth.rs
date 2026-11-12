//! Constructeurs de secteurs de démarrage et superblocs synthétiques (utilisés par les tests de ce workspace).

use crate::FsKind;

fn base(sig_jump: bool) -> [u8; 512] {
    let mut s = [0u8; 512];
    if sig_jump {
        s[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    }
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

fn label11(s: &mut [u8], off: usize, label: &str) {
    let mut l = [b' '; 11];
    for (i, c) in label.bytes().take(11).enumerate() {
        l[i] = c;
    }
    s[off..off + 11].copy_from_slice(&l);
}

/// Secteur de démarrage FAT12 de type disquette 1,44 Mo.
pub fn fat12_boot_sector() -> [u8; 512] {
    let mut s = base(true);
    s[3..11].copy_from_slice(b"MSDOS5.0");
    s[11..13].copy_from_slice(&512u16.to_le_bytes());
    s[13] = 1;
    s[14..16].copy_from_slice(&1u16.to_le_bytes());
    s[16] = 2;
    s[17..19].copy_from_slice(&224u16.to_le_bytes());
    s[19..21].copy_from_slice(&2880u16.to_le_bytes());
    s[21] = 0xF0;
    s[22..24].copy_from_slice(&9u16.to_le_bytes());
    s[38] = 0x29;
    label11(&mut s, 43, "FLOPPY");
    s[54..62].copy_from_slice(b"FAT12   ");
    s
}

/// Secteur de démarrage FAT16 pour un volume de 40000 secteurs.
pub fn fat16_boot_sector() -> [u8; 512] {
    let mut s = base(true);
    s[3..11].copy_from_slice(b"MSDOS5.0");
    s[11..13].copy_from_slice(&512u16.to_le_bytes());
    s[13] = 4;
    s[14..16].copy_from_slice(&1u16.to_le_bytes());
    s[16] = 2;
    s[17..19].copy_from_slice(&512u16.to_le_bytes());
    s[19..21].copy_from_slice(&40_000u16.to_le_bytes());
    s[21] = 0xF8;
    s[22..24].copy_from_slice(&40u16.to_le_bytes());
    s[38] = 0x29;
    label11(&mut s, 43, "DATA16");
    s[54..62].copy_from_slice(b"FAT16   ");
    s
}

/// Secteur de démarrage FAT32 avec des clusters de 512 octets. `total_sectors` doit être >= ~66000
/// pour que le nombre de clusters atteigne le seuil FAT32.
pub fn fat32_boot_sector(total_sectors: u32) -> [u8; 512] {
    let mut s = base(true);
    s[3..11].copy_from_slice(b"MSWIN4.1");
    s[11..13].copy_from_slice(&512u16.to_le_bytes());
    s[13] = 1;
    s[14..16].copy_from_slice(&32u16.to_le_bytes());
    s[16] = 2;
    s[21] = 0xF8;
    s[32..36].copy_from_slice(&total_sectors.to_le_bytes());
    let fat_size = total_sectors / 128 + 2;
    s[36..40].copy_from_slice(&fat_size.to_le_bytes());
    s[44..48].copy_from_slice(&2u32.to_le_bytes());
    s[48..50].copy_from_slice(&1u16.to_le_bytes());
    s[50..52].copy_from_slice(&6u16.to_le_bytes());
    s[66] = 0x29;
    label11(&mut s, 71, "BACKUP");
    s[82..90].copy_from_slice(b"FAT32   ");
    s
}

/// Secteur de démarrage NTFS (secteurs de 512 octets, clusters de 4 Kio).
pub fn ntfs_boot_sector(total_sectors: u64, mft_lcn: u64) -> [u8; 512] {
    let mut s = base(false);
    s[0..3].copy_from_slice(&[0xEB, 0x52, 0x90]);
    s[3..11].copy_from_slice(b"NTFS    ");
    s[11..13].copy_from_slice(&512u16.to_le_bytes());
    s[13] = 8;
    s[21] = 0xF8;
    s[40..48].copy_from_slice(&total_sectors.to_le_bytes());
    s[48..56].copy_from_slice(&mft_lcn.to_le_bytes());
    s[56..64].copy_from_slice(&(total_sectors / 16).to_le_bytes());
    s[64] = 0xF6; // -10 => enregistrements MFT de 1024 octets
    s[68] = 1;
    s[72..80].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    s
}

/// Secteur de démarrage exFAT (secteurs de 512 octets, clusters de 4 Kio).
pub fn exfat_boot_sector(volume_sectors: u64) -> [u8; 512] {
    let mut s = base(false);
    s[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
    s[3..11].copy_from_slice(b"EXFAT   ");
    s[72..80].copy_from_slice(&volume_sectors.to_le_bytes());
    s[80..84].copy_from_slice(&128u32.to_le_bytes());
    s[84..88].copy_from_slice(&64u32.to_le_bytes());
    s[88..92].copy_from_slice(&256u32.to_le_bytes());
    s[92..96].copy_from_slice(&((volume_sectors / 8) as u32).to_le_bytes());
    s[96..100].copy_from_slice(&4u32.to_le_bytes());
    s[108] = 9; // secteurs de 512 octets
    s[109] = 3; // 8 secteurs par cluster
    s[110] = 1;
    s
}

/// Superbloc ext de 1024 octets (à placer 1024 octets après le début du volume).
pub fn ext_superblock(kind: FsKind, blocks: u32, label: &str) -> [u8; 1024] {
    let mut sb = [0u8; 1024];
    sb[0..4].copy_from_slice(&(blocks / 4).to_le_bytes()); // inodes
    sb[4..8].copy_from_slice(&blocks.to_le_bytes());
    sb[24..28].copy_from_slice(&2u32.to_le_bytes()); // blocs de 4 Kio
    sb[32..36].copy_from_slice(&32768u32.to_le_bytes()); // blocs par groupe
    sb[56..58].copy_from_slice(&0xEF53u16.to_le_bytes());
    sb[58..60].copy_from_slice(&1u16.to_le_bytes());
    sb[76..80].copy_from_slice(&1u32.to_le_bytes()); // révision dynamique
    let (compat, incompat, ro) = match kind {
        FsKind::Ext2 => (0x38u32, 0x2u32, 0x1u32),
        FsKind::Ext3 => (0x3C, 0x2, 0x1),
        _ => (0x3C, 0x2 | 0x40 | 0x200, 0x1 | 0x8 | 0x10 | 0x40),
    };
    sb[92..96].copy_from_slice(&compat.to_le_bytes());
    sb[96..100].copy_from_slice(&incompat.to_le_bytes());
    sb[100..104].copy_from_slice(&ro.to_le_bytes());
    for (i, c) in label.bytes().take(16).enumerate() {
        sb[120 + i] = c;
    }
    sb
}
