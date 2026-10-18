//! Private app-folder discovery and conservative whole-ciphertext Drive sync.
use crate::oauth::{atomic_private, private_dir, read_private, xdg_dir, OAuth};
use anyhow::{anyhow, bail, Result};
use md5::Md5;
use reqwest::{
    blocking::{Client, Response},
    header::IF_MATCH,
    StatusCode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_JSON: usize = 4 * 1024 * 1024;
const DRIVE_ERROR: &str = "Google Drive connection failed. Local changes remain saved.";
const CONFLICT: &str = "The Drive file changed on another device. Local changes are saved; the cloud file was not overwritten.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct DriveSessionState {
    pub candidate: Candidate,
    pub etag: Option<String>,
    pub base_digest: String,
}
#[derive(Clone)]
pub struct Drive {
    oauth: OAuth,
    client: Client,
    directory: PathBuf,
    api: String,
    upload: String,
}
#[derive(Clone)]
pub struct DriveSession {
    drive: Drive,
    pub candidate: Candidate,
    pub path: PathBuf,
    pub etag: Option<String>,
    pub base_digest: [u8; 32],
}
#[derive(Serialize, Deserialize)]
struct SyncState {
    id: String,
    etag: Option<String>,
    base_digest: String,
}
#[derive(Deserialize)]
struct Source {
    id: String,
    #[serde(default, alias = "name")]
    path: String,
}
#[derive(Clone)]
struct Metadata {
    etag: String,
    checksum: String,
}

