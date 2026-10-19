//! Replace the entire secret-bearing process on lock. Recovery crosses exec
//! only as ciphertext in an anonymous file descriptor, never a master key.
use crate::{drive::DriveSessionState, vault::Vault};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Recovery {
    pub ciphertext: String,
    pub snapshot: [u8; 32],
    pub sync: Option<DriveSessionState>,
}

impl Recovery {
    pub fn capture(vault: &Vault, sync: Option<DriveSessionState>) -> Result<Self> {
        Ok(Self {
            ciphertext: STANDARD.encode(vault.encrypted()?),
            snapshot: vault.snapshot_digest(),
            sync,
        })
    }

    pub fn unlock(&self, path: &Path, password: &str) -> Result<Vault> {
        let bytes = STANDARD
            .decode(&self.ciphertext)
            .context("Die Wiederherstellung ist beschädigt.")?;
        let mut vault = Vault::from_encrypted(path, &bytes, password)?;
        vault.restore_pending_baseline(self.snapshot);
        Ok(vault)
    }
}

pub fn prevent_core_dumps() {
    // Unlike PR_SET_DUMPABLE=0, RLIMIT_CORE does not break desktop portals.
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &limit);
    }
}

fn recovery_file(recovery: &Recovery) -> Result<File> {
    let name = CString::new("safeincloud-encrypted-recovery")?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("Verschlüsselte Wiederherstellung nicht verfügbar.");
    }
    // SAFETY: a newly created owned descriptor, not aliased with another File.
    let mut file = unsafe { File::from_raw_fd(fd) };
    serde_json::to_writer(&mut file, recovery)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

pub fn read_recovery(fd: i32) -> Result<Recovery> {
    if fd < 3 {
        bail!("Ungültiger Wiederherstellungskanal.");
    }
    // SAFETY: this descriptor was intentionally inherited by our own exec.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.file_type().is_file() {
        bail!("Ungültiger Wiederherstellungskanal.");
    }
    let limit = (crate::vault::MAX_FILE * 2 + 16384) as u64;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("Die Wiederherstellung ist zu groß.");
    }
    let recovery: Recovery =
        serde_json::from_slice(&bytes).context("Ungültige Wiederherstellung.")?;
    if recovery.ciphertext.len() > crate::vault::MAX_FILE * 2 {
        bail!("Die Wiederherstellung ist zu groß.");
    }
    Ok(recovery)
}

pub fn restart_locked(path: Option<&Path>, recovery: Option<&Recovery>, notice: Option<&str>) -> ! {
    let inherited = match recovery.map(recovery_file).transpose() {
        Ok(file) => file,
        Err(_) => {
            eprintln!("Die verschlüsselte Wiederherstellung konnte nicht übergeben werden. Die Sitzung wird aus Sicherheitsgründen beendet.");
            std::process::exit(1);
        }
    };
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => std::process::exit(1),
    };
    let mut command = Command::new(executable);
    if let Some(path) = path {
        command.arg("--vault").arg(path);
    }
    if let Some(notice) = notice {
        command.arg("--notice").arg(notice);
    }
    // Close every unrelated inherited GTK/network descriptor on exec, including
    // descriptors held by another thread. Keep only the ciphertext channel.
    unsafe {
        libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32); // CLOSE_RANGE_CLOEXEC
    }
    if let Some(file) = inherited.as_ref() {
        let fd = file.as_raw_fd();
        if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } < 0 {
            std::process::exit(1);
        }
        command.arg("--recovery-fd").arg(fd.to_string());
    }
    let error = command.exec();
    eprintln!(
        "Der gesperrte Prozess konnte nicht neu gestartet werden: {}",
        error.kind()
    );
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{encrypt, Field};
    #[test]
    fn anonymous_recovery_contains_ciphertext_and_requires_password() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("demo.db");
        std::fs::write(&path, encrypt(b"<database/>", "synthetic-only", 2).unwrap()).unwrap();
        let mut vault = Vault::open(&path, "synthetic-only").unwrap();
        vault
            .upsert(
                None,
                "Unsaved",
                &[Field {
                    name: "Password".into(),
                    kind: "password".into(),
                    value: "private-synthetic-value".into(),
                }],
                "",
            )
            .unwrap();
        let recovery = Recovery::capture(&vault, None).unwrap();
        let file = recovery_file(&recovery).unwrap();
        // Duplicate the fd: read_recovery takes ownership exactly once.
        let fd = unsafe { libc::dup(file.as_raw_fd()) };
        let restored = read_recovery(fd).unwrap();
        let json = serde_json::to_string(&restored).unwrap();
        assert!(!json.contains("synthetic-only"));
        assert!(!json.contains("private-synthetic-value"));
        assert!(restored.unlock(&path, "wrong").is_err());
        let mut recovered = restored.unlock(&path, "synthetic-only").unwrap();
        assert!(recovered.dirty);
        recovered.save().unwrap();
        assert_eq!(
            Vault::open(&path, "synthetic-only").unwrap().cards()[0].title,
            "Unsaved"
        );
    }
}
