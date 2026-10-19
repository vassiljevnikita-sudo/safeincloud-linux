mod ui;

use anyhow::{bail, Result};
use safeincloud_linux::{passwords, session, vault};
use std::path::PathBuf;

#[derive(Default)]
pub struct Arguments {
    pub path: Option<PathBuf>,
    pub recovery: Option<session::Recovery>,
    pub notice: Option<String>,
    pub demo: bool,
    pub smoke: bool,
    pub screenshot: Option<PathBuf>,
}

fn arguments() -> Result<Arguments> {
    let mut result = Arguments::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--vault") => {
                result.path = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Datenbankpfad fehlt."))?
                        .into(),
                )
            }
            Some("--recovery-fd") => {
                let fd = args
                    .next()
                    .and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
                    .ok_or_else(|| anyhow::anyhow!("Wiederherstellungskanal fehlt."))?;
                result.recovery = Some(session::read_recovery(fd)?);
            }
            Some("--notice") => {
                result.notice = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--demo") => result.demo = true,
            Some("--smoke-test") => {
                result.demo = true;
                result.smoke = true;
            }
            Some("--screenshot") => {
                result.screenshot = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("Bildpfad fehlt."))?
                        .into(),
                )
            }
            Some("--gui") => {} // Existing desktop launchers remain compatible.
            Some("--version") | Some("-V") => {
                println!(
                    "SafeInCloud Linux {} (Rust + GTK4)",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            Some("--help") | Some("-h") => {
                println!("SafeInCloud Linux\n\n  safeincloud-linux [--vault DATEI]\n  safeincloud-linux --demo\n  safeincloud-linux --smoke-test\n\nStrg+F Suchen · Strg+N Neu · Strg+E Bearbeiten · Strg+G Generator · Strg+L Sperren");
                std::process::exit(0);
            }
            Some(value) if value.starts_with('-') => bail!("Unbekannte Option. Siehe --help."),
            _ if result.path.is_none() => result.path = Some(arg.into()),
            _ => bail!("Bitte nur eine Datenbank auswählen."),
        }
    }
    if result.demo && (result.path.is_some() || result.recovery.is_some()) {
        bail!("Die Demo verwendet ausschließlich fiktive Daten.");
    }
    Ok(result)
}

fn main() -> Result<()> {
    session::prevent_core_dumps();
    let args = arguments()?;
    ui::run(args);
    Ok(())
}