impl Drive {
    pub fn new() -> Result<Self> {
        let directory = xdg_dir("XDG_DATA_HOME", ".local/share")?.join("safeincloud-linux/vaults");
        let oauth = OAuth::new()?;
        Self::with_paths(oauth, directory)
    }
    fn with_paths(oauth: OAuth, directory: PathBuf) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| anyhow!(DRIVE_ERROR))?;
        Ok(Self {
            oauth,
            client,
            directory,
            api: "https://www.googleapis.com/drive".into(),
            upload: "https://www.googleapis.com/upload/drive".into(),
        })
    }
    pub fn configured(&self) -> bool {
        self.oauth.configured()
    }
    pub fn data_directory(&self) -> &Path {
        &self.directory
    }
    pub fn connect(&self) -> Result<()> {
        self.oauth.connect()
    }
    pub fn set_client_executable(&self, path: &Path) -> Result<()> {
        self.oauth.set_client_executable(path)
    }
    fn checked_response(response: reqwest::Result<Response>, limit: usize) -> Result<Vec<u8>> {
        let response = response.map_err(|_| anyhow!(DRIVE_ERROR))?;
        match response.status() {
            StatusCode::PRECONDITION_FAILED => bail!(CONFLICT),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => bail!("Google authorization for the private Safe app folder is required. Please reconnect."),
            status if !status.is_success() => bail!("Google Drive rejected the operation. Local changes remain saved."),
            _ => {},
        }
        if response.content_length().is_some_and(|n| n > limit as u64) {
            bail!("Google Drive file or response is too large.");
        }
        let mut bytes = Vec::new();
        response
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!(DRIVE_ERROR))?;
        if bytes.len() > limit {
            bail!("Google Drive file or response is too large.");
        }
        Ok(bytes)
    }
    fn get(
        &self,
        url: &str,
        params: &[(&str, &str)],
        limit: usize,
        etag: Option<&str>,
    ) -> Result<Vec<u8>> {
        let token = self.oauth.access_token()?;
        let mut request = self
            .client
            .get(url)
            .query(params)
            .bearer_auth(token.as_str());
        if let Some(etag) = etag {
            require_etag(etag)?;
            request = request.header(IF_MATCH, etag);
        }
        Self::checked_response(request.send(), limit)
    }
    pub fn discover(&self) -> Result<Vec<Candidate>> {
        let mut results = Vec::new();
        let mut pages = HashSet::new();
        let mut ids = HashSet::new();
        let mut page = String::new();
        loop {
            let mut params = vec![
                ("spaces", "appDataFolder"),
                ("q", "trashed = false"),
                ("fields", "nextPageToken,files(id,name,size)"),
                ("pageSize", "1000"),
            ];
            if !page.is_empty() {
                params.push(("pageToken", page.as_str()));
            }
            let raw = self.get(&format!("{}/v3/files", self.api), &params, MAX_JSON, None)?;
            let response: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|_| anyhow!("Could not read the Google Drive file list."))?;
            let rows = response
                .get("files")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow!("Could not read the Google Drive file list."))?;
            for row in rows {
                let Some(id) = row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|id| valid_id(id))
                else {
                    continue;
                };
                let Some(name) = row
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|n| n.len() <= 4096)
                else {
                    continue;
                };
                let Some(size) = row
                    .get("size")
                    .and_then(json_u64)
                    .filter(|n| *n <= MAX_FILE as u64)
                else {
                    continue;
                };
                let lower = name.to_lowercase();
                if size >= 2
                    && lower.starts_with("safeincloud")
                    && lower.ends_with(".db")
                    && ids.insert(id.to_owned())
                {
                    results.push(Candidate {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    });
                }
            }
            page = response
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if page.is_empty() {
                break;
            }
            if page.len() > 8192
                || !pages.insert(page.clone())
                || pages.len() > 1000
                || results.len() > 100_000
            {
                bail!("Google Drive returned an invalid or excessive file list.");
            }
        }
        results.sort_by_key(|candidate| candidate.name.to_lowercase());
        Ok(results)
    }
    pub fn preferred(&self, candidates: &[Candidate]) -> Option<Candidate> {
        if let Ok(raw) = read_private(&self.oauth.config_dir.join("source.json"), 128 * 1024) {
            if let Ok(source) = serde_json::from_slice::<Source>(&raw) {
                let matches: Vec<_> = candidates.iter().filter(|c| c.id == source.id).collect();
                if matches.len() == 1 {
                    return Some(matches[0].clone());
                }
            }
        }
        let canonical: Vec<_> = candidates
            .iter()
            .filter(|c| {
                c.name
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .eq_ignore_ascii_case("SafeInCloud.db")
            })
            .collect();
        if canonical.len() == 1 {
            return Some(canonical[0].clone());
        }
        if candidates.len() == 1 {
            return Some(candidates[0].clone());
        }
        None
    }
    fn remember(&self, candidate: &Candidate) -> Result<()> {
        atomic_private(
            &self.oauth.config_dir.join("source.json"),
            &serde_json::to_vec(&serde_json::json!({"id":candidate.id,"path":candidate.name}))?,
        )
    }
    fn stable_path(&self, id: &str) -> PathBuf {
        self.directory.join(format!(
            "vault-{}.db",
            &hex::encode(Sha256::digest(id.as_bytes()))[..24]
        ))
    }
    pub fn cached_session(&self) -> Result<Option<DriveSession>> {
        // Offline startup never contacts Google and preserves an older baseline for pending edits.
        let result = (|| -> Result<DriveSession> {
            let source: Source = serde_json::from_slice(&read_private(
                &self.oauth.config_dir.join("source.json"),
                128 * 1024,
            )?)?;
            if !valid_id(&source.id) {
                bail!("Invalid cached identity.");
            }
            let path = self.stable_path(&source.id);
            let state =
                read_state(&path, &source.id)?.ok_or_else(|| anyhow!("Missing cache baseline."))?;
            check_encrypted(&read_private(&path, MAX_FILE)?)?;
            self.restore_session(
                path,
                DriveSessionState {
                    candidate: Candidate {
                        id: source.id,
                        name: if source.path.is_empty() {
                            "SafeInCloud.db".into()
                        } else {
                            source.path
                        },
                    },
                    etag: state.etag,
                    base_digest: state.base_digest,
                },
            )
        })();
        Ok(result.ok())
    }
    pub fn restore_session(&self, path: PathBuf, state: DriveSessionState) -> Result<DriveSession> {
        if !valid_id(&state.candidate.id) || path != self.stable_path(&state.candidate.id) {
            bail!("Invalid Drive session identity.");
        }
        let base_digest = decode_digest(&state.base_digest)?;
        if state
            .etag
            .as_deref()
            .is_some_and(|etag| require_etag(etag).is_err())
        {
            bail!("Invalid cached Drive version.");
        }
        Ok(DriveSession {
            drive: self.clone(),
            candidate: state.candidate,
            path,
            etag: state.etag,
            base_digest,
        })
    }
    fn metadata(&self, id: &str) -> Result<Metadata> {
        require_id(id)?;
        let raw = self.get(
            &format!("{}/v2/files/{id}", self.api),
            &[
                ("supportsAllDrives", "true"),
                ("fields", "id,etag,md5Checksum,title,fileSize"),
            ],
            1024 * 1024,
            None,
        )?;
        let value: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|_| anyhow!("Invalid Google Drive metadata."))?;
        if value.get("id").and_then(|v| v.as_str()) != Some(id) {
            bail!("Google Drive returned a different file identity.");
        }
        let size = value
            .get("fileSize")
            .and_then(json_u64)
            .ok_or_else(|| anyhow!("Google Drive did not provide a safe file size."))?;
        if size > MAX_FILE as u64 || size < 2 {
            bail!("Google Drive database size is unsupported.");
        }
        let etag = value
            .get("etag")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Google Drive did not provide a safe file version."))?
            .to_owned();
        require_etag(&etag)?;
        let checksum = value
            .get("md5Checksum")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Google Drive did not provide a ciphertext checksum."))?
            .to_lowercase();
        if checksum.len() != 32 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Google Drive returned an invalid ciphertext checksum.");
        }
        Ok(Metadata { etag, checksum })
    }
    fn download(&self, candidate: &Candidate) -> Result<(Vec<u8>, Metadata)> {
        let before = self.metadata(&candidate.id)?;
        let data = self.get(
            &format!("{}/v3/files/{}", self.api, candidate.id),
            &[("alt", "media")],
            MAX_FILE,
            // Bind the media bytes with the before/after v2 ETag and checksum.
            // ETags from v2 metadata must not be assumed to identify a v3 media representation.
            None,
        )?;
        check_encrypted(&data)?;
        let after = self.metadata(&candidate.id)?;
        if before.etag != after.etag || md5(&data) != after.checksum {
            bail!("The Drive file changed during download. Please try again.");
        }
        Ok((data, after))
    }
    pub fn open_candidate(&self, candidate: Candidate) -> Result<DriveSession> {
        require_id(&candidate.id)?;
        private_dir(&self.directory)?;
        let path = self.stable_path(&candidate.id);
        let (data, metadata) = self.download(&candidate)?;
        let previous = read_state(&path, &candidate.id)?;
        let local = match read_private(&path, MAX_FILE) {
            Ok(data) => Some(data),
            Err(_) if !path.try_exists()? => None,
            Err(_) => bail!(
                "The existing encrypted cache is unreadable or unsafe. It was not overwritten."
            ),
        };
        if let Some(local) = local.as_ref() {
            match previous.as_ref() {
                Some(previous) if digest(local) != decode_digest(&previous.base_digest)? => {
                    // A freshly downloaded remote version must never erase offline edits.
                    check_encrypted(local)?;
                    self.remember(&candidate)?;
                    return self.restore_session(path, DriveSessionState { candidate, etag: previous.etag.clone(), base_digest: previous.base_digest.clone() });
                },
                None if local != &data => bail!("A local encrypted copy without a valid sync baseline exists. It was not overwritten."),
                _ => {},
            }
        }
        if local.as_deref() != Some(&data) {
            if local.is_some() {
                replace_encrypted(&path, &data)?;
            } else {
                atomic_private(&path, &data)?;
            }
        }
        let session = DriveSession {
            drive: self.clone(),
            candidate,
            path,
            etag: Some(metadata.etag),
            base_digest: digest(&data),
        };
        session.persist()?;
        self.remember(&session.candidate)?;
        Ok(session)
    }
    fn upload(&self, id: &str, etag: &str, bytes: &[u8]) -> Result<()> {
        require_id(id)?;
        require_etag(etag)?;
        check_encrypted(bytes)?;
        let token = self.oauth.access_token()?;
        let request = self
            .client
            .put(format!("{}/v2/files/{id}", self.upload))
            .query(&[
                ("uploadType", "media"),
                ("supportsAllDrives", "true"),
                ("newRevision", "true"),
            ])
            .bearer_auth(token.as_str())
            .header(IF_MATCH, etag)
            .header("Content-Type", "application/octet-stream")
            .body(bytes.to_owned());
        Self::checked_response(request.send(), 1024 * 1024)?;
        Ok(())
    }
}

