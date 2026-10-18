//! Synthetic process-replacement probe. Contains no user credentials or vaults.
use anyhow::{Context, Result};
use safeincloud_linux::{
    session::{self, Recovery},
    vault::{encrypt, Field, Vault},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static HEAP_CANARY: AtomicUsize = AtomicUsize::new(0);
const PASSWORD: &str = "public-synthetic-probe-only";

fn main() -> Result<()> {
    session::prevent_core_dumps();
    let args: Vec<_> = std::env::args().collect();
    if std::env::var("SAFEINCLOUD_BOUNDARY_PROBE").as_deref() == Ok("after-exec") {
        assert_eq!(
            HEAP_CANARY.load(Ordering::SeqCst),
            0,
            "old heap must be gone"
        );
        let path = args
            .windows(2)
            .find(|a| a[0] == "--vault")
            .map(|a| PathBuf::from(&a[1]))
            .context("probe path")?;
        let fd: i32 = args
            .windows(2)
            .find(|a| a[0] == "--recovery-fd")
            .context("recovery descriptor")?[1]
            .parse()?;
        let recovery = session::read_recovery(fd)?;
        let serialized = serde_json::to_string(&recovery)?;
        assert!(!serialized.contains(PASSWORD));
        assert!(!serialized.contains("pending-synthetic-secret"));
        let heartbeat = path.with_extension("heartbeat");
        let before = std::fs::read(&heartbeat)?;
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            std::fs::read(&heartbeat)?,
            before,
            "old worker must be gone"
        );
        assert!(recovery.unlock(&path, "incorrect").is_err());
        let mut vault = recovery.unlock(&path, PASSWORD)?;
        assert!(vault.dirty);
        vault.save()?;
        assert_eq!(
            Vault::open(&path, PASSWORD)?.cards()[0].fields[0].value,
            "pending-synthetic-secret"
        );
        drop(vault);
        std::fs::remove_dir_all(path.parent().context("probe directory")?)?;
        println!("Process boundary OK: old heap/workers gone; ciphertext-only recovery requires reauthentication.");
        return Ok(());
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("probe.db");
    std::fs::write(&path, encrypt(b"<database/>", PASSWORD, 2)?)?;
    let mut vault = Vault::open(&path, PASSWORD)?;
    vault.upsert(
        None,
        "Pending probe",
        &[Field {
            name: "Password".into(),
            kind: "password".into(),
            value: "pending-synthetic-secret".into(),
        }],
        "",
    )?;
    let recovery = Recovery::capture(&vault, None)?;
    HEAP_CANARY.store(42, Ordering::SeqCst);
    let heartbeat = path.with_extension("heartbeat");
    std::fs::write(&heartbeat, b"initial")?;
    std::thread::spawn(move || {
        for counter in 0u64..=u64::MAX {
            let _ = std::fs::write(&heartbeat, counter.to_le_bytes());
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    std::env::set_var("SAFEINCLOUD_BOUNDARY_PROBE", "after-exec");
    session::restart_locked(Some(&path), Some(&recovery), None)
}
