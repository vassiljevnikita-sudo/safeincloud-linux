import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from safeincloud_linux import MAX_FILE
from safeincloud_linux.drive import GoogleDrive, DriveError


class DriveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.drive = GoogleDrive(config=Path(self.temp.name) / "config/rclone.conf", binary="rclone")

    def test_login_readonly_and_no_token_output(self):
        with patch.object(self.drive, "_run") as run:
            self.drive.connect()
        args = run.call_args.args[0]
        self.assertIn("drive.readonly", args)
        self.assertIn("--no-output", args)
        self.assertNotIn("drive", args[args.index("scope") + 1:args.index("scope") + 2])

    def test_listing_ignores_folders_and_oversized_files(self):
        rows = [{"Path": "folder/Test.db", "Size": 100}, {"Path": "big.db", "Size": MAX_FILE+1},
                {"Path": "folder.db", "IsDir": True, "Size": 0}]
        with patch.object(self.drive, "_run", return_value=json.dumps(rows).encode()):
            self.assertEqual(self.drive.list_databases(), ["folder/Test.db"])

    def test_invalid_listing(self):
        for value in [b"not-json", b"{}", b'["bad"]']:
            with patch.object(self.drive, "_run", return_value=value), self.assertRaises(DriveError):
                self.drive.list_databases()

    def test_download_uses_private_unpredictable_local_name(self):
        with patch.object(self.drive, "_run", return_value=b"\x05\x05\x01fake") as run:
            path = self.drive.download("folder/name.db", self.temp.name)
        self.assertEqual(path.parent, Path(self.temp.name))
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(path.read_bytes(), b"\x05\x05\x01fake")
        self.assertEqual(run.call_args.args[0][-2:], ["--head", str(MAX_FILE + 1)])

    def test_bad_download_leaves_no_file(self):
        for value in [b"<html>login</html>", b"\x05\x05" + b"x" * MAX_FILE]:
            with patch.object(self.drive, "_run", return_value=value), self.assertRaises(DriveError):
                self.drive.download("test.db", self.temp.name)
        self.assertEqual(list(Path(self.temp.name).iterdir()), [])

    def test_transport_errors_do_not_expose_oauth_response(self):
        result = subprocess.CompletedProcess([], 1, b"secret-token", b"secret-error")
        with patch("subprocess.run", return_value=result), self.assertRaises(DriveError) as caught:
            self.drive.list_databases()
        self.assertNotIn("secret", str(caught.exception))

    def test_config_permissions(self):
        self.drive.config.parent.mkdir(parents=True)
        self.drive.config.write_text("fake config")
        self.drive.config.chmod(0o644)
        with patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, b"[]", b"")):
            self.drive.list_databases()
        self.assertEqual(self.drive.config.stat().st_mode & 0o777, 0o600)

    def test_timeout(self):
        with patch("subprocess.run", side_effect=subprocess.TimeoutExpired("rclone", 120)), self.assertRaises(DriveError):
            self.drive.list_databases()
