//! Accès aux disques physiques (Windows `\\.\PhysicalDriveN`, Linux `/dev/sdX`).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::{check_range, file_read_exact_at, BlockDevice};

/// Description sommaire d'un disque trouvé par [`enumerate_disks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    /// Chemin du périphérique, p. ex. `/dev/sda` ou `\\.\PhysicalDrive0`.
    pub path: String,
    /// Taille en octets.
    pub size_bytes: u64,
    /// Taille d'un secteur logique en octets.
    pub sector_size: u32,
    /// Chaîne du modèle lorsqu'elle est disponible.
    pub model: Option<String>,
}

/// Un disque physique ouvert en lecture seule.
///
/// Toutes les lectures sont émises alignées sur les secteurs, comme l'exigent
/// les périphériques bruts (notamment sous Windows).
#[derive(Debug)]
pub struct PhysicalDisk {
    file: File,
    size: u64,
    sector_size: u32,
    path: PathBuf,
}

impl PhysicalDisk {
    /// Ouvre un périphérique (ou, par commodité, un fichier ordinaire) en lecture seule.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (file, size, sector_size) = sys::open(&path)?;
        Ok(Self {
            file,
            size,
            sector_size,
            path,
        })
    }

    /// Chemin du périphérique depuis lequel ce disque a été ouvert.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Effectue une lecture non alignée de `buf.len()` octets à `offset` en
/// n'utilisant que des lectures brutes alignées sur les secteurs.
pub(crate) fn aligned_read(
    raw: &dyn Fn(u64, &mut [u8]) -> io::Result<()>,
    sector_size: u32,
    size: u64,
    offset: u64,
    buf: &mut [u8],
) -> io::Result<()> {
    check_range(offset, buf.len(), size)?;
    if buf.is_empty() {
        return Ok(());
    }
    let ss = u64::from(sector_size);
    if offset.is_multiple_of(ss) && (buf.len() as u64).is_multiple_of(ss) {
        return raw(offset, buf);
    }
    let start = offset - offset % ss;
    let end = (offset + buf.len() as u64).div_ceil(ss) * ss;
    let mut tmp = vec![0u8; (end - start) as usize];
    raw(start, &mut tmp)?;
    let skip = (offset - start) as usize;
    buf.copy_from_slice(&tmp[skip..skip + buf.len()]);
    Ok(())
}

impl BlockDevice for PhysicalDisk {
    fn sector_size(&self) -> u32 {
        self.sector_size
    }
    fn size_bytes(&self) -> u64 {
        self.size
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        aligned_read(
            &|o, b| file_read_exact_at(&self.file, b, o),
            self.sector_size,
            self.size,
            offset,
            buf,
        )
    }
}

/// Liste les disques physiques visibles par ce processus.
///
/// Windows : sonde `\\.\PhysicalDrive0..15`. Linux : parcourt `/sys/block`.
/// Les disques impossibles à ouvrir (p. ex. privilèges manquants) sont ignorés
/// sous Windows ; sous Linux, ils sont listés à partir des données sysfs.
pub fn enumerate_disks() -> Vec<DiskInfo> {
    sys::enumerate()
}

#[cfg(target_os = "linux")]
mod sys {
    use super::DiskInfo;
    use std::fs::{self, File};
    use std::io;
    use std::os::unix::io::AsRawFd;
    use std::path::Path;

    const BLKSSZGET: u64 = 0x1268;
    /// `_IOR(0x12, 114, size_t)`
    const BLKGETSIZE64: u64 =
        (2u64 << 30) | ((std::mem::size_of::<usize>() as u64) << 16) | (0x12 << 8) | 114;

    pub fn open(path: &Path) -> io::Result<(File, u64, u32)> {
        let file = File::open(path)?;
        let fd = file.as_raw_fd();
        let mut size: u64 = 0;
        // SÉCURITÉ : `size` est un pointeur de sortie u64 valide pour BLKGETSIZE64.
        let r = unsafe { libc::ioctl(fd, BLKGETSIZE64 as _, &mut size as *mut u64) };
        let err = io::Error::last_os_error();
        let size = if r == 0 {
            size
        } else {
            let meta = file.metadata()?;
            if meta.is_file() {
                meta.len()
            } else {
                return Err(err);
            }
        };
        let mut ss: libc::c_int = 0;
        // SÉCURITÉ : `ss` est un pointeur de sortie c_int valide pour BLKSSZGET.
        let r = unsafe { libc::ioctl(fd, BLKSSZGET as _, &mut ss as *mut libc::c_int) };
        let ss = if r == 0 && ss > 0 { ss as u32 } else { 512 };
        Ok((file, size, ss))
    }

    fn read_trim(p: &Path) -> Option<String> {
        fs::read_to_string(p).ok().map(|s| s.trim().to_string())
    }

