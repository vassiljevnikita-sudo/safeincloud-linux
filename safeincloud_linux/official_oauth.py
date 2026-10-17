"""User-authorized local interoperability with a supplied official desktop client.

Client configuration is read from the original binary into memory. User OAuth
refresh tokens are kept separately with private filesystem permissions. No
bundled client configuration, Windows token import, or silent authorization.
"""
import base64
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import secrets
import stat
import tempfile
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import Request, urlopen
import webbrowser

AUTH_ENDPOINT = 'https://accounts.google.com/o/oauth2/v2/auth'
TOKEN_ENDPOINT = 'https://oauth2.googleapis.com/token'
APPDATA_SCOPE = 'https://www.googleapis.com/auth/drive.appdata'
ALLOWED_SCOPES = {APPDATA_SCOPE, 'https://www.googleapis.com/auth/drive.readonly', 'https://www.googleapis.com/auth/drive'}
MAX_JSON = 128 * 1024


class OAuthError(RuntimeError):
    """Sanitized local authorization failure; never contains provider payloads."""


def extract_client(path):
    """Extract only ClientSecrets initialization, never user token material."""
    try:
        import dnfile
        from dncil.cil.body import CilMethodBody
        from dncil.cil.body.reader import CilMethodBodyReaderBytes
        pe = dnfile.dnPE(str(Path(path)))
        matches = [t for t in pe.net.mdtables.TypeDef
                   if str(t.TypeName) == 'GDriveDriver' and str(t.TypeNamespace) == 'SafeInCloud.Clouds.GDrive']
        if len(matches) != 1:
            raise ValueError()
        methods = [m.row for m in matches[0].MethodList if str(m.row.Name) == 'GetAuthTask']
        if len(methods) != 1:
            raise ValueError()
        body = CilMethodBody(CilMethodBodyReaderBytes(pe.get_data(methods[0].Rva, 100000)))
        result, pending = {}, None
        for instruction in body.instructions:
            token = getattr(instruction.operand, 'value', 0)
            if str(instruction.opcode) == 'ldstr' and token >> 24 == 0x70:
                pending = str(pe.net.user_strings.get(token & 0xffffff))
            elif str(instruction.opcode) == 'callvirt' and token >> 24 == 0x0a:
                member = pe.net.mdtables.MemberRef.rows[(token & 0xffffff)-1]
                owner = member.Class.row
                if str(getattr(owner, 'TypeName', '')) == 'ClientSecrets' and str(member.Name) in ('set_ClientId', 'set_ClientSecret'):
                    if pending is None:
                        raise ValueError()
                    result['client_id' if str(member.Name) == 'set_ClientId' else 'client_secret'] = pending
                    pending = None
        if set(result) != {'client_id', 'client_secret'} or not result['client_id'].endswith('.apps.googleusercontent.com'):
            raise ValueError()
        return result
    except Exception:
        raise OAuthError('Original desktop client configuration unavailable or unsupported.') from None


def _token_request(fields):
    try:
        request = Request(TOKEN_ENDPOINT, data=urlencode(fields).encode('ascii'),
                          headers={'Content-Type': 'application/x-www-form-urlencoded'}, method='POST')
        with urlopen(request, timeout=30) as response:
            raw = response.read(MAX_JSON + 1)
        if len(raw) > MAX_JSON:
            raise ValueError()
        result = json.loads(raw)
        if not isinstance(result, dict) or not isinstance(result.get('access_token'), str) or not result['access_token']:
            raise ValueError()
        return result
    except (OSError, ValueError, TypeError, HTTPError, URLError):
        raise OAuthError('Google authorization or token refresh failed. Please reconnect.') from None


