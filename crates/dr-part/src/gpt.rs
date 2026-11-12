//! Analyse GPT avec validation CRC et repli sur l'en-tête de secours.

use std::fmt;

use dr_disk::BlockDevice;

use crate::types::{PartError, PartKind, Partition, TypeId};

/// `"EFI PART"`
pub const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const MIN_HEADER_SIZE: u32 = 92;
const MAX_ENTRIES: u32 = 4096;
const MAX_ENTRIES_BYTES: u64 = 4 * 1024 * 1024;

/// En-tête GPT analysé et validé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GptHeader {
    /// Révision (0x00010000 pour 1.0).
    pub revision: u32,
    /// Taille de l'en-tête en octets.
    pub header_size: u32,
    /// CRC32 de l'en-tête tel que stocké.
    pub header_crc: u32,
    /// LBA de cet en-tête.
    pub my_lba: u64,
    /// LBA de l'autre en-tête.
    pub alternate_lba: u64,
    /// Premier LBA utilisable par les partitions.
    pub first_usable: u64,
    /// Dernier LBA utilisable par les partitions.
    pub last_usable: u64,
    /// GUID du disque (octets bruts).
    pub disk_guid: [u8; 16],
    /// LBA de début du tableau d'entrées.
    pub entries_lba: u64,
    /// Nombre d'entrées.
    pub num_entries: u32,
    /// Taille d'une entrée.
    pub entry_size: u32,
    /// CRC32 du tableau d'entrées tel que stocké.
    pub entries_crc: u32,
}

/// Anomalies possibles d'une GPT.
#[derive(Debug)]
pub enum GptError {
    /// Échec d'E/S pendant la lecture.
    Io(std::io::Error),
    /// La signature n'est pas « EFI PART ».
    BadSignature,
    /// Champ de taille d'en-tête hors limites.
    BadHeaderSize(u32),
    /// CRC32 de l'en-tête incorrect.
    BadHeaderChecksum {
        /// Valeur présente dans l'en-tête.
        stored: u32,
        /// Valeur calculée.
        computed: u32,
    },
    /// `my_lba` ne correspond pas au LBA d'où l'en-tête a été lu.
    BadCurrentLba {
        /// Valeur présente dans l'en-tête.
        found: u64,
        /// LBA de lecture.
        expected: u64,
    },
    /// La taille d'entrée n'est pas une puissance de deux >= 128.
    BadEntrySize(u32),
    /// Le nombre d'entrées est nul ou démesuré.
    BadEntryCount(u32),
    /// Le tableau d'entrées ne tient pas sur le disque.
    EntriesOutOfRange,
    /// CRC32 du tableau d'entrées incorrect.
    BadEntriesChecksum {
        /// Valeur présente dans l'en-tête.
        stored: u32,
        /// Valeur calculée.
        computed: u32,
    },
}

impl fmt::Display for GptError {
    // Les messages décrivent chaque échec de validation GPT en termes simples.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GptError::Io(e) => write!(f, "Partition GPT invalide, erreur de lecture : {e}"),
            GptError::BadSignature => f.write_str("Partition GPT invalide, signature incorrecte."),
            GptError::BadHeaderSize(s) => {
                write!(f, "Partition GPT invalide, taille d'en-tête incorrecte ({s}).")
            }
            GptError::BadHeaderChecksum { stored, computed } => write!(
                f,
                "Partition GPT invalide, somme de contrôle de l'en-tête incorrecte (stockée {stored:#010x}, calculée {computed:#010x})."
            ),
            GptError::BadCurrentLba { found, expected } => write!(
                f,
                "Partition GPT invalide, LBA courant incorrect ({found}, attendu {expected})."
            ),
            GptError::BadEntrySize(s) => {
                write!(f, "Partition GPT invalide, taille d'entrée de partition incorrecte ({s}).")
            }
            GptError::BadEntryCount(n) => {
                write!(f, "Partition GPT invalide, nombre d'entrées incorrect ({n}).")
            }
            GptError::EntriesOutOfRange => {
                f.write_str("Partition GPT invalide, le tableau d'entrées est en dehors du disque.")
            }
            GptError::BadEntriesChecksum { stored, computed } => write!(
                f,
                "Partition GPT invalide, somme de contrôle des entrées de partition incorrecte (stockée {stored:#010x}, calculée {computed:#010x})."
            ),
        }
    }
}

