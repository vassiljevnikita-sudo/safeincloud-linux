//! User-authorized installed-client OAuth, PKCE, and private token persistence.
use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

pub const APPDATA_SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
const MAX_JSON: usize = 128 * 1024;
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const AUTH_ERROR: &str = "Google authorization or token refresh failed. Please reconnect.";

#[derive(Clone)]
pub struct OAuth {
    pub(crate) config_dir: PathBuf,
    client: Client,
    lock: Arc<Mutex<()>>,
    token_endpoint: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Tokens {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    expires_at: f64,
}
impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

pub fn config_dir() -> Result<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|p| p.join("safeincloud-linux"))
}
pub(crate) fn xdg_dir(var: &str, fallback: &str) -> Result<PathBuf> {
    if let Some(path) = env::var_os(var).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home = env::var_os("HOME").ok_or_else(|| anyhow!("Home directory is unavailable."))?;
    Ok(PathBuf::from(home).join(fallback))
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|_| anyhow!("Could not create private application directory."))?;
    let info = fs::symlink_metadata(path).map_err(|_| anyhow!("Unsafe application directory."))?;
    if !info.is_dir() || info.uid() != unsafe { libc::geteuid() } {
        bail!("Unsafe application directory.");
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| anyhow!("Could not protect application directory."))?;
    Ok(())
}

pub(crate) fn read_private(path: &Path, max: usize) -> Result<Vec<u8>> {
    read_owned(path, max, true)
}

fn read_owned(path: &Path, max: usize, private: bool) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let info = file.metadata()?;
    if !info.is_file()
        || info.uid() != unsafe { libc::geteuid() }
        || info.mode() & (if private { 0o077 } else { 0o022 }) != 0
        || info.len() > max as u64
    {
        bail!("Unsafe private file.");
    }
    let mut bytes = Vec::new();
    (&mut file).take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        bail!("Private file too large.");
    }
    Ok(bytes)
}

pub(crate) fn atomic_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Invalid local storage path."))?;
    private_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|_| anyhow!("Could not persist private application data."))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