class OfficialOAuth:
    def __init__(self, config_dir=None):
        self.config_dir = Path(config_dir) if config_dir else Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config'))) / 'safeincloud-linux'
        self.token_file = self.config_dir / 'official-google-token.json'
        self.client_file = self.config_dir / 'official-client.json'
        self._lock = threading.Lock()

    def _directory(self):
        self.config_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = self.config_dir.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
            raise OAuthError('Unsafe local authorization directory.')
        os.chmod(self.config_dir, 0o700)

    def _read_tokens(self):
        try:
            info = self.token_file.lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > MAX_JSON:
                raise ValueError()
            data = json.loads(self.token_file.read_text())
            if not isinstance(data, dict):
                raise ValueError()
            return data
        except FileNotFoundError:
            return {}
        except (OSError, ValueError, TypeError):
            raise OAuthError('Local authorization data is unreadable or has unsafe permissions.') from None

    def _client(self):
        try:
            if self.client_file.stat().st_size > MAX_JSON:
                raise ValueError()
            config = json.loads(self.client_file.read_text())
            path = config['path']
            if not isinstance(path, str) or not Path(path).is_absolute():
                raise ValueError()
        except (OSError, ValueError, TypeError, KeyError):
            raise OAuthError('Select the supplied official Windows executable first.') from None
        return extract_client(path)

    def configured(self):
        try:
            tokens = self._read_tokens()
            return bool(tokens.get('refresh_token') or tokens.get('access_token'))
        except OAuthError:
            return False

    @property
    def scope(self):
        return self._read_tokens().get('scope', '')

    def _save(self, response, previous, requested_scope=None):
        tokens = dict(previous)
        for key in ('access_token', 'refresh_token', 'scope', 'token_type'):
            if key in response and isinstance(response[key], str) and response[key]:
                tokens[key] = response[key]
        if requested_scope and 'scope' not in response:
            tokens['scope'] = requested_scope
        try:
            expires = float(response.get('expires_in', 3600))
            if not 0 < expires <= 365 * 86400:
                raise ValueError()
            tokens['expires_at'] = time.time() + expires
        except (ValueError, TypeError):
            raise OAuthError('Invalid Google authorization response.') from None
        self._directory()
        descriptor, temporary = tempfile.mkstemp(prefix='.oauth-', dir=self.config_dir)
        try:
            with os.fdopen(descriptor, 'w') as handle:
                os.fchmod(handle.fileno(), 0o600)
                json.dump(tokens, handle)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, self.token_file)
        finally:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass
        return tokens

    def connect(self, scope=APPDATA_SCOPE):
        if scope not in ALLOWED_SCOPES:
            raise OAuthError('Unsupported Google authorization scope.')
        with self._lock:
            client = self._client()
            state = secrets.token_urlsafe(32)
            verifier = secrets.token_urlsafe(64)
            challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode('ascii')).digest()).rstrip(b'=').decode('ascii')
            result = {}

            class Handler(BaseHTTPRequestHandler):
                def log_message(self, *_):
                    pass

                def do_GET(self):
                    try:
                        parsed = urlsplit(self.path)
                        query = parse_qs(parsed.query, keep_blank_values=True, max_num_fields=12)
                        states = query.get('state', [])
                        valid = parsed.path == '/callback' and len(states) == 1 and hmac.compare_digest(states[0], state)
                        if not valid:
                            self.send_error(400, 'Invalid authorization callback')
                            return
                        codes = query.get('code', [])
                        if 'error' in query:
                            result['denied'] = True
                        elif len(codes) == 1 and codes[0] and len(codes[0]) < 8192:
                            result['code'] = codes[0]
                        else:
                            self.send_error(400, 'Invalid authorization callback')
                            return
                        body = b'Authorization received. You can close this tab and return to SafeInCloud Linux.'
                        self.send_response(200)
                        self.send_header('Content-Type', 'text/plain; charset=utf-8')
                        self.send_header('Cache-Control', 'no-store')
                        self.send_header('Content-Length', str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                    except (ValueError, OSError):
                        pass

            class CallbackServer(HTTPServer):
                def get_request(self):
                    connection, address = super().get_request()
                    connection.settimeout(2)
                    return connection, address

            try:
                with CallbackServer(('127.0.0.1', 0), Handler) as server:
                    server.timeout = 0.5
                    redirect = f'http://127.0.0.1:{server.server_port}/callback'
                    params = {'client_id': client['client_id'], 'redirect_uri': redirect,
                              'response_type': 'code', 'scope': scope, 'state': state,
                              'code_challenge': challenge, 'code_challenge_method': 'S256',
                              'access_type': 'offline', 'prompt': 'consent'}
                    if not webbrowser.open(AUTH_ENDPOINT + '?' + urlencode(params)):
                        raise OAuthError('Could not open the system browser for authorization.')
                    deadline = time.monotonic() + 300
                    while not result and time.monotonic() < deadline:
                        server.handle_request()
                if result.get('denied'):
                    raise OAuthError('Google authorization was declined.')
                if not result.get('code'):
                    raise OAuthError('Google authorization timed out. Please reconnect.')
                response = _token_request({**client, 'code': result['code'], 'code_verifier': verifier,
                                           'redirect_uri': redirect, 'grant_type': 'authorization_code'})
                if not response.get('refresh_token'):
                    raise OAuthError('Google did not return a refresh token. Please reconnect with consent.')
                tokens = self._save(response, {}, scope)
                return bool(tokens.get('access_token'))
            except OAuthError:
                raise
            except (OSError, ValueError, TypeError):
                raise OAuthError('Local Google authorization failed. Please reconnect.') from None

    def access_token(self):
        with self._lock:
            tokens = self._read_tokens()
            try:
                fresh = float(tokens.get('expires_at', 0)) > time.time() + 60
            except (ValueError, TypeError):
                fresh = False
            if fresh and isinstance(tokens.get('access_token'), str):
                return tokens['access_token']
            if not isinstance(tokens.get('refresh_token'), str) or not tokens['refresh_token']:
                raise OAuthError('Connect Google Drive first.')
            response = _token_request({**self._client(), 'refresh_token': tokens['refresh_token'], 'grant_type': 'refresh_token'})
            return self._save(response, tokens)['access_token']