impl DriveSession {
    pub fn state(&self) -> DriveSessionState {
        DriveSessionState {
            candidate: self.candidate.clone(),
            etag: self.etag.clone(),
            base_digest: hex::encode(self.base_digest),
        }
    }
    fn persist(&self) -> Result<()> {
        atomic_private(
            &self.path.with_extension("sync.json"),
            &serde_json::to_vec(&SyncState {
                id: self.candidate.id.clone(),
                etag: self.etag.clone(),
                base_digest: hex::encode(self.base_digest),
            })?,
        )
    }
    pub fn sync(&mut self, vault: &mut crate::vault::Vault) -> Result<String> {
        // Save first: any network error, denied write or concurrent remote edit leaves ciphertext recoverable.
        vault.save()?;
        let local = read_private(&self.path, MAX_FILE)
            .map_err(|_| anyhow!("Could not read the local encrypted database."))?;
        check_encrypted(&local)?;
        let latest = self.drive.metadata(&self.candidate.id)?;
        if Some(latest.etag.as_str()) != self.etag.as_deref() {
            if latest.checksum == md5(&local) {
                // Recover from termination after successful upload but before baseline commit.
                self.etag = Some(latest.etag);
                self.base_digest = digest(&local);
                self.persist()?;
                return Ok("Drive version confirmed. Encrypted copies agree.".into());
            }
            if digest(&local) != self.base_digest {
                bail!(CONFLICT);
            }
            let (fresh, metadata) = self.drive.download(&self.candidate)?;
            vault.validate_encrypted(&fresh)?;
            if digest(&read_private(&self.path, MAX_FILE)?) != digest(&local) {
                bail!("The local encrypted database changed during synchronization. It was not overwritten.");
            }
            replace_encrypted(&self.path, &fresh)?;
            vault.replace_from_encrypted(&fresh)?;
            self.etag = Some(metadata.etag);
            self.base_digest = digest(&fresh);
            self.persist()?;
            return Ok(
                "Newer Drive version loaded. The previous encrypted copy remains as a backup."
                    .into(),
            );
        }
        if digest(&local) == self.base_digest {
            return Ok("No local changes. Drive version is unchanged.".into());
        }
        let etag = self
            .etag
            .as_deref()
            .ok_or_else(|| anyhow!("Cloud saving requires a safe Drive file version."))?;
        require_etag(etag)?;
        // Persist a separate recovery copy before the conditional network mutation.
        let mut pending = tempfile::Builder::new()
            .prefix("pending-upload-")
            .suffix(".db")
            .tempfile_in(
                self.path
                    .parent()
                    .ok_or_else(|| anyhow!("Invalid cache directory."))?,
            )?;
        pending.write_all(&local)?;
        pending.as_file().sync_all()?;
        let (_file, recovery_path) = pending
            .keep()
            .map_err(|_| anyhow!("Could not preserve encrypted upload recovery copy."))?;
        self.drive.upload(&self.candidate.id, etag, &local)?;
        let confirmed = self.drive.metadata(&self.candidate.id)?;
        if confirmed.checksum != md5(&local) {
            bail!("The uploaded Drive version could not be confirmed. The encrypted recovery copy remains saved.");
        }
        self.etag = Some(confirmed.etag);
        self.base_digest = digest(&local);
        self.persist()?;
        fs::remove_file(recovery_path)?;
        Ok("Encrypted database synchronized with Google Drive.".into())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 1024
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn require_id(id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("Invalid Google Drive file ID.");
    }
    Ok(())
}
fn require_etag(etag: &str) -> Result<()> {
    if etag.len() < 2
        || etag.len() > 2048
        || !etag.starts_with('"')
        || !etag.ends_with('"')
        || etag[1..etag.len() - 1]
            .bytes()
            .any(|b| b < 0x21 || b == 0x22 || b == 0x7f)
    {
        bail!("Cloud saving requires a strong Drive file version.");
    }
    Ok(())
}
fn check_encrypted(data: &[u8]) -> Result<()> {
    if data.len() > MAX_FILE || !data.starts_with(&[5, 5]) {
        bail!("Not a supported encrypted SafeInCloud database.");
    }
    Ok(())
}
fn digest(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}
fn md5(data: &[u8]) -> String {
    hex::encode(Md5::digest(data))
}
fn decode_digest(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("Invalid local sync baseline.");
    }
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow!("Invalid local sync baseline."))
}
fn json_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}
fn read_state(path: &Path, id: &str) -> Result<Option<SyncState>> {
    let state = path.with_extension("sync.json");
    if !state.try_exists()? {
        return Ok(None);
    }
    let parsed = (|| -> Result<SyncState> {
        let value: SyncState = serde_json::from_slice(&read_private(&state, 128 * 1024)?)?;
        if value.id != id {
            bail!("Mismatched cached identity.");
        }
        decode_digest(&value.base_digest)?;
        if value
            .etag
            .as_deref()
            .is_some_and(|tag| require_etag(tag).is_err())
        {
            bail!("Invalid cached version.");
        }
        Ok(value)
    })();
    // An invalid state is equivalent to no baseline; open_candidate refuses to overwrite a differing local copy.
    Ok(parsed.ok())
}
fn replace_encrypted(path: &Path, bytes: &[u8]) -> Result<()> {
    let old = read_private(path, MAX_FILE)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Invalid encrypted cache path."))?;
    let mut backup = tempfile::Builder::new()
        .prefix("vault-backup-")
        .suffix(".db")
        .tempfile_in(parent)?;
    backup.write_all(&old)?;
    backup.as_file().sync_all()?;
    backup
        .keep()
        .map_err(|_| anyhow!("Could not retain the encrypted backup."))?;
    atomic_private(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{encrypt, Vault};
    use std::{
        net::TcpListener,
        os::unix::fs::PermissionsExt,
        thread::{self, JoinHandle},
        time::Instant,
    };

    fn candidate() -> Candidate {
        Candidate {
            id: "synthetic-id".into(),
            name: "SafeInCloud.db".into(),
        }
    }
    fn setup() -> (tempfile::TempDir, Drive) {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config");
        private_dir(&config).unwrap();
        atomic_private(
            &config.join("official-google-token.json"),
            br#"{"access_token":"synthetic-access","expires_at":9999999999}"#,
        )
        .unwrap();
        let drive = Drive::with_paths(
            OAuth::with_config(config).unwrap(),
            directory.path().join("vaults"),
        )
        .unwrap();
        private_dir(drive.data_directory()).unwrap();
        (directory, drive)
    }
    fn metadata(tag: &str, data: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"id":"synthetic-id","etag":tag,"md5Checksum":md5(data),"fileSize":data.len().to_string()})).unwrap()
    }
    fn serve(mut drive: Drive, replies: Vec<(u16, Vec<u8>)>) -> (Drive, JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        drive.api = format!("http://{}", listener.local_addr().unwrap());
        drive.upload = drive.api.clone();
        let thread = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            for (status, body) in replies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("mock request not received: {e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 8192];
                let mut expected = None;
                loop {
                    let count = stream.read(&mut chunk).unwrap();
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(pos) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..pos]);
                        let size = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        expected = Some(pos + 4 + size);
                    }
                    if expected.is_some_and(|n| bytes.len() >= n) {
                        break;
                    }
                }
                requests.push(bytes);
                write!(
                    stream,
                    "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (drive, thread)
    }
    fn session(drive: &Drive, bytes: &[u8]) -> DriveSession {
        let path = drive.stable_path(&candidate().id);
        atomic_private(&path, bytes).unwrap();
        let session = DriveSession {
            drive: drive.clone(),
            candidate: candidate(),
            path,
            etag: Some("\"base\"".into()),
            base_digest: digest(bytes),
        };
        session.persist().unwrap();
        drive.remember(&candidate()).unwrap();
        session
    }
    fn encrypted(title: &str) -> Vec<u8> {
        encrypt(
            format!("<database><card id=\"1\" title=\"{title}\"/></database>").as_bytes(),
            "synthetic-password",
            2,
        )
        .unwrap()
    }

    #[test]
    fn offline_cache_preserves_pending_edits_and_prior_baseline() {
        let (_dir, drive) = setup();
        let original = b"\x05\x05original ciphertext";
        let existing = session(&drive, original);
        atomic_private(&existing.path, b"\x05\x05pending ciphertext").unwrap();
        let cached = drive.cached_session().unwrap().unwrap();
        assert_eq!(cached.base_digest, digest(original));
        assert_eq!(cached.etag.as_deref(), Some("\"base\""));
        assert_eq!(
            read_private(&cached.path, MAX_FILE).unwrap(),
            b"\x05\x05pending ciphertext"
        );
        assert!(drive
            .restore_session(PathBuf::from("/arbitrary.db"), cached.state())
            .is_err());
    }
    #[test]
    fn invalid_baseline_and_public_cache_are_not_loaded() {
        let (_dir, drive) = setup();
        let existing = session(&drive, b"\x05\x05ciphertext");
        atomic_private(
            &existing.path.with_extension("sync.json"),
            br#"{"id":"other","etag":"\"base\"","base_digest":"0000"}"#,
        )
        .unwrap();
        assert!(drive.cached_session().unwrap().is_none());
        existing.persist().unwrap();
        fs::set_permissions(&existing.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(drive.cached_session().unwrap().is_none());
    }
    #[test]
    fn discovery_uses_private_folder_and_prefers_unambiguous_name() {
        let (_dir, drive) = setup();
        let listing = serde_json::json!({"files":[{"id":"synthetic-id","name":"SafeInCloud.db","size":"100"},{"id":"other","name":"unrelated.db","size":"100"},{"id":"too-big","name":"SafeInCloud-other.db","size":(MAX_FILE+1).to_string()}]});
        let (drive, server) = serve(drive, vec![(200, serde_json::to_vec(&listing).unwrap())]);
        let found = drive.discover().unwrap();
        assert_eq!(found, [candidate()]);
        assert_eq!(drive.preferred(&found).unwrap(), candidate());
        let request = String::from_utf8(server.join().unwrap().remove(0)).unwrap();
        assert!(request.contains("spaces=appDataFolder"));
        assert!(drive
            .preferred(&[
                Candidate {
                    id: "a".into(),
                    name: "SafeInCloud-A.db".into()
                },
                Candidate {
                    id: "b".into(),
                    name: "SafeInCloud-B.db".into()
                }
            ])
            .is_none());
    }
    #[test]
    fn download_race_cannot_replace_cache() {
        let (_dir, drive) = setup();
        let existing = session(&drive, b"\x05\x05original");
        let remote = b"\x05\x05downloaded";
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"before\"", remote)),
                (200, remote.to_vec()),
                (200, metadata("\"after\"", remote)),
            ],
        );
        assert!(drive.open_candidate(candidate()).is_err());
        assert_eq!(
            read_private(&existing.path, MAX_FILE).unwrap(),
            b"\x05\x05original"
        );
        assert_eq!(server.join().unwrap().len(), 3);
    }
    #[test]
    fn new_remote_download_never_discards_offline_edits() {
        let (_dir, drive) = setup();
        let existing = session(&drive, b"\x05\x05original");
        atomic_private(&existing.path, b"\x05\x05offline edits").unwrap();
        let remote = b"\x05\x05remote edits";
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"remote\"", remote)),
                (200, remote.to_vec()),
                (200, metadata("\"remote\"", remote)),
            ],
        );
        let reopened = drive.open_candidate(candidate()).unwrap();
        assert_eq!(reopened.base_digest, existing.base_digest);
        assert_eq!(reopened.etag, existing.etag);
        assert_eq!(
            read_private(&reopened.path, MAX_FILE).unwrap(),
            b"\x05\x05offline edits"
        );
        server.join().unwrap();
    }
    #[test]
    fn conditional_upload_412_retains_local_ciphertext_and_recovery() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"base\"", &original)),
                (412, b"synthetic provider payload must not escape".to_vec()),
            ],
        );
        let mut session = session(&drive, &original);
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        vault.set_favorite("1", true).unwrap();
        let error = session.sync(&mut vault).unwrap_err().to_string();
        assert!(error.contains("not overwritten"));
        assert!(!error.contains("provider payload"));
        assert!(!vault.dirty);
        let local = read_private(&session.path, MAX_FILE).unwrap();
        assert_ne!(local, original);
        assert_eq!(session.base_digest, digest(&original));
        assert!(fs::read_dir(drive.data_directory())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("pending-upload-")));
        let requests = server.join().unwrap();
        let upload = String::from_utf8_lossy(&requests[1]);
        assert!(upload.starts_with("PUT /v2/files/synthetic-id?"));
        assert!(upload.to_ascii_lowercase().contains("if-match: \"base\""));
        assert!(requests[1].ends_with(&local));
    }
    #[test]
    fn newer_remote_pulls_only_when_local_is_clean_and_validated() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let remote = encrypted("Remote");
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"remote\"", &remote)),
                (200, metadata("\"remote\"", &remote)),
                (200, remote.clone()),
                (200, metadata("\"remote\"", &remote)),
            ],
        );
        let mut session = session(&drive, &original);
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        session.sync(&mut vault).unwrap();
        assert_eq!(vault.cards()[0].title, "Remote");
        assert_eq!(read_private(&session.path, MAX_FILE).unwrap(), remote);
        assert_eq!(session.base_digest, digest(&remote));
        assert!(fs::read_dir(drive.data_directory())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("vault-backup-")));
        server.join().unwrap();
    }
    #[test]
    fn wrong_password_remote_is_never_installed() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let remote = encrypt(b"<database/>", "other-password", 2).unwrap();
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"remote\"", &remote)),
                (200, metadata("\"remote\"", &remote)),
                (200, remote.clone()),
                (200, metadata("\"remote\"", &remote)),
            ],
        );
        let mut session = session(&drive, &original);
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        assert!(session.sync(&mut vault).is_err());
        assert_eq!(read_private(&session.path, MAX_FILE).unwrap(), original);
        assert_eq!(vault.cards()[0].title, "Original");
        assert_eq!(session.etag.as_deref(), Some("\"base\""));
        server.join().unwrap();
    }
    #[test]
    fn successful_upload_commits_only_confirmed_ciphertext_baseline() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let mut session = session(&drive, &original);
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        vault.set_favorite("1", true).unwrap();
        vault.save().unwrap();
        let local = read_private(&session.path, MAX_FILE).unwrap();
        let (drive, server) = serve(
            drive,
            vec![
                (200, metadata("\"base\"", &original)),
                (200, b"{}".to_vec()),
                (200, metadata("\"uploaded\"", &local)),
            ],
        );
        session.drive = drive.clone();
        session.sync(&mut vault).unwrap();
        assert_eq!(session.etag.as_deref(), Some("\"uploaded\""));
        assert_eq!(session.base_digest, digest(&local));
        assert_eq!(
            drive.cached_session().unwrap().unwrap().base_digest,
            digest(&local)
        );
        assert!(!fs::read_dir(drive.data_directory())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("pending-upload-")));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].ends_with(&local));
    }
    #[test]
    fn interrupted_upload_adopts_matching_remote_without_reupload() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let mut session = session(&drive, &original);
        let local = encrypted("Already uploaded");
        atomic_private(&session.path, &local).unwrap();
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        let (drive, server) = serve(
            drive,
            vec![(200, metadata("\"uploaded-before-lock\"", &local))],
        );
        session.drive = drive;
        session.sync(&mut vault).unwrap();
        assert_eq!(session.base_digest, digest(&local));
        assert_eq!(session.etag.as_deref(), Some("\"uploaded-before-lock\""));
        assert_eq!(server.join().unwrap().len(), 1);
    }
    #[test]
    fn concurrent_remote_and_local_changes_refuse_download_and_upload() {
        let (_dir, drive) = setup();
        let original = encrypted("Original");
        let remote = encrypted("Remote edits");
        let (drive, server) = serve(drive, vec![(200, metadata("\"remote-edits\"", &remote))]);
        let mut session = session(&drive, &original);
        let mut vault = Vault::open(&session.path, "synthetic-password").unwrap();
        vault.set_favorite("1", true).unwrap();
        assert!(session.sync(&mut vault).is_err());
        assert!(!vault.dirty);
        assert_eq!(vault.cards()[0].title, "Original");
        assert!(vault.cards()[0].favorite);
        assert_eq!(session.base_digest, digest(&original));
        assert_ne!(read_private(&session.path, MAX_FILE).unwrap(), remote);
        assert_eq!(server.join().unwrap().len(), 1);
    }
    #[test]
    fn strong_etag_and_ciphertext_bounds_are_required() {
        for tag in ["", "*", "W/\"weak\"", "unquoted", "\"injected\r\nheader\""] {
            assert!(require_etag(tag).is_err());
        }
        assert!(require_etag("\"strong\"").is_ok());
        assert!(check_encrypted(b"plaintext").is_err());
        assert!(
            Drive::checked_response(reqwest::blocking::get("http://127.0.0.1:1"), MAX_JSON)
                .is_err()
        );
    }
}
