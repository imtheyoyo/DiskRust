use std::process::Command;

use dr_fs::synth::*;
use dr_part::synth::*;

fn write_image(dir: &std::path::Path) -> std::path::PathBuf {
    let mut img = vec![0u8; 512 * 160_000];
    let mbr = mbr_sector(&[
        mbr_entry(0x80, 0x0C, 2048, 70_000),
        mbr_entry(0, 0x07, 73_728, 60_001),
    ]);
    img[..512].copy_from_slice(&mbr);
    img[2048 * 512..2049 * 512].copy_from_slice(&fat32_boot_sector(70_000));
    img[73_728 * 512..73_729 * 512].copy_from_slice(&ntfs_boot_sector(60_000, 4));
    let p = dir.join("test.dd");
    std::fs::write(&p, img).unwrap();
    p
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_diskrust"))
}

#[test]
fn list_prints_table() {
    let dir = tempfile::tempdir().unwrap();
    let img = write_image(dir.path());
    let out = bin().arg("list").arg("--image").arg(&img).output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("bytes/sector"), "{s}");
    assert!(s.contains("Partition table type: Intel"), "{s}");
    assert!(s.contains("[FAT32]") && s.contains("[NTFS]"), "{s}");
}

#[test]
fn analyse_reports_structure_and_log() {
    let dir = tempfile::tempdir().unwrap();
    let img = write_image(dir.path());
    let log = dir.path().join("diskrust.log");
    let out = bin()
        .args(["analyse", "--log"])
        .arg(&log)
        .arg("--image")
        .arg(&img)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Structure: Ok"), "{s}");
    assert!(s.contains("Quick Search found 2"), "{s}");
    assert!(s.contains("(in partition table)"), "{s}");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("Structure: Ok") && logged.contains("read-only"));
}

#[test]
fn analyse_flags_missing_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut img = vec![0u8; 512 * 160_000];
    img[2048 * 512..2049 * 512].copy_from_slice(&fat32_boot_sector(70_000));
    let p = dir.path().join("lost.dd");
    std::fs::write(&p, img).unwrap();
    let out = bin()
        .arg("analyse")
        .arg("--image")
        .arg(&p)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Partition table type: None"), "{s}");
    assert!(s.contains("recovery candidate"), "{s}");
}

#[test]
fn missing_file_fails() {
    let out = bin()
        .args(["list", "--image", "/nonexistent/x.dd"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
