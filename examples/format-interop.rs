//! Write public synthetic envelopes for checking with an independent reader.
use anyhow::{Context, Result};
use safeincloud_linux::vault::encrypt;
use std::{fs::OpenOptions, io::Write, path::PathBuf};

fn main() -> Result<()> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Provide a disposable output directory.")?,
    );
    std::fs::create_dir_all(&directory)?;
    let xml = include_bytes!("../tests/fixtures/interop.xml");
    for version in [1, 2] {
        let bytes = encrypt(xml, "päss🔒-Interop", version)?;
        let path = directory.join(format!("rust-v{version}-unicode.bin"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&bytes)?;
    }
    println!("Wrote only public synthetic interoperability fixtures.");
    Ok(())
}