impl OAuth {
    pub fn new() -> Result<Self> {
        Self::with_config(config_dir()?)
    }
    pub(crate) fn with_config(config_dir: PathBuf) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| anyhow!("Could not initialize Google authorization."))?;
        Ok(Self {
            config_dir,
            client,
            lock: Arc::new(Mutex::new(())),
            token_endpoint: TOKEN_ENDPOINT.into(),
        })
    }
    fn tokens(&self) -> Result<Tokens> {
        let path = self.config_dir.join("official-google-token.json");
        if !path
            .try_exists()
            .map_err(|_| anyhow!("Local authorization data is unreadable."))?
        {
            return Ok(Tokens::default());
        }
        let raw = Zeroizing::new(read_private(&path, MAX_JSON).map_err(|_| {
            anyhow!("Local authorization data is unreadable or has unsafe permissions.")
        })?);
        serde_json::from_slice(&raw).map_err(|_| {
            anyhow!("Local authorization data is unreadable or has unsafe permissions.")
        })
    }
    pub fn configured(&self) -> bool {
        self.tokens()
            .map(|t| !t.refresh_token.is_empty() || !t.access_token.is_empty())
            .unwrap_or(false)
    }
    pub fn set_client_executable(&self, path: &Path) -> Result<()> {
        let path = path
            .canonicalize()
            .map_err(|_| anyhow!("Select the supplied official Windows executable first."))?;
        crate::dotnet::extract_client(&path)?;
        atomic_private(
            &self.config_dir.join("official-client.json"),
            &serde_json::to_vec(&serde_json::json!({"path": path}))?,
        )
        .map_err(|_| anyhow!("Could not save the selected desktop client path."))
    }
    fn secrets(&self) -> Result<crate::dotnet::ClientSecrets> {
        // This file contains a path, never client parameters or account credentials.
        let bytes = read_owned(
            &self.config_dir.join("official-client.json"),
            MAX_JSON,
            false,
        )
        .map_err(|_| anyhow!("Select the supplied official Windows executable first."))?;
        #[derive(Deserialize)]
        struct Config {
            path: PathBuf,
        }
        let config: Config = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("Select the supplied official Windows executable first."))?;
        if !config.path.is_absolute() {
            bail!("Select the supplied official Windows executable first.");
        }
        crate::dotnet::extract_client(&config.path)
    }
    fn exchange(&self, fields: &[(&str, &str)]) -> Result<serde_json::Value> {
        let response = self
            .client
            .post(&self.token_endpoint)
            .form(fields)
            .send()
            .map_err(|_| anyhow!(AUTH_ERROR))?;
        if !response.status().is_success() {
            bail!(AUTH_ERROR);
        }
        let mut raw = Zeroizing::new(Vec::new());
        response
            .take(MAX_JSON as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| anyhow!(AUTH_ERROR))?;
        if raw.len() > MAX_JSON {
            bail!(AUTH_ERROR);
        }
        let result: serde_json::Value =
            serde_json::from_slice(&raw).map_err(|_| anyhow!(AUTH_ERROR))?;
        if result
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .is_none()
        {
            bail!(AUTH_ERROR);
        }
        Ok(result)
    }
    fn save(
        &self,
        mut response: serde_json::Value,
        mut previous: Tokens,
        requested_scope: Option<&str>,
    ) -> Result<Tokens> {
        let expiry = match response.get("expires_in") {
            Some(value) => value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| anyhow!("Invalid Google authorization response."))?,
            None => 3600.0,
        };
        if !expiry.is_finite() || expiry <= 0.0 || expiry > 365.0 * 86400.0 {
            bail!("Invalid Google authorization response.");
        }
        for (key, slot) in [
            ("access_token", &mut previous.access_token),
            ("refresh_token", &mut previous.refresh_token),
            ("scope", &mut previous.scope),
            ("token_type", &mut previous.token_type),
        ] {
            if let Some(serde_json::Value::String(value)) = response.get_mut(key) {
                if !value.is_empty() {
                    slot.zeroize();
                    *slot = std::mem::take(value);
                }
            }
        }
        if previous.scope.is_empty() {
            previous.scope = requested_scope.unwrap_or("").to_owned();
        }
        previous.expires_at = unix_now() + expiry;
        let encoded = Zeroizing::new(
            serde_json::to_vec(&previous).map_err(|_| anyhow!("Could not save authorization."))?,
        );
        atomic_private(
            &self.config_dir.join("official-google-token.json"),
            &encoded,
        )
        .map_err(|_| anyhow!("Could not save authorization securely."))?;
        Ok(previous)
    }
    pub fn access_token(&self) -> Result<Zeroizing<String>> {
        let _guard = self.lock.lock().map_err(|_| anyhow!(AUTH_ERROR))?;
        let mut tokens = self.tokens()?;
        if tokens.expires_at > unix_now() + 60.0 && !tokens.access_token.is_empty() {
            return Ok(Zeroizing::new(std::mem::take(&mut tokens.access_token)));
        }
        if tokens.refresh_token.is_empty() {
            bail!("Connect Google Drive first.");
        }
        let secrets = self.secrets()?;
        let response = self.exchange(&[
            ("client_id", &secrets.client_id),
            ("client_secret", &secrets.client_secret),
            ("refresh_token", &tokens.refresh_token),
            ("grant_type", "refresh_token"),
        ])?;
        let mut tokens = self.save(response, tokens, None)?;
        Ok(Zeroizing::new(std::mem::take(&mut tokens.access_token)))
    }
    pub fn connect(&self) -> Result<()> {
        let _guard = self.lock.lock().map_err(|_| anyhow!(AUTH_ERROR))?;
        let secrets = self.secrets()?;
        self.connect_with(&secrets, |url| {
            webbrowser::open(url)
                .map_err(|_| anyhow!("Could not open the system browser for authorization."))
        })
    }
    fn connect_with(
        &self,
        secrets: &crate::dotnet::ClientSecrets,
        open_browser: impl FnOnce(&str) -> Result<()>,
    ) -> Result<()> {
        let state = random_url(32);
        let verifier = Zeroizing::new(random_url(64));
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| anyhow!("Could not open the local authorization callback."))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| anyhow!("Could not open the local authorization callback."))?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener
                .local_addr()
                .map_err(|_| anyhow!("Local authorization unavailable."))?
                .port()
        );
        let mut url = Url::parse(AUTH_ENDPOINT).expect("constant URL");
        url.query_pairs_mut().extend_pairs([
            ("client_id", secrets.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", APPDATA_SCOPE),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ]);
        open_browser(url.as_str())?;
        let deadline = Instant::now() + Duration::from_secs(300);
        let code = loop {
            if Instant::now() >= deadline {
                bail!("Google authorization timed out. Please reconnect.");
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(result) = callback(&mut stream, &state) {
                        break result?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50))
                }
                Err(_) => bail!("Local Google authorization failed. Please reconnect."),
            }
        };
        let response = self.exchange(&[
            ("client_id", &secrets.client_id),
            ("client_secret", &secrets.client_secret),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
        ])?;
        if response
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .is_none()
        {
            bail!("Google did not return a refresh token. Please reconnect with consent.");
        }
        self.save(response, Tokens::default(), Some(APPDATA_SCOPE))?;
        Ok(())
    }
}
fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
fn random_url(size: usize) -> String {
    let mut bytes = Zeroizing::new(vec![0u8; size]);
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes.as_slice())
}
fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |sum, (a, b)| sum | (a ^ b))
        == 0
}

