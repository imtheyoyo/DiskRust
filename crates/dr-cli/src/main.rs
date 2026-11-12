use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use dr_disk::{enumerate_disks, BlockDevice, FileImage, PhysicalDisk};

/// Boîte à outils d'analyse de partitions en lecture seule.
#[derive(Parser)]
#[command(name = "diskrust", version, about)]
struct Cli {
    /// Ajoute une transcription dans FICHIER (par défaut : diskrust.log).
    #[arg(
        long,
        global = true,
        value_name = "FICHIER",
        num_args = 0..=1,
        default_missing_value = "diskrust.log"
    )]
    log: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Target {
    /// Fichier image disque brut (.dd) au lieu d'un périphérique.
    #[arg(long, value_name = "FICHIER", conflicts_with = "device")]
    image: Option<PathBuf>,
    /// Chemin du périphérique, par ex. /dev/sda ou \\.\PhysicalDrive0.
    device: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Liste les disques et leurs tables de partitions.
    List(Target),
    /// Analyse un disque : table de partitions, vérification de la structure et recherche rapide.
    Analyse(Target),
}

struct Output {
    log: Option<std::fs::File>,
}

impl Output {
    fn line(&mut self, s: &str) {
        println!("{s}");
        if let Some(f) = &mut self.log {
            let _ = writeln!(f, "{s}");
        }
    }
}

fn open(image: &Option<PathBuf>, device: &str) -> std::io::Result<(String, Box<dyn BlockDevice>)> {
    match image {
        Some(p) => Ok((p.display().to_string(), Box::new(FileImage::open(p)?))),
        None => Ok((device.to_string(), Box::new(PhysicalDisk::open(device)?))),
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let log = match &cli.log {
        Some(p) => Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| {
                    format!(
                        "impossible d'ouvrir le fichier journal {} : {e}",
                        p.display()
                    )
                })?,
        ),
        None => None,
    };
    let mut out = Output { log };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    out.line(&format!(
        "diskrust {} (lecture seule) - heure unix {secs}",
        env!("CARGO_PKG_VERSION")
    ));
    out.line(&format!(
        "Ligne de commande : {}",
        std::env::args().collect::<Vec<_>>().join(" ")
    ));

    let (analyse, target) = match cli.command {
        Command::List(t) => (false, t),
        Command::Analyse(t) => (true, t),
    };

    let mut targets: Vec<(String, Box<dyn BlockDevice>)> = Vec::new();
    match (&target.image, &target.device) {
        (None, None) => {
            if analyse {
                return Err("analyse nécessite --image FICHIER ou un PÉRIPHÉRIQUE".to_string());
            }
            for d in enumerate_disks() {
                match PhysicalDisk::open(&d.path) {
                    Ok(dev) => targets.push((d.path, Box::new(dev))),
                    Err(e) => out.line(&format!("Disque {} - ouverture impossible : {e}", d.path)),
                }
            }
            if targets.is_empty() {
                out.line("Aucun disque lisible trouvé (essayez avec des privilèges élevés).");
            }
        }
        (img, dev) => {
            let dev = dev.clone().unwrap_or_default();
            targets
                .push(open(img, &dev).map_err(|e| format!("impossible d'ouvrir la cible : {e}"))?);
        }
    }

    let tty = std::io::stderr().is_terminal();
    for (label, dev) in &targets {
        let lines = if analyse {
            dr_cli::analyse_report(label, dev.as_ref(), &mut |p| {
                if tty {
                    eprint!("\rRecherche rapide : {:>3} %", p.percent());
                }
            })
        } else {
            dr_cli::list_report(label, dev.as_ref())
        };
        if analyse && tty {
            eprintln!();
        }
        for l in lines {
            out.line(&l);
        }
        out.line("");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("diskrust: {e}");
            ExitCode::FAILURE
        }
    }
}
