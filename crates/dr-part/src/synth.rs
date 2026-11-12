//! Builders for synthetic partition tables (used by tests in this workspace).

use crate::gpt::parse_guid;

/// Encode an LBA as 3 CHS bytes using 255/63 geometry (saturating at 1023).
pub fn lba_to_chs(lba: u64) -> [u8; 3] {
    let (h, s) = (255u64, 63u64);
    let cyl = lba / (h * s);
    if cyl > 1023 {
        return [254, 0xFF, 0xFF];
    }
    let head = (lba / s) % h;
    let sec = lba % s + 1;
    [
        head as u8,
        (sec as u8 & 0x3F) | (((cyl >> 2) as u8) & 0xC0),
        (cyl & 0xFF) as u8,
    ]
}

/// A 16-byte MBR partition entry with consistent CHS values.
pub fn mbr_entry(boot: u8, type_id: u8, start: u32, size: u32) -> [u8; 16] {
    let mut e = [0u8; 16];
    e[0] = boot;
    e[1..4].copy_from_slice(&lba_to_chs(u64::from(start)));
    e[4] = type_id;
    e[5..8].copy_from_slice(&lba_to_chs(u64::from(start) + u64::from(size) - 1));
    e[8..12].copy_from_slice(&start.to_le_bytes());
    e[12..16].copy_from_slice(&size.to_le_bytes());
    e
}

/// A 512-byte sector with up to four entries and the 0x55AA signature.
pub fn mbr_sector(entries: &[[u8; 16]]) -> [u8; 512] {
    let mut s = [0u8; 512];
    for (i, e) in entries.iter().take(4).enumerate() {
        s[446 + 16 * i..446 + 16 * (i + 1)].copy_from_slice(e);
    }
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

/// One GPT partition to synthesise: (type GUID text, first LBA, last LBA, name).
pub type GptSpec<'a> = (&'a str, u64, u64, &'a str);

/// Build a complete GPT disk image (protective MBR, both headers, both entry arrays).
pub fn gpt_image(total_sectors: u64, parts: &[GptSpec<'_>]) -> Vec<u8> {
    let ss = 512usize;
    let mut img = vec![0u8; total_sectors as usize * ss];
    let last = total_sectors - 1;
    let prot = mbr_sector(&[mbr_entry(
        0,
        0xEE,
        1,
        u32::try_from(last).unwrap_or(u32::MAX),
    )]);
    img[..512].copy_from_slice(&prot);

    let mut entries = vec![0u8; 128 * 128];
    for (i, (guid, first, lst, name)) in parts.iter().enumerate() {
        let e = &mut entries[i * 128..(i + 1) * 128];
        e[0..16].copy_from_slice(&parse_guid(guid).expect("valid type guid"));
        e[16..32].copy_from_slice(&[0x11 + i as u8; 16]);
        e[32..40].copy_from_slice(&first.to_le_bytes());
        e[40..48].copy_from_slice(&lst.to_le_bytes());
        for (j, u) in name.encode_utf16().take(36).enumerate() {
            e[56 + 2 * j..58 + 2 * j].copy_from_slice(&u.to_le_bytes());
        }
    }
    let entries_crc = crc32fast::hash(&entries);
    let backup_entries_lba = last - 32;

    let header = |my: u64, alt: u64, entries_lba: u64| -> [u8; 512] {
        let mut h = [0u8; 512];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        h[12..16].copy_from_slice(&92u32.to_le_bytes());
        h[24..32].copy_from_slice(&my.to_le_bytes());
        h[32..40].copy_from_slice(&alt.to_le_bytes());
        h[40..48].copy_from_slice(&34u64.to_le_bytes());
        h[48..56].copy_from_slice(&(last - 33).to_le_bytes());
        h[56..72].copy_from_slice(&[0xAB; 16]);
        h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        h[80..84].copy_from_slice(&128u32.to_le_bytes());
        h[84..88].copy_from_slice(&128u32.to_le_bytes());
        h[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let crc = crc32fast::hash(&h[..92]);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        h
    };

    img[ss..2 * ss].copy_from_slice(&header(1, last, 2));
    img[2 * ss..2 * ss + entries.len()].copy_from_slice(&entries);
    let b = backup_entries_lba as usize * ss;
    img[b..b + entries.len()].copy_from_slice(&entries);
    let lh = last as usize * ss;
    img[lh..lh + 512].copy_from_slice(&header(last, 1, backup_entries_lba));
    img
}
