//! Analyse du MBR et des partitions étendues (chaîne EBR).

use std::collections::HashSet;
use std::fmt;

use dr_disk::BlockDevice;

use crate::types::{PartError, PartKind, Partition, TypeId};

/// Géométrie de traduction par défaut utilisée pour les contrôles de cohérence CHS.
pub const DEFAULT_HEADS: u32 = 255;
/// Nombre de secteurs par piste par défaut utilisé pour les contrôles de cohérence CHS.
pub const DEFAULT_SECTORS_PER_TRACK: u32 = 63;
/// Borne supérieure du nombre de liens EBR suivis (protection contre les boucles et les dérives).
const MAX_LOGICAL: usize = 256;

/// Triplet cylindre/tête/secteur décodé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chs {
    /// Cylindre (0..=1023).
    pub cylinder: u16,
    /// Tête (0..=255).
    pub head: u8,
    /// Secteur (1..=63, 0 signifie invalide).
    pub sector: u8,
}

impl Chs {
    /// Décode la forme sur disque de 3 octets.
    pub fn decode(b: [u8; 3]) -> Self {
        Chs {
            head: b[0],
            sector: b[1] & 0x3F,
            cylinder: (u16::from(b[1] & 0xC0) << 2) | u16::from(b[2]),
        }
    }

    /// Convertit en LBA selon la géométrie donnée.
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

    /// Les valeurs CHS saturent au cylindre 1023 sur les gros disques et n'ont alors plus de sens.
    pub fn is_saturated(self) -> bool {
        self.cylinder >= 1023
    }
}

impl fmt::Display for Chs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.cylinder, self.head, self.sector)
    }
}

/// Une entrée de partition brute de 16 octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrEntry {
    /// Octet d'indicateur d'amorçage (0x80 actif, 0x00 inactif).
    pub boot: u8,
    /// CHS du premier secteur.
    pub chs_start: Chs,
    /// Identifiant système.
    pub type_id: u8,
    /// CHS du dernier secteur.
    pub chs_end: Chs,
    /// LBA de début (relatif à la base de la table).
    pub start_lba: u32,
    /// Taille en secteurs.
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

    /// Emplacement inutilisé.
    pub fn is_empty(&self) -> bool {
        self.type_id == 0 || self.size_sectors == 0
    }

    /// Conteneur étendu DOS/Win95/Linux.
    pub fn is_extended(&self) -> bool {
        matches!(self.type_id, 0x05 | 0x0F | 0x85)
    }
}

/// Résultat de l'analyse d'une table de partitions Intel.
#[derive(Debug, Clone, Default)]
pub struct MbrTable {
    /// Partitions primaires, étendues et logiques dans l'ordre de découverte.
    pub partitions: Vec<Partition>,
    /// Diagnostics non fatals (incohérences CHS, boucles, valeurs hors limites…).
    pub warnings: Vec<String>,
}

/// Analyse les quatre entrées d'un secteur de démarrage de 512 octets ; exige la signature 0x55AA.
pub fn parse_mbr_sector(sec: &[u8]) -> Result<[MbrEntry; 4], PartError> {
    if sec.len() < 512 || sec[510] != 0x55 || sec[511] != 0xAA {
        return Err(PartError::BadMbrSignature);
    }
    let e = |i: usize| MbrEntry::parse(&sec[446 + 16 * i..446 + 16 * (i + 1)]);
    Ok([e(0), e(1), e(2), e(3)])
}

/// Identifiant système MBR lisible.
pub fn mbr_type_name(t: u8) -> &'static str {
    match t {
        0x00 => "Vide",
        0x01 => "FAT12",
        0x04 => "FAT16 <32M",
        0x05 => "Étendue",
        0x06 => "FAT16",
        0x07 => "HPFS/NTFS/exFAT",
        0x0B => "FAT32",
        0x0C => "FAT32 LBA",
        0x0E => "FAT16 LBA",
        0x0F => "Étendue LBA",
        0x11 => "FAT12 masquée",
        0x14 => "FAT16 <32M masquée",
        0x16 => "FAT16 masquée",
        0x17 => "HPFS/NTFS masquée",
        0x1B => "FAT32 masquée",
        0x1C => "FAT32 LBA masquée",
        0x1E => "FAT16 LBA masquée",
        0x27 => "NTFS masquée (WinRE)",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x85 => "Linux étendue",
        0x8E => "Linux LVM",
        0xA5 => "FreeBSD",
        0xA6 => "OpenBSD",
        0xA9 => "NetBSD",
        0xAF => "Apple HFS/HFS+",
        0xEE => "EFI GPT protectrice",
        0xEF => "EFI System",
        0xFD => "Linux RAID (détection automatique)",
        _ => "Inconnue",
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
            "{label} : indicateur d'amorçage 0x{:02X} invalide (0x00 ou 0x80 attendu)",
            e.boot
        ));
    }
    let (h, s) = (DEFAULT_HEADS, DEFAULT_SECTORS_PER_TRACK);
    if !e.chs_start.is_saturated() {
        if let Some(lba) = e.chs_start.to_lba(h, s) {
            if lba != abs_start {
                warnings.push(format!(
                    "{label} : incohérence CHS/LBA au début : CHS {} = LBA {lba}, mais champ LBA = {abs_start}",
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
                    "{label} : incohérence CHS/LBA à la fin : CHS {} = LBA {lba}, mais champ LBA = {abs_end}",
                    e.chs_end
                ));
            }
        }
    }
    if abs_end >= disk_sectors {
        warnings.push(format!(
            "{label} : la partition se termine au LBA {abs_end}, au-delà de la fin du disque ({disk_sectors} secteurs)"
        ));
    }
}

/// Analyse le MBR au LBA 0 et suit la chaîne EBR de la partition étendue.
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
                    "Partition {} : type 0x{:02X} de taille nulle",
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
                    .push(format!("{label} : plus d'une partition étendue"));
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
    let mut rel = 0u64; // les décalages de liens sont toujours relatifs au début de la partition étendue
    for n in 0..MAX_LOGICAL {
        let ebr_lba = ext_start + rel;
        if !visited.insert(ebr_lba) {
            table.warnings.push(format!(
                "Boucle détectée dans la chaîne EBR au LBA {ebr_lba}"
            ));
            return;
        }
        if ebr_lba >= ext_start + ext_size || ebr_lba >= disk_sectors {
            table.warnings.push(format!(
                "L'EBR au LBA {ebr_lba} se trouve en dehors de la partition étendue"
            ));
            return;
        }
        let sec = match dev.read_sectors(ebr_lba, 1) {
            Ok(s) => s,
            Err(e) => {
                table.warnings.push(format!(
                    "Lecture impossible de l'EBR au LBA {ebr_lba} : {e}"
                ));
                return;
            }
        };
        let entries = match parse_mbr_sector(&sec) {
            Ok(e) => e,
            Err(_) => {
                table
                    .warnings
                    .push(format!("EBR au LBA {ebr_lba} : signature 0x55AA absente"));
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
            let label = format!("Logique {}", n + 5);
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
        "Chaîne EBR de plus de {MAX_LOGICAL} entrées, abandon"
    ));
}
