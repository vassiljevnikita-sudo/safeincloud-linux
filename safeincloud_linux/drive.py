"""Read-only Google Drive transport via a dedicated rclone OAuth configuration."""
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

    def connect(self):
        self._run(["config", "create", "safeincloud-google", "drive", "scope", "drive.readonly",
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