    pub fn enumerate() -> Vec<DiskInfo> {
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir("/sys/block") else {
            return out;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if ["loop", "ram", "zram", "dm-", "sr", "fd"]
                .iter()
                .any(|p| name.starts_with(p))
            {
                continue;
            }
            let base = e.path();
            let sectors512: u64 = read_trim(&base.join("size"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if sectors512 == 0 {
                continue;
            }
            let sector_size = read_trim(&base.join("queue/logical_block_size"))
                .and_then(|s| s.parse().ok())
                .unwrap_or(512);
            out.push(DiskInfo {
                path: format!("/dev/{name}"),
                size_bytes: sectors512 * 512,
                sector_size,
                model: read_trim(&base.join("device/model")),
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod sys {
    use super::DiskInfo;
    use std::fs::File;
    use std::io;
    use std::path::Path;

    // TODO : ioctl BSD/macOS (DIOCGMEDIASIZE / DKIOCGETBLOCKCOUNT). Pour l'instant,
    // seuls les fichiers ordinaires et les périphériques qui indiquent une taille
    // via leurs métadonnées fonctionnent.
    pub fn open(path: &Path) -> io::Result<(File, u64, u32)> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok((file, len, 512))
    }

    pub fn enumerate() -> Vec<DiskInfo> {
        Vec::new()
    }
}

#[cfg(windows)]
mod sys {
    use super::DiskInfo;
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::Path;
    use windows_sys::Win32::Foundation::{GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405C;
    const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00A0;

    fn ioctl_out(file: &File, code: u32, out: &mut [u8]) -> io::Result<u32> {
        let mut returned = 0u32;
        // SÉCURITÉ : le handle est valide pendant toute la durée de vie de `file` ;
        // `out` est un tampon inscriptible valide de la longueur indiquée ; aucun
        // tampon d'entrée n'est utilisé.
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle() as HANDLE,
                code,
                std::ptr::null(),
                0,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(returned)
        }
    }

    pub fn open(path: &Path) -> io::Result<(File, u64, u32)> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SÉCURITÉ : `wide` est une chaîne UTF-16 terminée par NUL ; accès en lecture seule.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SÉCURITÉ : `handle` est un handle valide et possédé, renvoyé par CreateFileW.
        let file = unsafe { File::from_raw_handle(handle as RawHandle) };

        // DISK_GEOMETRY_EX : DISK_GEOMETRY (24 octets, BytesPerSector à +20) puis DiskSize (i64 à +24).
        let mut geo = [0u8; 128];
        let geo_ok = ioctl_out(&file, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &mut geo).is_ok();
        let bps = u32::from_le_bytes(geo[20..24].try_into().unwrap());
        let sector_size = if geo_ok && bps >= 512 { bps } else { 512 };

        let mut len = [0u8; 8];
        let size = if ioctl_out(&file, IOCTL_DISK_GET_LENGTH_INFO, &mut len).is_ok() {
            i64::from_le_bytes(len) as u64
        } else if geo_ok {
            i64::from_le_bytes(geo[24..32].try_into().unwrap()) as u64
        } else {
            let meta = file.metadata()?;
            if meta.is_file() {
                meta.len()
            } else {
                return Err(io::Error::last_os_error());
            }
        };
        Ok((file, size, sector_size))
    }

    pub fn enumerate() -> Vec<DiskInfo> {
        use crate::BlockDevice;
        (0..16)
            .filter_map(|n| {
                let path = format!(r"\\.\PhysicalDrive{n}");
                let d = super::PhysicalDisk::open(&path).ok()?;
                Some(DiskInfo {
                    path,
                    size_bytes: d.size_bytes(),
                    sector_size: d.sector_size(),
                    model: None,
                })
            })
            .collect()
    }
}

#[cfg(not(any(unix, windows)))]
mod sys {
    use super::DiskInfo;
    use std::fs::File;
    use std::io;
    use std::path::Path;

    pub fn open(_path: &Path) -> io::Result<(File, u64, u32)> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "les disques physiques ne sont pas pris en charge sur cette plateforme",
        ))
    }

    pub fn enumerate() -> Vec<DiskInfo> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_read_handles_unaligned_ranges() {
        let data: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let raw = |o: u64, b: &mut [u8]| -> io::Result<()> {
            assert_eq!(
                o % 512,
                0,
                "la lecture brute doit être alignée sur les secteurs"
            );
            assert_eq!(
                b.len() % 512,
                0,
                "la longueur de lecture brute doit être alignée sur les secteurs"
            );
            b.copy_from_slice(&data[o as usize..o as usize + b.len()]);
            Ok(())
        };
        let mut buf = [0u8; 700];
        aligned_read(&raw, 512, 4096, 300, &mut buf).unwrap();
        assert_eq!(&buf[..], &data[300..1000]);
        let mut buf = [0u8; 1024];
        aligned_read(&raw, 512, 4096, 1024, &mut buf).unwrap();
        assert_eq!(&buf[..], &data[1024..2048]);
        assert!(aligned_read(&raw, 512, 4096, 4000, &mut buf).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn physical_disk_opens_regular_file_fallback() {
        let dir = std::env::temp_dir().join(format!("dr-phys-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("disk.bin");
        std::fs::write(&p, vec![7u8; 8192]).unwrap();
        let d = PhysicalDisk::open(&p).unwrap();
        assert_eq!(d.size_bytes(), 8192);
        let mut b = [0u8; 3];
        d.read_at(1, &mut b).unwrap();
        assert_eq!(b, [7, 7, 7]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn enumerate_does_not_panic() {
        let _ = enumerate_disks();
    }
}
