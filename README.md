# diskrust

A **read-only-first** partition analysis toolkit written in Rust: partition
table parsing (MBR/GPT), filesystem detection and lost-partition search.

> **Provenance.** `diskrust` is implemented from public format specifications
> (UEFI GPT, the classic MBR/EBR layout, Microsoft FAT/exFAT documentation,
> NTFS boot sector layout, ext2/3/4 on-disk format docs). It contains no
> third-party source code.
> Licensed under MIT OR Apache-2.0 (see below).

## Goals

- Understand and analyse disks without ever modifying them.
- Small, auditable crates with a clear separation: device access, partition
  tables, filesystem signatures, searching, CLI.
- Write support, if it ever exists, will be opt-in, behind a separate trait and
  an explicit confirmation flag. **Nothing in this repository writes to a disk.**

## Workspace layout

| Crate | Purpose |
|-------|---------|
| `dr-disk` | `BlockDevice` trait; `FileImage` (raw `.dd`), `PhysicalDisk` (Windows `\\.\PhysicalDriveN`, Linux `/dev/sdX`), `enumerate_disks()`; `WritableDevice` declared but unimplemented |
| `dr-part` | MBR + EBR chain (loop protection, CHS/LBA checks), GPT (header and entry CRC32, backup-header fallback), `detect_table_type()` |
| `dr-fs`   | FAT12/16/32, NTFS, exFAT, ext2/3/4 detection; `probe(dev, start_lba)` |
| `dr-search` | Quick Search (cylinder- and 1 MiB-aligned scan), `check_structure()` |
| `dr-cli`  | `diskrust` binary |

## Status

| Feature | Status |
|---------|--------|
| Raw image reading | done |
| Linux block devices | done (ioctl `BLKGETSIZE64`, `BLKSSZGET`) |
| Windows physical drives | implemented, CI-built on Windows, untested on real hardware |
| macOS/BSD devices | not yet (regular files only) |
| MBR / EBR parsing | done |
| GPT parsing + backup fallback | done |
| FAT / NTFS / exFAT / ext2-4 detection | done (signature + geometry only) |
| Quick Search | done |
| ext backup-superblock search | TODO (stub) |
| Deeper Search (extra FS types, MFT/inode scans) | TODO |
| Filesystem listing / file undelete | TODO |
| Any write operation | intentionally absent (TODO, must require explicit confirmation flag) |

## Usage

```console
$ cargo build --release
$ diskrust list --image disk.dd        # list disks and partitions
$ diskrust list /dev/sda               # needs read permission on the device
$ diskrust list                        # enumerate and list all readable disks
$ diskrust analyse --image disk.dd     # table + "Structure: Ok/Bad" + Quick Search
$ diskrust analyse /dev/sdb --log      # also append a transcript to diskrust.log
```

On Windows use `\\.\PhysicalDrive0` and an elevated terminal.

Development: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`.

## Section francaise

`diskrust` est une boite a outils d'analyse de partitions ecrite en Rust a partir
des specifications publiques des formats (MBR, GPT, FAT, NTFS, exFAT, ext2/3/4),
sans reprise de code tiers. Elle fonctionne en **lecture seule** : aucune
ecriture sur le disque n'est implementee. Commandes :
`diskrust list` (liste des disques et partitions) et `diskrust analyse`
(table de partitions, verification de structure, recherche rapide).
Les ecritures futures devront imposer un drapeau de confirmation explicite.
Voir le tableau d'etat ci-dessus pour les fonctions manquantes.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
