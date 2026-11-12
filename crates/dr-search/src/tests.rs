use dr_disk::MemImage;
use dr_fs::synth::*;
use dr_part::synth::{mbr_entry, mbr_sector};
use dr_part::{PartKind, Partition, TypeId};

use crate::*;

fn part(start: u64, size: u64, kind: PartKind) -> Partition {
    Partition {
        start_lba: start,
        size_sectors: size,
        kind,
        type_id: TypeId::Mbr(0x83),
        name: String::new(),
        bootable: false,
    }
}

/// Disque de 80 Mio avec FAT32 à 2048, NTFS à 2048+70000 (non aligné sur un cylindre : aligné sur 1 Mio).
fn disk() -> MemImage {
    let ntfs_start = 73_728u64; // 36 Mio, aligné sur 1 Mio
    let mut img = MemImage::zeroed(512 * 160_000);
    img.put(2048 * 512, &fat32_boot_sector(70_000));
    img.put(ntfs_start as usize * 512, &ntfs_boot_sector(60_000, 4));
    img.put(
        0,
        &mbr_sector(&[
            mbr_entry(0x80, 0x0C, 2048, 70_000),
            mbr_entry(0, 0x07, ntfs_start as u32, 60_001),
        ]),
    );
    img
}

#[test]
fn quick_search_finds_filesystems() {
    let img = disk();
    let mut calls = 0;
    let found = quick_search(&img, &SearchOptions::default(), &mut |p: &Progress| {
        calls += 1;
        assert!(p.percent() <= 100);
    });
    assert!(calls >= 2);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!((found[0].start_lba, found[0].size_sectors), (2048, 70_000));
    assert_eq!(found[0].type_id, TypeId::Mbr(0x0C));
    assert!(found[0].name.contains("FAT32"));
    assert_eq!(
        (found[1].start_lba, found[1].size_sectors),
        (73_728, 60_001)
    );
    assert!(check_structure(&found).is_ok());
}

#[test]
fn legacy_track_start_is_found() {
    let mut img = MemImage::zeroed(512 * 100_000);
    img.put(63 * 512, &fat16_boot_sector());
    let found = quick_search(&img, &SearchOptions::default(), &mut |_| {});
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].start_lba, 63);
}

#[test]
fn no_options_finds_nothing() {
    let opts = SearchOptions {
        scan_cylinders: false,
        scan_mib: false,
        ..Default::default()
    };
    assert!(quick_search(&disk(), &opts, &mut |_| {}).is_empty());
    assert!(search_ext_backup_superblocks(&disk(), &mut |_| {}).is_empty());
}

#[test]
fn structure_ok_and_bad() {
    let ok = [
        part(2048, 1000, PartKind::Primary),
        part(5000, 10_000, PartKind::Extended),
        part(5063, 2000, PartKind::Logical),
    ];
    assert!(check_structure(&ok).is_ok());

    let overlap = [
        part(100, 1000, PartKind::Primary),
        part(500, 1000, PartKind::Primary),
    ];
    let r = check_structure(&overlap);
    assert_eq!(r.issues, vec![StructureIssue::Overlap { a: 0, b: 1 }]);

    let order = [
        part(5000, 10, PartKind::Primary),
        part(100, 10, PartKind::Primary),
    ];
    assert!(check_structure(&order)
        .issues
        .contains(&StructureIssue::OutOfOrder { index: 1 }));

    let zero = [part(100, 0, PartKind::Primary)];
    assert!(!check_structure(&zero).is_ok());

    let five: Vec<_> = (0..5)
        .map(|i| part(i * 100, 50, PartKind::Primary))
        .collect();
    assert!(check_structure(&five)
        .issues
        .contains(&StructureIssue::TooManyPrimaries(5)));

    let stray = [part(100, 10, PartKind::Logical)];
    assert!(!check_structure(&stray).is_ok());

    let beyond = [part(100, 1000, PartKind::Primary)];
    assert!(!check_structure_on_disk(&beyond, Some(500)).is_ok());
    assert!(check_structure_on_disk(&beyond, Some(5000)).is_ok());
}
