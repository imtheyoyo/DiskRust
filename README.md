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
| `dr-gui`  | binaire `diskrust-gui` : interface graphique `eframe`/`egui` (lecture seule), modèle sans écran testable dans `model.rs` |

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
| Interface graphique (`dr-gui`) | fait (compilée par la CI Windows ; affichage non vérifié sur machine Windows réelle) |
| Recherche des superblocs de secours ext | À FAIRE (ébauche) |
| Recherche approfondie (autres systèmes de fichiers, parcours MFT/inodes) | À FAIRE |
| Liste des fichiers / récupération de fichiers supprimés | À FAIRE |
| Toute opération d'écriture | volontairement absente (À FAIRE, drapeau de confirmation explicite obligatoire) |

## Interface graphique (Windows)

`diskrust-gui` est une fenêtre `eframe`/`egui` strictement **en lecture seule** :
un bandeau permanent rappelle qu'aucune écriture n'est effectuée sur le disque.

```console
$ cargo build --release -p dr-gui
$ target\release\diskrust-gui.exe        # Windows
$ cargo run --release -p dr-gui           # lancement direct
```

Description de l'interface :

- **Panneau de gauche** : liste des disques physiques (chemin et taille), boutons
  « Actualiser » et « Ouvrir une image… » (boîte de dialogue native, fichiers
  `.dd`, `.img`, `.raw` ou tous les fichiers).
- **Panneau central** : taille et octets par secteur du disque, type de table de
  partitions, avertissements et erreurs, tableau des partitions (numéro,
  amorçable, genre, type, début, fin, taille, système de fichiers et étiquette),
  « Structure : Ok / Mauvaise » avec la liste des problèmes.
- **Recherche rapide** : exécutée dans un fil d'exécution séparé avec barre de
  progression ; les systèmes de fichiers trouvés sont marqués « dans la table »
  ou « candidat à la récupération ». Il n'y a pas de bouton « Annuler » : l'API
  de recherche ne permet pas d'interrompre un balayage en cours.
- **Rapport** : « Exporter le rapport… » (fichier `.txt`) et « Copier le
  rapport » (presse-papiers), avec le même texte que `diskrust list` /
  `diskrust analyse` (via la bibliothèque `dr-cli`).
- Barre d'état et fenêtre « À propos » (version, licence MIT OU Apache-2.0).

**Droits d'administrateur.** Le manifeste Windows demande `asInvoker` : l'ouverture
d'une image `.dd` ne nécessite aucun privilège, alors que `requireAdministrator`
l'imposerait même pour de simples fichiers. Pour lister et lire les disques
physiques (`\\.\PhysicalDriveN`), lancez le programme en tant qu'administrateur
(l'interface le rappelle : « Pour lister les disques physiques, lancez le
programme en tant qu'administrateur »).

**Garantie de lecture seule.** Les disques et images sont ouverts uniquement en
lecture ; le seul fichier écrit est le rapport `.txt` choisi explicitement par
l'utilisateur.

Sans écran (CI Linux, tests) : `cargo test -p dr-gui --no-default-features`
ne compile que le modèle (`model.rs`) et ses tests sur images synthétiques.

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
