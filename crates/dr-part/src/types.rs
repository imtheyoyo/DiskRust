use std::fmt;
use std::io;

use crate::gpt::{format_guid, gpt_type_name, GptError};
use crate::mbr::mbr_type_name;

/// Mode de stockage d'une partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Partition primaire MBR.
    Primary,
    /// Partition étendue servant de conteneur (MBR).
    Extended,
    /// Partition logique à l'intérieur d'une partition étendue (chaîne EBR).
    Logical,
    /// Partition GPT.
    Gpt,
}

impl fmt::Display for PartKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PartKind::Primary => "Primaire",
            PartKind::Extended => "Étendue",
            PartKind::Logical => "Logique",
            PartKind::Gpt => "GPT",
        })
    }
}

/// Identifiant de type de partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeId {
    /// Identifiant système MBR sur un octet.
    Mbr(u8),
    /// GUID de type GPT, octets bruts tels que sur le disque (mixed-endian).
    Gpt([u8; 16]),
}

/// Partition décrite par une table (ou découverte par une recherche).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// Premier secteur (LBA absolu).
    pub start_lba: u64,
    /// Longueur en secteurs.
    pub size_sectors: u64,
    /// Mode de stockage.
    pub kind: PartKind,
    /// Identifiant de type (octet MBR ou GUID GPT).
    pub type_id: TypeId,
    /// Nom lisible (nom GPT, ou description du type pour MBR).
    pub name: String,
    /// Indicateur actif/amorçable (MBR 0x80, attribut GPT « amorçable BIOS hérité »).
    pub bootable: bool,
}

impl Partition {
    /// Dernier secteur (inclus). Égal à `start_lba` pour les partitions vides.
    pub fn end_lba(&self) -> u64 {
        self.start_lba + self.size_sectors.saturating_sub(1)
    }
}

/// Famille de table de partitions trouvée sur un disque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableType {
    /// Table de partitions Intel/PC (MBR).
    Intel,
    /// GPT EFI.
    Gpt,
    /// Disque vierge (aucune table).
    None,
    /// Il y a des données, mais ce n'est pas une table reconnue.
    Unknown,
}

impl fmt::Display for TableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TableType::Intel => "Intel",
            TableType::Gpt => "GPT EFI",
            TableType::None => "Aucune",
            TableType::Unknown => "Inconnue",
        })
    }
}

/// Description lisible du type d'une partition.
pub fn type_label(p: &Partition) -> String {
    match p.type_id {
        TypeId::Mbr(t) => mbr_type_name(t).to_string(),
        TypeId::Gpt(g) => gpt_type_name(&g)
            .map(str::to_string)
            .unwrap_or_else(|| format_guid(&g)),
    }
}

/// Erreurs produites par les analyseurs de tables.
#[derive(Debug)]
pub enum PartError {
    /// Échec d'E/S sous-jacent.
    Io(io::Error),
    /// Le secteur 0 ne se termine pas par 0x55AA.
    BadMbrSignature,
    /// Aucune GPT exploitable (primaire et secours invalides). Porte le journal de diagnostic.
    NoValidGpt(Vec<String>),
    /// Un problème GPT précis.
    Gpt(GptError),
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartError::Io(e) => write!(f, "Erreur d'E/S : {e}"),
            PartError::BadMbrSignature => f.write_str("MBR invalide : signature 0x55AA absente"),
            PartError::NoValidGpt(log) => {
                write!(f, "Aucune GPT valide trouvée ({})", log.join("; "))
            }
            PartError::Gpt(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PartError {}

impl From<io::Error> for PartError {
    fn from(e: io::Error) -> Self {
        PartError::Io(e)
    }
}

impl From<GptError> for PartError {
    fn from(e: GptError) -> Self {
        PartError::Gpt(e)
    }
}
