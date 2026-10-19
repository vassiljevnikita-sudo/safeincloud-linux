//! Verify a locally supplied compatibility reference without printing its values.
use anyhow::{Context, Result};
use safeincloud_linux::{dotnet, session};
use std::path::PathBuf;

fn main() -> Result<()> {
    session::prevent_core_dumps();
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Provide the locally supplied SafeInCloud.exe path.")?,
    );
    let configuration = dotnet::extract_client(&path)?;
    drop(configuration);
    println!("Original client configuration is supported. No values were printed or saved.");
    Ok(())
}
