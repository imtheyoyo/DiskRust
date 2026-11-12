//! Abstraction de périphérique bloc en lecture seule pour `diskrust`.
//!
//! L'élément central est le trait [`BlockDevice`]. L'écriture en est
//! volontairement exclue : tout futur support d'écriture devra passer par le
//! trait distinct [`WritableDevice`], qu'aucun type de cette crate n'implémente.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

mod physical;

pub use physical::{enumerate_disks, DiskInfo, PhysicalDisk};

/// Un périphérique bloc en lecture seule à accès aléatoire.
pub trait BlockDevice {
    /// Taille d'un secteur logique en octets (généralement 512 ou 4096).
    fn sector_size(&self) -> u32;
    /// Taille totale du périphérique en octets.
    fn size_bytes(&self) -> u64;
    /// Lit exactement `buf.len()` octets à partir de l'octet `offset`.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;

    /// Nombre de secteurs entiers du périphérique.
    fn sector_count(&self) -> u64 {
        self.size_bytes() / u64::from(self.sector_size())
    }

    /// Lit `count` secteurs à partir de `lba` dans un nouveau tampon.
    fn read_sectors(&self, lba: u64, count: usize) -> io::Result<Vec<u8>> {
        let ss = u64::from(self.sector_size());
        let offset = lba.checked_mul(ss).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "dépassement de capacité du LBA",
            )
        })?;
        let mut buf = vec![0u8; count * ss as usize];
        self.read_at(offset, &mut buf)?;
        Ok(buf)
    }
}

/// Accès en écriture à un périphérique.
///
/// TODO : volontairement NON implémenté par aucun périphérique pour le
/// moment. `diskrust` est strictement en lecture seule. Toute implémentation
/// future ne devra être accessible que derrière un indicateur de confirmation
/// explicite de l'utilisateur.
pub trait WritableDevice: BlockDevice {
    /// Écrit `data` à l'octet `offset`.
    fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()>;
}

pub(crate) fn check_range(offset: u64, len: usize, size: u64) -> io::Result<()> {
    let end = offset.checked_add(len as u64).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "dépassement de capacité du décalage",
        )
    })?;
    if end > size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "lecture de {len} octets au décalage {offset} au-delà de la fin du périphérique ({size} octets)"
            ),
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn file_read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(windows)]
pub(crate) fn file_read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn file_read_exact_at(_file: &File, _buf: &mut [u8], _offset: u64) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "les lectures positionnelles ne sont pas prises en charge sur cette plateforme",
    ))
}

/// Une image disque brute (`.dd`) ouverte en lecture seule.
#[derive(Debug)]
pub struct FileImage {
    file: File,
    size: u64,
    sector_size: u32,
    path: PathBuf,
}

impl FileImage {
    /// Ouvre `path` en lecture seule avec des secteurs de 512 octets.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            size,
            sector_size: 512,
            path,
        })
    }

    /// Remplace la taille de secteur (p. ex. 4096 pour une image 4Kn).
    pub fn with_sector_size(mut self, sector_size: u32) -> Self {
        assert!(sector_size >= 512 && sector_size.is_power_of_two());
        self.sector_size = sector_size;
        self
    }

    /// Chemin depuis lequel cette image a été ouverte.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl BlockDevice for FileImage {
    fn sector_size(&self) -> u32 {
        self.sector_size
    }
    fn size_bytes(&self) -> u64 {
        self.size
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.size)?;
        file_read_exact_at(&self.file, buf, offset)
    }
}

/// Une image en mémoire. Surtout utile pour les tests et les données synthétiques.
#[derive(Debug, Clone)]
pub struct MemImage {
    data: Vec<u8>,
    sector_size: u32,
}

impl MemImage {
    /// Encapsule des octets existants (secteurs de 512 octets).
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            sector_size: 512,
        }
    }

    /// Une image de `size` octets remplie de zéros.
    pub fn zeroed(size: usize) -> Self {
        Self::new(vec![0; size])
    }

    /// Remplace la taille de secteur.
    pub fn with_sector_size(mut self, sector_size: u32) -> Self {
        self.sector_size = sector_size;
        self
    }

    /// Accès mutable aux octets sous-jacents (c'est un tampon en RAM, pas un périphérique).
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Copie `src` dans le tampon à `offset`.
    pub fn put(&mut self, offset: usize, src: &[u8]) {
        self.data[offset..offset + src.len()].copy_from_slice(src);
    }

    /// Octets sous-jacents.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
}

impl BlockDevice for MemImage {
    fn sector_size(&self) -> u32 {
        self.sector_size
    }
    fn size_bytes(&self) -> u64 {
        self.data.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        check_range(offset, buf.len(), self.size_bytes())?;
        let o = offset as usize;
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }
}

/// Formate un nombre d'octets de façon lisible : `"500 GB / 465 GiB"`.
pub fn format_size(bytes: u64) -> String {
    fn scaled(bytes: u64, base: f64, units: &[&str]) -> String {
        let mut v = bytes as f64;
        let mut i = 0;
        while v >= base && i + 1 < units.len() {
            v /= base;
            i += 1;
        }
        format!("{:.0} {}", v, units[i])
    }
    format!(
        "{} / {}",
        scaled(bytes, 1000.0, &["B", "kB", "MB", "GB", "TB", "PB"]),
        scaled(bytes, 1024.0, &["B", "KiB", "MiB", "GiB", "TiB", "PiB"])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_image_reads_and_bounds() {
        let mut img = MemImage::zeroed(2048);
        img.put(512, b"hello");
        let mut b = [0u8; 5];
        img.read_at(512, &mut b).unwrap();
        assert_eq!(&b, b"hello");
        assert_eq!(img.sector_count(), 4);
        assert_eq!(&img.read_sectors(1, 1).unwrap()[..5], b"hello");
        assert!(img.read_at(2046, &mut b).is_err());
    }

    #[test]
    fn file_image_roundtrip() {
        let dir = std::env::temp_dir().join(format!("dr-disk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.dd");
        let mut data = vec![0u8; 4096];
        data[1000..1004].copy_from_slice(&[1, 2, 3, 4]);
        std::fs::write(&p, &data).unwrap();
        let img = FileImage::open(&p).unwrap();
        assert_eq!(img.size_bytes(), 4096);
        let mut b = [0u8; 4];
        img.read_at(1000, &mut b).unwrap();
        assert_eq!(b, [1, 2, 3, 4]);
        assert!(img.read_at(4094, &mut b).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_formatting() {
        assert_eq!(format_size(64 * 1024 * 1024), "67 MB / 64 MiB");
        assert_eq!(format_size(100), "100 B / 100 B");
    }
}
