# diskrust

Boîte à outils d'analyse de partitions écrite en Rust, **en lecture seule
d'abord** : lecture des tables de partitions (MBR/GPT), détection des systèmes
de fichiers et recherche de partitions perdues.

> **Provenance.** `diskrust` est implémenté à partir des spécifications
> publiques des formats (UEFI GPT, structure classique MBR/EBR, documentation
> Microsoft FAT/exFAT, secteur de démarrage NTFS, format disque ext2/3/4). Il ne
> contient aucun code source tiers.
> Double licence MIT OU Apache-2.0 (voir plus bas).

## Objectifs

- Comprendre et analyser des disques sans jamais les modifier.
- De petites crates auditables, avec une séparation claire : accès aux
  périphériques, tables de partitions, signatures de systèmes de fichiers,
  recherche, ligne de commande.
- Si l'écriture existe un jour, elle sera optionnelle, derrière un trait
  distinct et un drapeau de confirmation explicite. **Rien dans ce dépôt
  n'écrit sur un disque.**

## Organisation du workspace

| Crate | Rôle |
|-------|------|
| `dr-disk` | trait `BlockDevice` ; `FileImage` (image brute `.dd`), `PhysicalDisk` (Windows `\\.\PhysicalDriveN`, Linux `/dev/sdX`), `enumerate_disks()` ; `WritableDevice` déclaré mais non implémenté |
| `dr-part` | MBR + chaîne EBR (protection contre les boucles, contrôles CHS/LBA), GPT (CRC32 de l'en-tête et des entrées, repli sur l'en-tête de secours), `detect_table_type()` |
| `dr-fs`   | détection FAT12/16/32, NTFS, exFAT, ext2/3/4 ; `probe(dev, start_lba)` |
| `dr-search` | recherche rapide (balayage aligné sur les cylindres et sur 1 Mio), `check_structure()` |
| `dr-cli`  | binaire `diskrust` |

## État d'avancement

| Fonction | État |
|----------|------|
| Lecture d'images brutes | fait |
| Périphériques bloc Linux | fait (ioctl `BLKGETSIZE64`, `BLKSSZGET`) |
| Disques physiques Windows | implémenté, compilé par la CI Windows, non testé sur matériel réel |
| Périphériques macOS/BSD | pas encore (fichiers ordinaires uniquement) |
| Analyse MBR / EBR | fait |
| Analyse GPT + repli sur la table de secours | fait |
| Détection FAT / NTFS / exFAT / ext2-4 | fait (signature et géométrie seulement) |
| Recherche rapide | fait |
| Recherche des superblocs de secours ext | À FAIRE (ébauche) |
| Recherche approfondie (autres systèmes de fichiers, parcours MFT/inodes) | À FAIRE |
| Liste des fichiers / récupération de fichiers supprimés | À FAIRE |
| Toute opération d'écriture | volontairement absente (À FAIRE, drapeau de confirmation explicite obligatoire) |

## Utilisation

```console
$ cargo build --release
$ diskrust list --image disque.dd      # liste les disques et partitions
$ diskrust list /dev/sda               # nécessite le droit de lecture sur le périphérique
$ diskrust list                        # énumère et liste tous les disques lisibles
$ diskrust analyse --image disque.dd   # table + « Structure : Ok/Bad » + recherche rapide
$ diskrust analyse /dev/sdb --log      # ajoute aussi un journal dans diskrust.log
```

Sous Windows, utiliser `\\.\PhysicalDrive0` dans un terminal élevé
(administrateur).

Développement : `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`.

## Licence

Double licence, au choix : [MIT](LICENSE-MIT) ou
[Apache-2.0](LICENSE-APACHE).
