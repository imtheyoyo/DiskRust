//! Report generation for the `diskrust` command line (strictly read-only).
//!
//! TODO(write support): any future operation that modifies a disk (writing a
//! partition table, fixing boot sectors) must be gated behind an explicit
//! confirmation flag and a separate `WritableDevice` implementation. Nothing
//! in this crate opens a device for writing.

use dr_disk::{format_size, BlockDevice};
use dr_fs::{probe, FsInfo};
use dr_part::{read_table, type_label, Partition, TableType};
use dr_search::{check_structure_on_disk, quick_search, Progress, SearchOptions};

/// `Disk <label> - <size> - <sector size>` header line.
pub fn disk_header(label: &str, dev: &dyn BlockDevice) -> String {
    format!(
        "Disk {label} - {} - {} bytes/sector",
        format_size(dev.size_bytes()),
        dev.sector_size()
    )
}

fn table_header() -> String {
    format!(
        "     {:<10} {:<30} {:>12} {:>12} {:>12}",
        "Kind", "Type", "Start", "End", "Size (sect.)"
    )
}

fn partition_row(index: usize, p: &Partition, fs: Option<&FsInfo>) -> String {
    let mut row = format!(
        "{:>3} {} {:<10} {:<30} {:>12} {:>12} {:>12}",
        index + 1,
        if p.bootable { '*' } else { ' ' },
        p.kind.to_string(),
        type_label(p),
        p.start_lba,
        p.end_lba(),
        p.size_sectors
    );
    if let Some(fs) = fs {
        row.push_str(&format!("  [{}]", fs.kind));
        if let Some(l) = &fs.label {
            row.push_str(&format!(" {l}"));
        }
    } else if !p.name.is_empty() && p.name != type_label(p) {
        row.push_str(&format!("  {}", p.name));
    }
    row
}

/// Output of `diskrust list` for one disk (disk and partition listing).
pub fn list_report(label: &str, dev: &dyn BlockDevice) -> Vec<String> {
    let mut out = vec![disk_header(label, dev)];
    let info = read_table(dev);
    out.push(format!("Partition table type: {}", info.table_type));
    for w in &info.warnings {
        out.push(format!("Warning: {w}"));
    }
    if let Some(e) = &info.error {
        out.push(format!("Error: {e}"));
    }
    if !info.partitions.is_empty() {
        out.push(table_header());
        for (i, p) in info.partitions.iter().enumerate() {
            let fs = probe(dev, p.start_lba);
            out.push(partition_row(i, p, fs.as_ref()));
        }
    } else if matches!(info.table_type, TableType::Intel | TableType::Gpt) && info.error.is_none() {
        out.push("No partitions in table.".to_string());
    }
    out
}

/// Output of `diskrust analyse`: table + structure check + Quick Search.
pub fn analyse_report(
    label: &str,
    dev: &dyn BlockDevice,
    progress: &mut dyn FnMut(&Progress),
) -> Vec<String> {
    let mut out = vec![disk_header(label, dev)];
    let info = read_table(dev);
    out.push(format!("Partition table type: {}", info.table_type));
    for w in &info.warnings {
        out.push(format!("Warning: {w}"));
    }
    if let Some(e) = &info.error {
        out.push(format!("Error: {e}"));
    }
    if !info.partitions.is_empty() {
        out.push(table_header());
        for (i, p) in info.partitions.iter().enumerate() {
            out.push(partition_row(i, p, probe(dev, p.start_lba).as_ref()));
        }
    }
    let report = check_structure_on_disk(&info.partitions, Some(dev.sector_count()));
    if report.is_ok() {
        out.push("Structure: Ok".to_string());
    } else {
        out.push("Structure: Bad".to_string());
        for i in &report.issues {
            out.push(format!("  {i}"));
        }
    }

    out.push("Analyse: Quick Search".to_string());
    let found = quick_search(dev, &SearchOptions::default(), progress);
    if found.is_empty() {
        out.push("Quick Search found no filesystem.".to_string());
    } else {
        out.push(format!("Quick Search found {} filesystem(s):", found.len()));
        out.push(table_header());
        for (i, p) in found.iter().enumerate() {
            let listed = info
                .partitions
                .iter()
                .any(|t| t.start_lba == p.start_lba && t.size_sectors == p.size_sectors);
            let mut row = partition_row(i, p, probe(dev, p.start_lba).as_ref());
            row.push_str(if listed {
                "  (in partition table)"
            } else {
                "  (NOT in partition table: recovery candidate)"
            });
            out.push(row);
        }
    }
    out.push("Read-only analysis: nothing was written to the disk.".to_string());
    out
}
