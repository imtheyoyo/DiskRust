use dr_disk::MemImage;

use crate::synth::*;
use crate::*;

fn with_boot(sector: &[u8], at_lba: u64, total: usize) -> MemImage {
    let mut img = MemImage::zeroed(total);
    img.put(at_lba as usize * 512, sector);
    img
}

#[test]
fn fat_types_by_cluster_count() {
    let i = probe(&with_boot(&fat12_boot_sector(), 0, 4096), 0).unwrap();
    assert_eq!(i.kind, FsKind::Fat12);
    assert_eq!(i.size_sectors, 2880);
    assert_eq!(i.label.as_deref(), Some("FLOPPY"));

    let i = probe(&with_boot(&fat16_boot_sector(), 63, 512 * 200), 63).unwrap();
    assert_eq!(
        (i.kind, i.block_size, i.size_sectors),
        (FsKind::Fat16, 2048, 40_000)
    );
    assert_eq!(i.label.as_deref(), Some("DATA16"));

    let i = probe(
        &with_boot(&fat32_boot_sector(70_000), 2048, 512 * 4096),
        2048,
    )
    .unwrap();
    assert_eq!(
        (i.kind, i.block_size, i.size_sectors),
        (FsKind::Fat32, 512, 70_000)
    );
    assert_eq!(i.label.as_deref(), Some("BACKUP"));
}

#[test]
fn fat_rejects_garbage() {
    let mut s = fat32_boot_sector(70_000);
    s[11..13].copy_from_slice(&1000u16.to_le_bytes()); // octets/secteur n'est pas une puissance de deux
    assert!(detect_fat(&s, 512).is_none());
    let mut s = fat32_boot_sector(70_000);
    s[510] = 0;
    assert!(detect_fat(&s, 512).is_none());
    let mut s = fat32_boot_sector(70_000);
    s[16] = 0;
    assert!(detect_fat(&s, 512).is_none());
    assert!(detect_fat(&[0u8; 512], 512).is_none());
}

#[test]
fn ntfs_detected() {
    let s = ntfs_boot_sector(100_000, 4);
    let i = probe(&with_boot(&s, 2048, 512 * 4096), 2048).unwrap();
    assert_eq!(i.kind, FsKind::Ntfs);
    assert_eq!(i.block_size, 4096);
    assert_eq!(i.size_sectors, 100_001); // + secteur de démarrage de secours
    let mut bad = s;
    bad[14] = 1; // les secteurs réservés doivent être nuls
    assert!(detect_ntfs(&bad, 512).is_none());
    let mut bad = s;
    bad[48..56].copy_from_slice(&u64::MAX.to_le_bytes()); // MFT hors du volume
    assert!(detect_ntfs(&bad, 512).is_none());
}

#[test]
fn ntfs_backup_boot_sector_at_last_lba() {
    let s = ntfs_boot_sector(2047, 4);
    let img = with_boot(&s, 2047, 512 * 2048);
    let i = probe_ntfs_backup(&img, 2047).unwrap();
    assert_eq!(i.kind, FsKind::Ntfs);
    assert!(probe_ntfs_backup(&img, 2046).is_none());
}

#[test]
fn exfat_detected() {
    let s = exfat_boot_sector(65_536);
    let i = probe(&with_boot(&s, 0, 512 * 128), 0).unwrap();
    assert_eq!(
        (i.kind, i.block_size, i.size_sectors),
        (FsKind::ExFat, 4096, 65_536)
    );
    let mut bad = s;
    bad[20] = 1; // MustBeZero violé
    assert!(detect_exfat(&bad, 512).is_none());
}

#[test]
fn ext_variants() {
    for (kind, want) in [
        (FsKind::Ext2, FsKind::Ext2),
        (FsKind::Ext3, FsKind::Ext3),
        (FsKind::Ext4, FsKind::Ext4),
    ] {
        let mut img = MemImage::zeroed(512 * 4096);
        img.put(2048 * 512 + 1024, &ext_superblock(kind, 25_600, "rootfs"));
        let i = probe(&img, 2048).unwrap();
        assert_eq!(i.kind, want);
        assert_eq!(i.block_size, 4096);
        assert_eq!(i.size_sectors, 25_600 * 8);
        assert_eq!(i.label.as_deref(), Some("rootfs"));
    }
    let mut sb = ext_superblock(FsKind::Ext4, 100, "");
    sb[56] = 0;
    assert!(detect_ext(&sb, 512).is_none());
}

#[test]
fn blank_disk_has_no_fs() {
    assert!(probe(&MemImage::zeroed(512 * 64), 0).is_none());
    assert!(probe(&MemImage::zeroed(512 * 64), 1000).is_none()); // hors limites
}
