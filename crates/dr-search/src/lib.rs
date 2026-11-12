//! "Quick Search": scan cylinder- and MiB-aligned sectors for filesystem
//! boot sectors, and sanity-check partition layouts.
//!
//! Strictly read-only.

use std::collections::BTreeSet;
use std::fmt;

use dr_disk::BlockDevice;
use dr_fs::probe;
use dr_part::{PartKind, Partition, TypeId};

/// Search parameters.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Translation geometry: heads per cylinder.
    pub heads: u32,
    /// Translation geometry: sectors per track.
    pub sectors_per_track: u32,
    /// Probe every cylinder boundary (and LBA `sectors_per_track`, the classic first partition).
    pub scan_cylinders: bool,
    /// Probe every 1 MiB boundary (modern alignment).
    pub scan_mib: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            heads: 255,
            sectors_per_track: 63,
            scan_cylinders: true,
            scan_mib: true,
        }
    }
}

/// Progress report passed to the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Sector currently being examined.
    pub current_lba: u64,
    /// Total sectors on the device.
    pub total_lba: u64,
    /// Partitions found so far.
    pub found: usize,
}

impl Progress {
    /// Completion in percent (0..=100).
    pub fn percent(&self) -> u64 {
        (self.current_lba.min(self.total_lba) * 100)
            .checked_div(self.total_lba)
            .unwrap_or(100)
    }
}

fn candidate_lbas(dev: &dyn BlockDevice, opts: &SearchOptions) -> BTreeSet<u64> {
    let total = dev.sector_count();
    let mut set = BTreeSet::new();
    if opts.scan_cylinders {
        let spt = u64::from(opts.sectors_per_track.max(1));
        let cyl = spt * u64::from(opts.heads.max(1));
        set.insert(spt);
        let mut lba = 0;
        while lba < total {
            set.insert(lba);
            lba += cyl;
        }
    }
    if opts.scan_mib {
        let step = (1024 * 1024 / u64::from(dev.sector_size())).max(1);
        let mut lba = 0;
        while lba < total {
            set.insert(lba);
            lba += step;
        }
    }
    set.retain(|&l| l < total);
    set
}

/// Scan for filesystems and return candidate partitions in LBA order.
///
/// Once a filesystem is found, scanning resumes after its end so nested
/// false positives are not reported.
pub fn quick_search(
    dev: &dyn BlockDevice,
    opts: &SearchOptions,
    progress: &mut dyn FnMut(&Progress),
) -> Vec<Partition> {
    let total = dev.sector_count();
    let mut found = Vec::new();
    let mut skip_until = 0u64;
    for (n, lba) in candidate_lbas(dev, opts).into_iter().enumerate() {
        if n % 256 == 0 {
            progress(&Progress {
                current_lba: lba,
                total_lba: total,
                found: found.len(),
            });
        }
        if lba < skip_until {
            continue;
        }
        if let Some(fs) = probe(dev, lba) {
            skip_until = lba + fs.size_sectors.max(1);
            found.push(Partition {
                start_lba: lba,
                size_sectors: fs.size_sectors,
                kind: PartKind::Primary,
                type_id: TypeId::Mbr(fs.kind.mbr_type()),
                name: match &fs.label {
                    Some(l) => format!("{} [{l}]", fs.kind),
                    None => fs.kind.to_string(),
                },
                bootable: false,
            });
        }
    }
    progress(&Progress {
        current_lba: total,
        total_lba: total,
        found: found.len(),
    });
    found
}

/// TODO: search for ext2/3/4 backup superblocks (at block group 1, 3, 5, 7, 9,
/// 25, 27 ...) to recover ext partitions whose primary superblock is lost.
/// Not implemented yet; always returns an empty list.
pub fn search_ext_backup_superblocks(
    _dev: &dyn BlockDevice,
    _progress: &mut dyn FnMut(&Progress),
) -> Vec<Partition> {
    Vec::new()
}