fn callback(stream: &mut TcpStream, state: &str) -> Option<Result<Zeroizing<String>>> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut request = Zeroizing::new(Vec::new());
    let mut chunk = [0u8; 1024];
    while request.len() <= 16 * 1024 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    }
    chunk.zeroize();
    let parsed = (|| {
        if request.len() > 16 * 1024 {
            return None;
        }
        let line = std::str::from_utf8(&request).ok()?.lines().next()?;
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() != 3
            || parts[0] != "GET"
            || !parts[1].starts_with("/callback?")
            || !parts[2].starts_with("HTTP/1.")
        {
            return None;
        }
        let url = Url::parse(&format!("http://127.0.0.1{}", parts[1])).ok()?;
        if url.path() != "/callback" {
            return None;
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        if pairs.len() > 12 {
            return None;
        }
        let states: Vec<_> = pairs.iter().filter(|(key, _)| key == "state").collect();
        if states.len() != 1 || !constant_time_equal(states[0].1.as_bytes(), state.as_bytes()) {
            return None;
        }
        if pairs.iter().any(|(key, _)| key == "error") {
            return Some(Err(anyhow!("Google authorization was declined.")));
        }
        let codes: Vec<_> = pairs.iter().filter(|(key, _)| key == "code").collect();
        if codes.len() != 1 || codes[0].1.is_empty() || codes[0].1.len() >= 8192 {
            return None;
        }
        Some(Ok(Zeroizing::new(codes[0].1.to_string())))
    })();
    let (status, body) = if parsed.is_some() {
        (
            "200 OK",
            "Authorization received. You can close this tab and return to SafeInCloud Linux.",
        )
    } else {
        ("400 Bad Request", "Invalid authorization callback.")
    };
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use std::collections::HashMap;
    fn token_server(
        response: &'static [u8],
    ) -> (String, thread::JoinHandle<HashMap<String, String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let thread = thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let (start, size) = loop {
                let n = stream.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..pos]);
                    let size: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse().ok())
                        })
                        .unwrap();
                    if bytes.len() >= pos + 4 + size {
                        break (pos + 4, size);
                    }
                }
            };
            let fields = url::form_urlencoded::parse(&bytes[start..start + size])
                .into_owned()
                .collect();
            write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", response.len()).unwrap();
            stream.write_all(response).unwrap();
            fields
        });
        (endpoint, thread)
    }
    fn install_synthetic_client(oauth: &OAuth) {
        private_dir(&oauth.config_dir).unwrap();
        let exe = oauth.config_dir.join("synthetic.exe");
        fs::write(&exe, crate::dotnet::tests::synthetic_assembly()).unwrap();
        oauth.set_client_executable(&exe).unwrap();
    }
    #[test]
    fn refresh_existing_tokens_reads_supplied_assembly_and_preserves_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut oauth = OAuth::with_config(dir.path().join("config")).unwrap();
        install_synthetic_client(&oauth);
        let mut tokens = oauth.save(serde_json::json!({"access_token":"expired", "refresh_token":"synthetic-refresh", "expires_in":1}), Tokens::default(), Some(APPDATA_SCOPE)).unwrap();
        tokens.expires_at = 0.0;
        atomic_private(
            &oauth.config_dir.join("official-google-token.json"),
            &serde_json::to_vec(&tokens).unwrap(),
        )
        .unwrap();
        let (endpoint, server) =
            token_server(br#"{"access_token":"refreshed-access","expires_in":3600}"#);
        oauth.token_endpoint = endpoint;
        assert_eq!(oauth.access_token().unwrap().as_str(), "refreshed-access");
        let fields = server.join().unwrap();
        assert_eq!(fields["grant_type"], "refresh_token");
        assert_eq!(fields["refresh_token"], "synthetic-refresh");
        assert_eq!(fields["client_id"], "synthetic.apps.googleusercontent.com");
        assert_eq!(fields["client_secret"], "synthetic-client-secret");
        assert_eq!(oauth.tokens().unwrap().refresh_token, "synthetic-refresh");
        let stored =
            fs::read_to_string(oauth.config_dir.join("official-google-token.json")).unwrap();
        assert!(!stored.contains("synthetic-client-secret"));
        assert!(!stored.contains("googleusercontent"));
    }
    #[test]
    fn browser_login_checks_pkce_and_bad_state_before_exchanging_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut oauth = OAuth::with_config(dir.path().join("config")).unwrap();
        install_synthetic_client(&oauth);
        let (endpoint, server) = token_server(br#"{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":3600}"#);
        oauth.token_endpoint = endpoint;
        let secrets = oauth.secrets().unwrap();
        let captured = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let mut browser_thread = None;
        oauth
            .connect_with(&secrets, |url| {
                let query: HashMap<String, String> = Url::parse(url)
                    .unwrap()
                    .query_pairs()
                    .into_owned()
                    .collect();
                assert_eq!(query["scope"], APPDATA_SCOPE);
                assert_eq!(query["code_challenge_method"], "S256");
                assert!(!url.contains("synthetic-client-secret"));
                *captured.lock().unwrap() = query.clone();
                browser_thread = Some(thread::spawn(move || {
                    let redirect = &query["redirect_uri"];
                    assert!(redirect.starts_with("http://127.0.0.1:"));
                    assert_eq!(
                        reqwest::blocking::get(format!(
                            "{redirect}?code=synthetic-code&state=wrong"
                        ))
                        .unwrap()
                        .status(),
                        StatusCode::BAD_REQUEST
                    );
                    let mut correct = Url::parse(redirect).unwrap();
                    correct.query_pairs_mut().extend_pairs([
                        ("code", "synthetic-code"),
                        ("state", query["state"].as_str()),
                    ]);
                    assert_eq!(
                        reqwest::blocking::get(correct).unwrap().status(),
                        StatusCode::OK
                    );
                }));
                Ok(())
            })
            .unwrap();
        browser_thread.unwrap().join().unwrap();
        let fields = server.join().unwrap();
        let query = captured.lock().unwrap();
        assert_eq!(fields["grant_type"], "authorization_code");
        assert_eq!(fields["code"], "synthetic-code");
        assert_eq!(fields["redirect_uri"], query["redirect_uri"]);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(fields["code_verifier"].as_bytes())),
            query["code_challenge"]
        );
        assert_eq!(oauth.tokens().unwrap().refresh_token, "synthetic-refresh");
    }
    #[test]
    fn private_tokens_preserve_refresh_and_reject_public_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let oauth = OAuth::with_config(dir.path().join("config")).unwrap();
        assert!(!oauth.configured());
        let tokens = oauth.save(serde_json::json!({"access_token":"fake-access", "refresh_token":"fake-refresh", "expires_in":3600}), Tokens::default(), Some(APPDATA_SCOPE)).unwrap();
        oauth
            .save(
                serde_json::json!({"access_token":"replacement"}),
                tokens,
                None,
            )
            .unwrap();
        assert_eq!(oauth.tokens().unwrap().refresh_token, "fake-refresh");
        assert_eq!(oauth.access_token().unwrap().as_str(), "replacement");
        let path = oauth.config_dir.join("official-google-token.json");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!oauth.configured());
    }
    #[test]
    fn reject_token_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let oauth = OAuth::with_config(dir.path().to_owned()).unwrap();
        fs::write(dir.path().join("other"), b"{}").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("other"),
            dir.path().join("official-google-token.json"),
        )
        .unwrap();
        assert!(oauth.tokens().is_err());
    }
    #[test]
    fn callback_rejects_wrong_state_and_duplicate_code() {
        for query in [
            "state=wrong&code=fake",
            "state=expected&code=a&code=b",
            "state=expected&state=expected&code=a",
        ] {
            assert!(callback_test(query).is_none());
        }
        assert_eq!(
            callback_test("state=expected&code=fake")
                .unwrap()
                .unwrap()
                .as_str(),
            "fake"
        );
        assert!(callback_test("state=expected&error=access_denied")
            .unwrap()
            .is_err());
    }
    fn callback_test(query: &str) -> Option<Result<Zeroizing<String>>> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let request = format!("GET /callback?{query} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        let thread = thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            client.write_all(request.as_bytes()).unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).unwrap();
        });
        let mut stream = listener.accept().unwrap().0;
        let result = callback(&mut stream, "expected");
        drop(stream);
        thread.join().unwrap();
        result
    }
}
