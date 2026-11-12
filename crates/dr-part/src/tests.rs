use dr_disk::MemImage;

use crate::synth::*;
use crate::*;

const ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const LINUX: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

fn mbr_disk() -> MemImage {
    let mut img = MemImage::zeroed(512 * 100_000);
    let s = mbr_sector(&[
        mbr_entry(0x80, 0x0C, 2048, 20_000),
        mbr_entry(0x00, 0x83, 30_000, 40_000),
    ]);
    img.put(0, &s);
    img
}

#[test]
fn mbr_two_primaries() {
    let img = mbr_disk();
    assert_eq!(detect_table_type(&img), TableType::Intel);
    let t = parse_mbr(&img).unwrap();
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    assert_eq!(t.partitions.len(), 2);
    let p = &t.partitions[0];
    assert_eq!((p.start_lba, p.size_sectors), (2048, 20_000));
    assert!(p.bootable);
    assert_eq!(p.type_id, TypeId::Mbr(0x0C));
    assert_eq!(p.kind, PartKind::Primary);
    assert_eq!(t.partitions[1].start_lba, 30_000);
    assert!(!t.partitions[1].bootable);
    assert_eq!(type_label(p), "FAT32 LBA");
}

#[test]
fn mbr_bad_signature() {
    let img = MemImage::zeroed(512 * 64);
    assert!(matches!(parse_mbr(&img), Err(PartError::BadMbrSignature)));
    assert_eq!(detect_table_type(&img), TableType::None);
    let mut junk = MemImage::zeroed(512 * 64);
    junk.put(0, b"garbage");
    assert_eq!(detect_table_type(&junk), TableType::Unknown);
}

#[test]
fn mbr_chs_mismatch_warns() {
    let mut img = MemImage::zeroed(512 * 100_000);
    let mut e = mbr_entry(0x00, 0x83, 2048, 10_000);
    e[1..4].copy_from_slice(&lba_to_chs(4096)); // wrong start CHS
    img.put(0, &mbr_sector(&[e]));
    let t = parse_mbr(&img).unwrap();
    assert_eq!(t.partitions.len(), 1);
    assert!(
        t.warnings.iter().any(|w| w.contains("CHS/LBA mismatch")),
        "{:?}",
        t.warnings
    );
}

#[test]
fn mbr_extended_chain() {
    let mut img = MemImage::zeroed(512 * 100_000);
    img.put(
        0,
        &mbr_sector(&[
            mbr_entry(0x80, 0x07, 2048, 8_000),
            mbr_entry(0, 0x0F, 10_000, 50_000),
        ]),
    );
    // EBR 1 at 10000: logical at +63, link to +20000 (relative to extended start)
    let ebr1 = mbr_sector(&[mbr_entry(0, 0x83, 63, 9_000), {
        let mut l = mbr_entry(0, 0x05, 20_000, 5_000);
        l[8..12].copy_from_slice(&10_000u32.to_le_bytes());
        l
    }]);
    img.put(10_000 * 512, &ebr1);
    // EBR 2 at 10000+10000=20000: logical at +63, no link
    let ebr2 = mbr_sector(&[mbr_entry(0, 0x82, 63, 4_000)]);
    img.put(20_000 * 512, &ebr2);
    let t = parse_mbr(&img).unwrap();
    let logical: Vec<_> = t
        .partitions
        .iter()
        .filter(|p| p.kind == PartKind::Logical)
        .collect();
    assert_eq!(logical.len(), 2, "{:?} {:?}", t.partitions, t.warnings);
    assert_eq!(logical[0].start_lba, 10_063);
    assert_eq!(logical[1].start_lba, 20_063);
    assert_eq!(t.partitions[1].kind, PartKind::Extended);
}

#[test]
fn mbr_ebr_loop_is_detected() {
    let mut img = MemImage::zeroed(512 * 100_000);
    img.put(0, &mbr_sector(&[mbr_entry(0, 0x05, 1000, 50_000)]));
    // EBR at 1000 links to itself (relative 0).
    let mut link = mbr_entry(0, 0x05, 0, 1000);
    link[8..12].copy_from_slice(&0u32.to_le_bytes());
    img.put(
        1000 * 512,
        &mbr_sector(&[mbr_entry(0, 0x83, 63, 500), link]),
    );
    let t = parse_mbr(&img).unwrap();
    assert!(
        t.warnings.iter().any(|w| w.contains("loop")),
        "{:?}",
        t.warnings
    );
    assert_eq!(
        t.partitions
            .iter()
            .filter(|p| p.kind == PartKind::Logical)
            .count(),
        1
    );
}

