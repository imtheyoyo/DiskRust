use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use dr_disk::{enumerate_disks, BlockDevice, FileImage, PhysicalDisk};

/// Read-only partition recovery toolkit (clean-room implementation).
#[derive(Parser)]
#[command(name = "diskrust", version, about)]
struct Cli {
    /// Append a diskrust.log-like transcript to FILE (default: diskrust.log).
    #[arg(
        long,
        global = true,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "diskrust.log"
    )]
    log: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Target {
    /// Raw disk image file (.dd) instead of a device.
    #[arg(long, value_name = "FILE", conflicts_with = "device")]
    image: Option<PathBuf>,
    /// Device path, e.g. /dev/sda or \\.\PhysicalDrive0.
    device: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// List disks and their partition tables (disk and partition listing).
    List(Target),
    /// Analyse a disk: partition table, structure check and Quick Search.
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
                .map_err(|e| format!("cannot open log file {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let mut out = Output { log };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    out.line(&format!(
        "diskrust {} (read-only) - unix time {secs}",
        env!("CARGO_PKG_VERSION")
    ));
    out.line(&format!(
        "Command line: {}",
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
                return Err("analyse needs --image FILE or a DEVICE".to_string());
            }
            for d in enumerate_disks() {
                match PhysicalDisk::open(&d.path) {
                    Ok(dev) => targets.push((d.path, Box::new(dev))),
                    Err(e) => out.line(&format!("Disk {} - cannot open: {e}", d.path)),
                }
            }
            if targets.is_empty() {
                out.line("No readable disk found (try running with elevated privileges).");
            }
        }
        (img, dev) => {
            let dev = dev.clone().unwrap_or_default();
            targets.push(open(img, &dev).map_err(|e| format!("cannot open target: {e}"))?);
        }
    }

    let tty = std::io::stderr().is_terminal();
    for (label, dev) in &targets {
        let lines = if analyse {
            dr_cli::analyse_report(label, dev.as_ref(), &mut |p| {
                if tty {
                    eprint!("\rQuick Search: {:>3}%", p.percent());
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