/// A problem with a partition layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureIssue {
    /// Partition has zero length.
    ZeroSize {
        /// Index in the input slice.
        index: usize,
    },
    /// Partition starts before the previous one.
    OutOfOrder {
        /// Index in the input slice.
        index: usize,
    },
    /// Two partitions overlap (extended containers and their logicals excepted).
    Overlap {
        /// First partition index.
        a: usize,
        /// Second partition index.
        b: usize,
    },
    /// More than four primary/extended entries.
    TooManyPrimaries(usize),
    /// A logical partition is not inside any extended partition.
    LogicalOutsideExtended {
        /// Index in the input slice.
        index: usize,
    },
    /// Partition extends past the end of the disk.
    BeyondDisk {
        /// Index in the input slice.
        index: usize,
    },
}

impl fmt::Display for StructureIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StructureIssue::ZeroSize { index } => {
                write!(f, "partition {} has zero size", index + 1)
            }
            StructureIssue::OutOfOrder { index } => {
                write!(
                    f,
                    "partition {} starts before the previous partition",
                    index + 1
                )
            }
            StructureIssue::Overlap { a, b } => {
                write!(f, "partitions {} and {} overlap", a + 1, b + 1)
            }
            StructureIssue::TooManyPrimaries(n) => {
                write!(f, "{n} primary/extended partitions (maximum is 4)")
            }
            StructureIssue::LogicalOutsideExtended { index } => {
                write!(
                    f,
                    "logical partition {} is outside any extended partition",
                    index + 1
                )
            }
            StructureIssue::BeyondDisk { index } => {
                write!(
                    f,
                    "partition {} extends beyond the end of the disk",
                    index + 1
                )
            }
        }
    }
}

/// Result of [`check_structure`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructureReport {
    /// All problems found.
    pub issues: Vec<StructureIssue>,
}

impl StructureReport {
    /// True when the layout has no problems ("Structure: Ok").
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Check ordering, overlaps and MBR container rules of a partition list.
pub fn check_structure(parts: &[Partition]) -> StructureReport {
    check_structure_on_disk(parts, None)
}

/// Like [`check_structure`], additionally verifying that nothing ends past `disk_sectors`.
pub fn check_structure_on_disk(parts: &[Partition], disk_sectors: Option<u64>) -> StructureReport {
    let mut issues = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if p.size_sectors == 0 {
            issues.push(StructureIssue::ZeroSize { index: i });
        }
        if i > 0 && p.start_lba < parts[i - 1].start_lba {
            issues.push(StructureIssue::OutOfOrder { index: i });
        }
        if let Some(d) = disk_sectors {
            if p.start_lba.saturating_add(p.size_sectors) > d {
                issues.push(StructureIssue::BeyondDisk { index: i });
            }
        }
    }
    let primaries = parts
        .iter()
        .filter(|p| matches!(p.kind, PartKind::Primary | PartKind::Extended))
        .count();
    if primaries > 4 {
        issues.push(StructureIssue::TooManyPrimaries(primaries));
    }
    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            let (a, b) = (&parts[i], &parts[j]);
            if a.size_sectors == 0 || b.size_sectors == 0 {
                continue;
            }
            let container = matches!(
                (a.kind, b.kind),
                (PartKind::Extended, PartKind::Logical) | (PartKind::Logical, PartKind::Extended)
            );
            if !container && a.start_lba <= b.end_lba() && b.start_lba <= a.end_lba() {
                issues.push(StructureIssue::Overlap { a: i, b: j });
            }
        }
    }
    for (i, p) in parts.iter().enumerate() {
        if p.kind == PartKind::Logical {
            let inside = parts.iter().any(|e| {
                e.kind == PartKind::Extended
                    && e.size_sectors > 0
                    && p.start_lba >= e.start_lba
                    && p.end_lba() <= e.end_lba()
            });
            if !inside {
                issues.push(StructureIssue::LogicalOutsideExtended { index: i });
            }
        }
    }
    StructureReport { issues }
}

#[cfg(test)]
mod tests;
