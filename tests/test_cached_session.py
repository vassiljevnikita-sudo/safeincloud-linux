import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
from safeincloud_linux.drive import GoogleDrive


class CachedSessionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.drive = GoogleDrive(config=self.root/'rclone.conf', binary='unused')
        self.drive._request = Mock(side_effect=AssertionError('Offline cache must not access network'))
        self.candidate = {'id':'fake-id', 'path':'SafeInCloud.db'}
        self.drive.remember(self.candidate)
        self.path = self.root / ('vault-'+hashlib.sha256(b'fake-id').hexdigest()[:24]+'.db')
        self.path.write_bytes(b'fake encrypted pending edits')
        self.state = self.path.with_suffix('.sync.json')

    def test_offline_copy_keeps_pending_edits_and_baseline(self):
        digest = hashlib.sha256(b'earlier remote ciphertext').hexdigest()
        self.state.write_text(json.dumps({'id':'fake-id','etag':'"prior"','base_digest':digest}))
        session = self.drive.cached_session(self.root)
        self.assertEqual(session.path, self.path)
        self.assertEqual(session.etag, '"prior"')
        self.assertEqual(session.base_digest.hex(), digest)
        self.drive._request.assert_not_called()
        self.assertEqual(self.path.read_bytes(), b'fake encrypted pending edits')

    def test_corrupt_or_mismatched_baseline_is_not_used(self):
        for data in ['bad-json', json.dumps({'id':'other','base_digest':'0'*64}), json.dumps({'id':'fake-id','base_digest':'invalid'})]:
            self.state.write_text(data)
            self.assertIsNone(self.drive.cached_session(self.root))
