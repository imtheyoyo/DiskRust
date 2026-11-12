//! « Recherche rapide » : analyse les secteurs alignés sur les cylindres et sur
//! 1 Mio à la recherche de secteurs de démarrage de systèmes de fichiers, et
//! vérifie la cohérence des schémas de partitions.
//!
//! Strictement en lecture seule.

use std::collections::BTreeSet;
use std::fmt;

use dr_disk::BlockDevice;
use dr_fs::probe;
use dr_part::{PartKind, Partition, TypeId};

/// Paramètres de recherche.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Géométrie de traduction : têtes par cylindre.
    pub heads: u32,
    /// Géométrie de traduction : secteurs par piste.
    pub sectors_per_track: u32,
    /// Sonde chaque frontière de cylindre (et le LBA `sectors_per_track`, la première partition classique).
    pub scan_cylinders: bool,
    /// Sonde chaque frontière de 1 Mio (alignement moderne).
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

/// Rapport de progression transmis au rappel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Secteur actuellement examiné.
    pub current_lba: u64,
    /// Nombre total de secteurs du périphérique.
    pub total_lba: u64,
    /// Partitions trouvées jusqu'ici.
    pub found: usize,
}

impl Progress {
    /// Avancement en pourcentage (0..=100).
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

/// Recherche des systèmes de fichiers et renvoie les partitions candidates dans l'ordre des LBA.
///
/// Dès qu'un système de fichiers est trouvé, l'analyse reprend après sa fin
/// afin de ne pas signaler de faux positifs imbriqués.
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

/// TODO : rechercher les superblocs de sauvegarde ext2/3/4 (aux groupes de blocs
/// 1, 3, 5, 7, 9, 25, 27 ...) pour récupérer les partitions ext dont le
/// superbloc principal est perdu. Pas encore implémenté ; renvoie toujours une
/// liste vide.
pub fn search_ext_backup_superblocks(
    _dev: &dyn BlockDevice,
    _progress: &mut dyn FnMut(&Progress),
) -> Vec<Partition> {
    Vec::new()
}

/// Un problème dans un schéma de partitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureIssue {
    /// La partition a une taille nulle.
    ZeroSize {
        /// Indice dans la tranche d'entrée.
        index: usize,
    },
    /// La partition commence avant la précédente.
    OutOfOrder {
        /// Indice dans la tranche d'entrée.
        index: usize,
    },
    /// Deux partitions se chevauchent (hors conteneurs étendus et leurs partitions logiques).
    Overlap {
        /// Indice de la première partition.
        a: usize,
        /// Indice de la seconde partition.
        b: usize,
    },
    /// Plus de quatre entrées primaires/étendues.
    TooManyPrimaries(usize),
    /// Une partition logique ne se trouve dans aucune partition étendue.
    LogicalOutsideExtended {
        /// Indice dans la tranche d'entrée.
        index: usize,
    },
    /// La partition dépasse la fin du disque.
    BeyondDisk {
        /// Indice dans la tranche d'entrée.
        index: usize,
    },
}

impl fmt::Display for StructureIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StructureIssue::ZeroSize { index } => {
                write!(f, "la partition {} a une taille nulle", index + 1)
            }
            StructureIssue::OutOfOrder { index } => {
                write!(
                    f,
                    "la partition {} commence avant la partition précédente",
                    index + 1
                )
            }
            StructureIssue::Overlap { a, b } => {
                write!(f, "les partitions {} et {} se chevauchent", a + 1, b + 1)
            }
            StructureIssue::TooManyPrimaries(n) => {
                write!(f, "{n} partitions primaires/étendues (maximum : 4)")
            }
            StructureIssue::LogicalOutsideExtended { index } => {
                write!(
                    f,
                    "la partition logique {} est en dehors de toute partition étendue",
                    index + 1
                )
            }
            StructureIssue::BeyondDisk { index } => {
                write!(f, "la partition {} dépasse la fin du disque", index + 1)
            }
        }
    }
}

/// Résultat de [`check_structure`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructureReport {
    /// Tous les problèmes trouvés.
    pub issues: Vec<StructureIssue>,
}

impl StructureReport {
    /// Vrai lorsque le schéma ne présente aucun problème (« Structure : Ok »).
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Vérifie l'ordre, les chevauchements et les règles de conteneurs MBR d'une liste de partitions.
pub fn check_structure(parts: &[Partition]) -> StructureReport {
    check_structure_on_disk(parts, None)
}

/// Comme [`check_structure`], en vérifiant en plus que rien ne dépasse `disk_sectors`.
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
