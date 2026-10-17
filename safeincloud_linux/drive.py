"""Google Drive discovery and explicit, conditional encrypted-file synchronization."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from . import MAX_FILE


class DriveError(ValueError):
    pass


class GoogleDrive:
    def __init__(self, config=None, binary=None):
        self.binary = binary or shutil.which("rclone")
        if not self.binary:
            local = Path.home() / ".local/bin/rclone"
            if local.is_file():
                self.binary = str(local)
        self.config = Path(config) if config else Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "safeincloud-linux/rclone.conf"

    def _run(self, args, timeout=120):
        if not self.binary:
            raise DriveError("rclone fehlt. Bitte rclone installieren und erneut versuchen.")
        self.config.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        fd = os.open(self.config, os.O_CREAT | os.O_WRONLY, 0o600)
        os.close(fd)
        self.config.chmod(0o600)
        try:
            result = subprocess.run([self.binary, "--config", str(self.config), *args],
                                    capture_output=True, timeout=timeout, stdin=subprocess.DEVNULL)
        except (OSError, subprocess.TimeoutExpired):
            raise DriveError("Google Drive ist nicht erreichbar oder die Anmeldung wurde nicht abgeschlossen.") from None
        finally:
            if self.config.exists():
                self.config.chmod(0o600)
        if result.returncode:
            # rclone errors can include OAuth responses: never expose raw output.
            raise DriveError("Google-Drive-Zugriff fehlgeschlagen. Bitte anmelden und die Verbindung prüfen.")
        return result.stdout

    def connect(self, writable=False):
        self._run(["config", "create", "safeincloud-google", "drive", "scope", "drive" if writable else "drive.readonly",
                   "config_is_local", "true", "--no-output"], timeout=300)

    def list_databases(self):
        raw = self._run(["lsjson", "safeincloud-google:", "--recursive", "--files-only", "--include", "*.db"])
        try:
            rows = json.loads(raw)
            if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
                raise ValueError("Invalid listing")
            return sorted([r["Path"] for r in rows if not r.get("IsDir") and isinstance(r.get("Path"), str)
                           and 0 <= r.get("Size", -1) <= MAX_FILE])
        except (ValueError, TypeError, KeyError):
            raise DriveError("Die Dateiliste von Google Drive konnte nicht gelesen werden.") from None

    def download(self, remote_path, directory):
        if not remote_path or remote_path.startswith("/") or "\x00" in remote_path:
            raise DriveError("Ungültiger Dateipfad.")
        data = self._run(["cat", "safeincloud-google:" + remote_path, "--head", str(MAX_FILE + 1)])
        if len(data) > MAX_FILE:
            raise DriveError("Die Datenbank ist größer als 32 MiB.")
        if data[:2] != b"\x05\x05":
            raise DriveError("Die gewählte Datei ist keine unterstützte verschlüsselte SafeInCloud-Datenbank.")
        # Never use remote path components as local filesystem names.
        fd, name = tempfile.mkstemp(prefix="drive-", suffix=".db", dir=directory)
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(data)
        except BaseException:
            Path(name).unlink(missing_ok=True)
            raise
        return Path(name)

    def configured(self):
        from configparser import ConfigParser
        parser = ConfigParser(interpolation=None)
        parser.read(self.config)
        return bool(parser.get("safeincloud-google", "token", fallback=""))

    def writable(self):
        from configparser import ConfigParser
        parser = ConfigParser(interpolation=None)
        parser.read(self.config)
        scope = parser.get("safeincloud-google", "scope", fallback="")
        return scope == "drive"

    def discover(self):
        """Find normal/shared database files, retaining IDs to avoid path ambiguity."""
        result = []
        seen = set()
        for shared in (False, True):
            args = ["lsjson", "safeincloud-google:", "--recursive", "--files-only", "--ignore-case", "--include", "*.db"]
            if shared:
                args.append("--drive-shared-with-me")
            raw = self._run(args)
            try:
                rows = json.loads(raw)
                if not isinstance(rows, list):
                    raise ValueError()
                for row in rows:
                    if (not isinstance(row, dict) or row.get("IsDir") or not isinstance(row.get("Path"), str)
                            or not isinstance(row.get("ID"), str) or not 0 <= row.get("Size", -1) <= MAX_FILE):
                        continue
                    if row["ID"] not in seen:
                        seen.add(row["ID"])
                        result.append({"id": row["ID"], "path": row["Path"], "shared": shared})
            except (ValueError, TypeError):
                raise DriveError("Die Dateiliste von Google Drive konnte nicht gelesen werden.") from None
        return sorted(result, key=lambda row: row["path"].casefold())

    def preferred(self, candidates):
        """Only choose automatically when the identity is unambiguous."""
        try:
            remembered = json.loads(self.config.with_name("source.json").read_text()).get("id")
        except (OSError, ValueError, AttributeError):
            remembered = None
        previous = [row for row in candidates if row["id"] == remembered]
        if len(previous) == 1:
            return previous[0]
        canonical = [row for row in candidates if Path(row["path"]).name.casefold() == "safeincloud.db"]
        if len(canonical) == 1:
            return canonical[0]
        if len(candidates) == 1:
            return candidates[0]
        return None

    def remember(self, candidate):
        name = self.config.with_name("source.json")
        fd, temporary = tempfile.mkstemp(dir=name.parent)
        try:
            with os.fdopen(fd, "w") as handle:
                json.dump({"id": candidate["id"], "path": candidate.get("path", "SafeInCloud.db")}, handle)
            os.replace(temporary, name)
        finally:
            Path(temporary).unlink(missing_ok=True)

    def cached_session(self, directory):
        """Open the encrypted working copy without requiring network connectivity."""
        import hashlib
        import re
        try:
            remembered = json.loads(self.config.with_name("source.json").read_text())
            ident = remembered["id"]
            if not isinstance(ident, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", ident):
                return None
            path = Path(directory) / ("vault-" + hashlib.sha256(ident.encode()).hexdigest()[:24] + ".db")
            state = json.loads(path.with_suffix(".sync.json").read_text())
            if state.get("id") != ident or not re.fullmatch(r"[0-9a-f]{64}", str(state.get("base_digest", ""))):
                return None
            if not path.is_file() or path.stat().st_size > MAX_FILE:
                return None
            candidate = {"id": ident, "path": remembered.get("path", "SafeInCloud.db"), "shared": False}
            session = DriveSession(self, candidate, path, {"etag": state.get("etag")}, b"")
            session.base_digest = bytes.fromhex(state["base_digest"])
            return session
        except (OSError, ValueError, TypeError, KeyError):
            return None

    def _token(self):
        # rclone refreshes an expired OAuth token and persists it in its config.
        from configparser import ConfigParser
        self._run(["about", "safeincloud-google:", "--json"])
        parser = ConfigParser(interpolation=None)
        parser.read(self.config)
        try:
            token = json.loads(parser.get("safeincloud-google", "token"))["access_token"]
            if not token or not isinstance(token, str):
                raise ValueError()
            return token
        except Exception:
            raise DriveError("Google-Anmeldung fehlt. Bitte erneut verbinden.") from None

    def _request(self, file_id, *, data=None, etag=None, metadata=False):
        """Use v2 ETags for atomic conditional writes; no path-based overwrites."""
        import re
        from urllib.error import HTTPError, URLError
        from urllib.request import Request, build_opener, HTTPRedirectHandler
        if not re.fullmatch(r"[A-Za-z0-9_-]+", file_id):
            raise DriveError("Ungültige Google-Datei-ID.")
        if data is None:
            url = "https://www.googleapis.com/drive/v2/files/" + file_id
            url += "?supportsAllDrives=true&fields=id,etag,md5Checksum,title,fileSize"
            method = "GET"
        else:
            if not etag or etag.startswith("W/"):
                raise DriveError("Ohne gültigen Versionsschutz wird keine Cloud-Datei überschrieben.")
            if metadata:
                url = "https://www.googleapis.com/drive/v2/files/" + file_id + "?supportsAllDrives=true"
            else:
                url = "https://www.googleapis.com/upload/drive/v2/files/" + file_id + "?uploadType=media&supportsAllDrives=true&newRevision=true"
            method = "PUT"
        headers = {"Authorization": "Bearer " + self._token()}
        if data is not None:
            headers.update({"If-Match": etag, "Content-Type": "application/json" if metadata else "application/octet-stream"})
        class NoRedirect(HTTPRedirectHandler):
            def redirect_request(self, *_args, **_kwargs):
                return None
        try:
            with build_opener(NoRedirect()).open(Request(url, data=data, headers=headers, method=method), timeout=60) as response:
                raw = response.read(1024 * 1024 + 1)
                if len(raw) > 1024 * 1024:
                    raise DriveError("Unerwartet große Google-Antwort.")
                return json.loads(raw)
        except HTTPError as error:
            error.close()
            if error.code == 412:
                raise DriveConflict("Die Drive-Datei wurde zwischenzeitlich geändert. Es wurde nichts überschrieben.") from None
            if error.code in (401, 403):
                raise DrivePermissionError("Google verweigert den Schreibzugriff. Bitte Drive-Schreibzugriff erlauben.") from None
            raise DriveError("Google Drive hat den Vorgang abgelehnt.") from None
        except (URLError, TimeoutError, ValueError, OSError):
            raise DriveError("Google-Drive-Verbindung fehlgeschlagen. Lokale Änderungen bleiben erhalten.") from None

    def _download_id(self, file_id, destination):
        return self._run(["backend", "copyid", "safeincloud-google:", file_id, str(destination)])

    def open_candidate(self, candidate, directory, validate=None):
        """Bind download to a stable ID, checksum, and remote ETag."""
        import hashlib
        metadata = self._request(candidate["id"])
        # Metadata size is checked before transfer; the official transport also
        # enforces a bounded read while streaming from Google.
        if int(metadata.get("fileSize", MAX_FILE + 1)) > MAX_FILE:
            raise DriveError("Die Datenbank ist größer als 32 MiB.")
        fd, name = tempfile.mkstemp(prefix="drive-", suffix=".db", dir=directory)
        os.close(fd)
        try:
            self._download_id(candidate["id"], name)
            path = Path(name)
            if path.stat().st_size > MAX_FILE:
                raise DriveError("Die Datenbank ist größer als 32 MiB.")
            data = path.read_bytes()
            after = self._request(candidate["id"])
            if metadata.get("etag") != after.get("etag") or hashlib.md5(data).hexdigest() != after.get("md5Checksum"):
                raise DriveConflict("Die Datei wurde beim Laden geändert. Bitte erneut versuchen.")
            if data[:2] != b"\x05\x05":
                raise DriveError("Keine unterstützte verschlüsselte SafeInCloud-Datenbank.")
            if validate is not None:
                validate(data)
            self.remember(candidate)
            stable = Path(directory) / ("vault-" + hashlib.sha256(candidate["id"].encode()).hexdigest()[:24] + ".db")
            state_file = stable.with_suffix(".sync.json")
            try:
                previous = json.loads(state_file.read_text())
            except (OSError, ValueError):
                previous = None
            import re
            valid_previous = (isinstance(previous, dict) and previous.get("id", candidate["id"]) == candidate["id"]
                              and re.fullmatch(r"[0-9a-f]{64}", str(previous.get("base_digest", ""))))
            if stable.exists() and not valid_previous and stable.read_bytes() != data:
                raise DriveConflict("Lokale Kopie ohne gültige Abgleichsbasis gefunden. Sie wurde nicht überschrieben.")
            if stable.exists() and valid_previous:
                if hashlib.sha256(stable.read_bytes()).hexdigest() != previous.get("base_digest"):
                    # Keep unsynchronized local edits across application restarts.
                    session = DriveSession(self, candidate, stable, {"etag": previous.get("etag")}, data)
                    session.base_digest = bytes.fromhex(previous["base_digest"])
                    path.unlink()
                    return session
            if stable.exists() and stable.read_bytes() != data:
                _replace_encrypted(stable, data)
                path.unlink()
            else:
                os.replace(path, stable)
                stable.chmod(0o600)
            session = DriveSession(self, candidate, stable, after, data)
            session.persist()
            return session
        except BaseException:
            Path(name).unlink(missing_ok=True)
            raise


class DriveConflict(DriveError):
    pass


class DrivePermissionError(DriveError):
    pass


class DriveSession:
    """Explicit whole-file synchronization; refuses concurrent remote changes."""
    auto_sync_on_unlock = True
    def __init__(self, drive, candidate, path, metadata, data):
        import hashlib
        self.drive, self.candidate, self.path = drive, candidate, Path(path)
        self.etag = metadata.get("etag")
        self.base_digest = hashlib.sha256(data).digest()

    def persist(self):
        name = self.path.with_suffix(".sync.json")
        fd, temporary = tempfile.mkstemp(dir=name.parent)
        try:
            with os.fdopen(fd, "w") as handle:
                json.dump({"id": self.candidate["id"], "etag": self.etag,
                           "base_digest": self.base_digest.hex()}, handle)
            os.replace(temporary, name)
        finally:
            Path(temporary).unlink(missing_ok=True)

    def enable_writes(self):
        self.drive.connect(writable=True)
        return "Drive-Schreibzugriff ist eingerichtet. Jetzt erneut synchronisieren."

    def __call__(self, vault):
        import hashlib
        if vault.dirty:
            vault.save()
        local = self.path.read_bytes()
        latest = self.drive._request(self.candidate["id"])
        if latest.get("etag") != self.etag:
            if hashlib.sha256(local).digest() == self.base_digest:
                # No local edits: safely pull the newer remote version, preserving backup.
                from . import decrypt
                fresh = self.drive.open_candidate(self.candidate, self.path.parent,
                                                 validate=lambda data: decrypt(data, vault.password))
                vault.reload_from_disk()
                self.etag, self.base_digest = fresh.etag, fresh.base_digest
                self.persist()
                return "Neuere Drive-Version geladen. Die vorherige Kopie bleibt als Backup erhalten."
            raise DriveConflict("Die Drive-Datei wurde auf einem anderen Gerät geändert. Lokale Änderungen sind gespeichert; kein Überschreiben.")
        if hashlib.sha256(local).digest() == self.base_digest:
            return "Keine lokalen Änderungen. Drive-Version ist unverändert."
        if not self.drive.writable():
            raise DrivePermissionError("Zum Speichern im Drive zuerst ‚Drive-Schreibzugriff erlauben‘ wählen.")
        if not self.etag:
            raise DriveError("Google lieferte keine sichere Dateiversion. Cloud-Speichern wurde abgebrochen.")
        # Preserve an encrypted recovery copy locally before the network mutation.
        fd, recovery_name = tempfile.mkstemp(prefix="pending-upload-", suffix=".db", dir=self.path.parent)
        recovery = Path(recovery_name)
        with os.fdopen(fd, "wb") as handle:
            handle.write(local)
            handle.flush()
            os.fsync(handle.fileno())
        self.drive._request(self.candidate["id"], data=local, etag=self.etag)
        refreshed = self.drive._request(self.candidate["id"])
        if refreshed.get("md5Checksum") != hashlib.md5(local).hexdigest():
            raise DriveConflict("Die Cloud-Version konnte nach dem Speichern nicht bestätigt werden. Wiederherstellungskopie bleibt erhalten.")
        self.etag = refreshed.get("etag")
        self.base_digest = hashlib.sha256(local).digest()
        self.persist()
        recovery.unlink()
        return "Verschlüsselte Datenbank mit Google Drive synchronisiert."


def _replace_encrypted(path, data):
    """Keep previous ciphertext and atomically install new ciphertext."""
    import uuid
    backup = path.with_name(path.name + ".backup-" + uuid.uuid4().hex + ".db")
    fd = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(path.read_bytes())
        handle.flush()
        os.fsync(handle.fileno())
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)