impl std::error::Error for GptError {}

impl From<std::io::Error> for GptError {
    fn from(e: std::io::Error) -> Self {
        GptError::Io(e)
    }
}

/// Résultat de [`parse_gpt`].
#[derive(Debug, Clone)]
pub struct GptTable {
    /// L'en-tête qui a été accepté.
    pub header: GptHeader,
    /// Partitions non vides.
    pub partitions: Vec<Partition>,
    /// Diagnostics, y compris l'échec de l'en-tête primaire lorsque la copie de secours a été utilisée.
    pub warnings: Vec<String>,
    /// Vrai lorsque la GPT primaire était invalide et que la copie de secours au dernier LBA a été utilisée.
    pub used_backup: bool,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl GptHeader {
    /// Analyse et valide (signature, taille, CRC32) un en-tête à partir d'un secteur.
    pub fn parse(sector: &[u8]) -> Result<Self, GptError> {
        if sector.len() < MIN_HEADER_SIZE as usize || &sector[0..8] != GPT_SIGNATURE {
            return Err(GptError::BadSignature);
        }
        let header_size = u32_at(sector, 12);
        if header_size < MIN_HEADER_SIZE || header_size as usize > sector.len() {
            return Err(GptError::BadHeaderSize(header_size));
        }
        let stored = u32_at(sector, 16);
        let mut copy = sector[..header_size as usize].to_vec();
        copy[16..20].fill(0);
        let computed = crc32fast::hash(&copy);
        if stored != computed {
            return Err(GptError::BadHeaderChecksum { stored, computed });
        }
        let mut disk_guid = [0u8; 16];
        disk_guid.copy_from_slice(&sector[56..72]);
        Ok(GptHeader {
            revision: u32_at(sector, 8),
            header_size,
            header_crc: stored,
            my_lba: u64_at(sector, 24),
            alternate_lba: u64_at(sector, 32),
            first_usable: u64_at(sector, 40),
            last_usable: u64_at(sector, 48),
            disk_guid,
            entries_lba: u64_at(sector, 72),
            num_entries: u32_at(sector, 80),
            entry_size: u32_at(sector, 84),
            entries_crc: u32_at(sector, 88),
        })
    }
}

fn load_gpt_at(
    dev: &dyn BlockDevice,
    lba: u64,
) -> Result<(GptHeader, Vec<Partition>, Vec<String>), GptError> {
    let sector = dev.read_sectors(lba, 1)?;
    let header = GptHeader::parse(&sector)?;
    if header.my_lba != lba {
        return Err(GptError::BadCurrentLba {
            found: header.my_lba,
            expected: lba,
        });
    }
    if header.entry_size < 128 || !header.entry_size.is_power_of_two() {
        return Err(GptError::BadEntrySize(header.entry_size));
    }
    if header.num_entries == 0 || header.num_entries > MAX_ENTRIES {
        return Err(GptError::BadEntryCount(header.num_entries));
    }
    let total = u64::from(header.num_entries) * u64::from(header.entry_size);
    let ss = u64::from(dev.sector_size());
    let start = header
        .entries_lba
        .checked_mul(ss)
        .ok_or(GptError::EntriesOutOfRange)?;
    if total > MAX_ENTRIES_BYTES || start.saturating_add(total) > dev.size_bytes() {
        return Err(GptError::EntriesOutOfRange);
    }
    let mut buf = vec![0u8; total as usize];
    dev.read_at(start, &mut buf)?;
    let computed = crc32fast::hash(&buf);
    if computed != header.entries_crc {
        return Err(GptError::BadEntriesChecksum {
            stored: header.entries_crc,
            computed,
        });
    }

    let mut warnings = Vec::new();
    let mut parts = Vec::new();
    if header.first_usable > header.last_usable {
        warnings.push(
            "En-tête GPT : le premier LBA utilisable est postérieur au dernier LBA utilisable"
                .to_string(),
        );
    }
    for (i, e) in buf.chunks_exact(header.entry_size as usize).enumerate() {
        if e[0..16].iter().all(|&b| b == 0) {
            continue;
        }
        let mut type_guid = [0u8; 16];
        type_guid.copy_from_slice(&e[0..16]);
        let first = u64_at(e, 32);
        let last = u64_at(e, 40);
        let attrs = u64_at(e, 48);
        if last < first || first < header.first_usable || last > header.last_usable {
            warnings.push(format!(
                "Partition GPT {} : la plage {first}-{last} est invalide ou hors de la zone utilisable {}-{}",
                i + 1,
                header.first_usable,
                header.last_usable
            ));
            if last < first {
                continue;
            }
        }
        let units: Vec<u16> = e[56..128.min(e.len())]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        parts.push(Partition {
            start_lba: first,
            size_sectors: last - first + 1,
            kind: PartKind::Gpt,
            type_id: TypeId::Gpt(type_guid),
            name: String::from_utf16_lossy(&units),
            bootable: attrs & 0x4 != 0,
        });
    }
    Ok((header, parts, warnings))
}

/// Analyse la GPT : en-tête primaire au LBA 1, avec repli sur la copie de secours
/// au dernier LBA du disque lorsque la primaire est invalide.
pub fn parse_gpt(dev: &dyn BlockDevice) -> Result<GptTable, PartError> {
    let mut log = Vec::new();
    match load_gpt_at(dev, 1) {
        Ok((header, partitions, mut w)) => {
            let mut warnings = log;
            warnings.append(&mut w);
            return Ok(GptTable {
                header,
                partitions,
                warnings,
                used_backup: false,
            });
        }
        Err(e) => log.push(e.to_string()),
    }
    log.push("Tentative avec la GPT de secours".to_string());
    let last = dev.sector_count().saturating_sub(1);
    if last > 1 {
        match load_gpt_at(dev, last) {
            Ok((header, partitions, mut w)) => {
                let mut warnings = log;
                warnings.push("Utilisation de l'en-tête GPT de secours".to_string());
                warnings.append(&mut w);
                return Ok(GptTable {
                    header,
                    partitions,
                    warnings,
                    used_backup: true,
                });
            }
            Err(e) => log.push(format!("GPT de secours : {e}")),
        }
    }
    Err(PartError::NoValidGpt(log))
}

/// Formate 16 octets bruts (mixed-endian) de GUID sous la forme `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`.
pub fn format_guid(b: &[u8; 16]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Convertit la forme textuelle d'un GUID en octets bruts mixed-endian.
pub fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let parts: Vec<&str> = s.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    if parts.len() != 5 || parts.iter().zip(lens).any(|(p, l)| p.len() != l) {
        return None;
    }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&d1.to_le_bytes());
    out[4..6].copy_from_slice(&d2.to_le_bytes());
    out[6..8].copy_from_slice(&d3.to_le_bytes());
    let tail = format!("{}{}", parts[3], parts[4]);
    for i in 0..8 {
        out[8 + i] = u8::from_str_radix(&tail[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

const GPT_TYPES: &[(&str, &str)] = &[
    ("C12A7328-F81F-11D2-BA4B-00A0C93EC93B", "EFI System"),
    ("21686148-6449-6E6F-744E-656564454649", "BIOS boot"),
    (
        "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7",
        "Microsoft basic data",
    ),
    ("E3C9E316-0B5C-4DB8-817D-F92DF00215AE", "Microsoft reserved"),
    (
        "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC",
        "Windows recovery environment",
    ),
    (
        "5808C8AA-7E8F-42E0-85D2-E1E90434CFB3",
        "Windows LDM metadata",
    ),
    ("AF9B60A0-1431-4F62-BC68-3311714A69AD", "Windows LDM data"),
    (
        "E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D",
        "Windows Storage Spaces",
    ),
    ("0FC63DAF-8483-4772-8E79-3D69D8477DE4", "Linux filesystem"),
    ("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F", "Linux swap"),
    ("A19D880F-05FC-4D3B-A006-743F0F84911E", "Linux RAID"),
    ("E6D6D379-F507-44C2-A23C-238F2A3DF928", "Linux LVM"),
    ("933AC7E1-2EB4-4F13-B844-0E14E2AEF915", "Linux /home"),
    (
        "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
        "Linux root (x86-64)",
    ),
    ("48465300-0000-11AA-AA11-00306543ECAC", "Apple HFS/HFS+"),
    ("7C3457EF-0000-11AA-AA11-00306543ECAC", "Apple APFS"),
];

/// Nom d'un GUID de type de partition GPT bien connu.
pub fn gpt_type_name(guid: &[u8; 16]) -> Option<&'static str> {
    let s = format_guid(guid);
    GPT_TYPES.iter().find(|(g, _)| *g == s).map(|(_, n)| *n)
}