#[test]
fn guid_roundtrip_and_names() {
    let raw = parse_guid(ESP).unwrap();
    assert_eq!(raw[0..4], [0x28, 0x73, 0x2A, 0xC1]); // mixed endian
    assert_eq!(format_guid(&raw), ESP);
    assert_eq!(gpt_type_name(&raw), Some("EFI System"));
    assert!(parse_guid("nope").is_none());
}

fn gpt_disk() -> Vec<u8> {
    gpt_image(
        4096,
        &[(ESP, 34, 1033, "EFI system"), (LINUX, 1034, 3000, "root")],
    )
}

#[test]
fn gpt_valid() {
    let img = MemImage::new(gpt_disk());
    assert_eq!(detect_table_type(&img), TableType::Gpt);
    let t = parse_gpt(&img).unwrap();
    assert!(!t.used_backup);
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    assert_eq!(t.partitions.len(), 2);
    assert_eq!(t.partitions[0].name, "EFI system");
    assert_eq!(t.partitions[0].size_sectors, 1000);
    assert_eq!(type_label(&t.partitions[1]), "Linux filesystem");
}

#[test]
fn gpt_corrupt_primary_header_falls_back_to_backup() {
    let mut data = gpt_disk();
    data[512 + 40] ^= 0xFF; // corrupt first_usable_lba: header CRC no longer matches
    let img = MemImage::new(data);
    let t = parse_gpt(&img).unwrap();
    assert!(t.used_backup);
    assert_eq!(t.partitions.len(), 2);
    assert!(t
        .warnings
        .iter()
        .any(|w| w.contains("invalid header checksum")));
    assert!(t.warnings.iter().any(|w| w == "Trying alternate GPT"));
}

#[test]
fn gpt_corrupt_primary_entries_falls_back_to_backup() {
    let mut data = gpt_disk();
    data[2 * 512 + 32] ^= 0x01; // corrupt an entry in the primary array
    let t = parse_gpt(&MemImage::new(data)).unwrap();
    assert!(t.used_backup);
    assert!(t
        .warnings
        .iter()
        .any(|w| w.contains("invalid partition entries checksum")));
}

#[test]
fn gpt_both_corrupt_is_error() {
    let mut data = gpt_disk();
    data[512 + 40] ^= 0xFF;
    let last = 4095 * 512;
    data[last + 40] ^= 0xFF;
    let r = parse_gpt(&MemImage::new(data));
    assert!(matches!(r, Err(PartError::NoValidGpt(_))));
}

#[test]
fn gpt_sanity_checks() {
    // Bad entry size, with a re-computed, valid header CRC.
    let mut data = gpt_disk();
    data[512 + 84..512 + 88].copy_from_slice(&100u32.to_le_bytes());
    data[512 + 16..512 + 20].fill(0);
    let crc = crc32fast::hash(&data[512..512 + 92]);
    data[512 + 16..512 + 20].copy_from_slice(&crc.to_le_bytes());
    let h = GptHeader::parse(&data[512..1024]).unwrap();
    assert_eq!(h.entry_size, 100);
    let t = parse_gpt(&MemImage::new(data)).unwrap(); // falls back to the good backup
    assert!(t.used_backup);
    assert!(t.warnings[0].contains("invalid partition entry size"));
    assert!(matches!(
        GptHeader::parse(&[0u8; 512]),
        Err(GptError::BadSignature)
    ));
}

#[test]
fn read_table_summary() {
    let info = read_table(&mbr_disk());
    assert_eq!(info.table_type, TableType::Intel);
    assert_eq!(info.partitions.len(), 2);
    let info = read_table(&MemImage::new(gpt_disk()));
    assert_eq!(info.table_type, TableType::Gpt);
    assert_eq!(info.partitions.len(), 2);
    assert!(info.error.is_none());
}
