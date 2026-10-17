"""No network: exercise whole-file sync and conditional HTTP request construction."""
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from urllib.error import HTTPError

from safeincloud_linux.drive import GoogleDrive, DriveSession, DriveConflict, DriveError, DrivePermissionError
from safeincloud_linux.vault import Vault, encrypt


class SyncTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.path = self.directory / 'local.db'
        self.original = encrypt(b'<database><card id="1" title="Before"/></database>', 'test')
        self.path.write_bytes(self.original)
        self.vault = Vault.open(self.path, 'test')
        self.drive = GoogleDrive(config=self.directory / 'rclone.conf', binary='unused')
        self.drive._request = Mock()
        self.drive.writable = Mock(return_value=True)
        self.candidate = {'id': 'file-id', 'path': 'SafeInCloud.db'}
        self.session = DriveSession(self.drive, self.candidate, self.path, {'etag': '"base"'}, self.original)
        self.session.persist()

    def edit(self):
        self.vault.upsert('1', 'Changed', [])

    def test_conditional_upload_uses_baseline_and_confirms_checksum(self):
        self.edit()
        def request(file_id, **kwargs):
            if kwargs:
                self.assertEqual(kwargs['etag'], '"base"')
                self.assertEqual(kwargs['data'], self.path.read_bytes())
                return {'etag': '"new"'}
            if self.drive._request.call_count == 1:
                return {'etag': '"base"'}
            return {'etag': '"new"', 'md5Checksum': hashlib.md5(self.path.read_bytes()).hexdigest()}
        self.drive._request.side_effect = request
        self.session(self.vault)
        self.assertEqual(self.session.etag, '"new"')
        self.assertEqual(self.session.base_digest, hashlib.sha256(self.path.read_bytes()).digest())
        self.assertEqual(list(self.directory.glob('pending-upload-*')), [])
        self.assertTrue(list(self.directory.glob('local.db.backup-*')))

    def test_remote_conflict_keeps_unsaved_edits_and_does_not_put(self):
        self.edit()
        self.drive._request.return_value = {'etag': '"other"'}
        with self.assertRaises(DriveConflict):
            self.session(self.vault)
        self.assertEqual(Vault.open(self.path, 'test').cards()[0].get('title'), 'Changed')
        self.assertEqual(self.drive._request.call_count, 1)
        self.assertEqual(self.session.etag, '"base"')

    def test_permission_and_missing_etag_never_upload(self):
        self.edit()
        self.drive.writable.return_value = False
        self.drive._request.return_value = {'etag': '"base"'}
        with self.assertRaises(DrivePermissionError):
            self.session(self.vault)
        self.session.etag = None
        self.drive.writable.return_value = True
        self.drive._request.return_value = {}
        with self.assertRaises(DriveError):
            self.session(self.vault)
        self.assertEqual(self.drive._request.call_count, 2)

    def test_failed_conditional_put_keeps_encrypted_recovery_copy(self):
        self.edit()
        self.drive._request.side_effect = [{'etag': '"base"'}, DriveConflict('changed')]
        with self.assertRaises(DriveConflict):
            self.session(self.vault)
        recovery = list(self.directory.glob('pending-upload-*'))
        self.assertEqual(len(recovery), 1)
        self.assertEqual(recovery[0].read_bytes(), self.path.read_bytes())
        self.assertEqual(self.session.etag, '"base"')

    def test_changed_remote_without_local_edits_pulls_then_reloads(self):
        from safeincloud_linux.drive import _replace_encrypted
        remote = encrypt(b'<database><card id="2" title="Remote"/></database>', 'test')
        self.drive._request.return_value = {'etag': '"remote"'}
        def pull(*args, **kwargs):
            kwargs["validate"](remote)
            _replace_encrypted(self.path, remote)
            return DriveSession(self.drive, self.candidate, self.path, {'etag': '"remote"'}, remote)
        self.drive.open_candidate = Mock(side_effect=pull)
        self.session(self.vault)
        self.assertEqual(self.vault.cards()[0].get('title'), 'Remote')
        self.assertEqual(self.session.etag, '"remote"')
        self.assertEqual(self.drive._request.call_count, 1)
        self.assertTrue(list(self.directory.glob('local.db.backup-*')))

    def test_reopen_preserves_pending_local_copy_and_old_baseline(self):
        # Use the actual stable-name algorithm and actual open_candidate implementation.
        stable = self.directory / ('vault-' + hashlib.sha256(b'file-id').hexdigest()[:24] + '.db')
        self.path.rename(stable)
        self.session.path = stable
        self.session.persist()
        changed = encrypt(b'<database><card id="3" title="Pending"/></database>', 'test')
        stable.write_bytes(changed)
        meta = {'etag': '"remote"', 'fileSize': len(self.original), 'md5Checksum': hashlib.md5(self.original).hexdigest()}
        self.drive._request.return_value = meta
        self.drive._run = Mock(side_effect=lambda args: Path(args[-1]).write_bytes(self.original))
        session = self.drive.open_candidate(self.candidate, self.directory)
        self.assertEqual(stable.read_bytes(), changed)
        self.assertEqual(session.etag, '"base"')
        self.assertEqual(session.base_digest, hashlib.sha256(self.original).digest())

    def test_missing_or_corrupt_baseline_never_overwrites_local(self):
        stable = self.directory / ('vault-' + hashlib.sha256(b'file-id').hexdigest()[:24] + '.db')
        pending = encrypt(b'<database><card id="3" title="Pending"/></database>', 'test')
        stable.write_bytes(pending)
        meta = {'etag': '"remote"', 'fileSize': len(self.original), 'md5Checksum': hashlib.md5(self.original).hexdigest()}
        self.drive._request.return_value = meta
        self.drive._download_id = Mock(side_effect=lambda _id, destination: Path(destination).write_bytes(self.original))
        state = stable.with_suffix('.sync.json')
        for raw in (None, 'broken', '{}', '{"base_digest":"bad"}', '[1,2]'):
            if raw is None:
                state.unlink(missing_ok=True)
            else:
                state.write_text(raw)
            with self.subTest(raw=raw), self.assertRaises(DriveConflict):
                self.drive.open_candidate(self.candidate, self.directory)
            self.assertEqual(stable.read_bytes(), pending)
            self.assertEqual(list(self.directory.glob('drive-*.db')), [])

    def test_wrong_password_pull_never_replaces_local_ciphertext(self):
        from safeincloud_linux import DatabaseError
        stable = self.directory / ('vault-' + hashlib.sha256(b'file-id').hexdigest()[:24] + '.db')
        self.path.rename(stable)
        self.session.path = stable
        self.session.persist()
        self.vault = Vault.open(stable, 'test')
        remote = encrypt(b'<database><card id="2" title="Remote"/></database>', 'different')
        meta = {'etag': '"new"', 'fileSize': len(remote), 'md5Checksum': hashlib.md5(remote).hexdigest()}
        self.drive._request.return_value = meta
        self.drive._download_id = Mock(side_effect=lambda _id, destination: Path(destination).write_bytes(remote))
        with self.assertRaises(DatabaseError):
            self.session(self.vault)
        self.assertEqual(stable.read_bytes(), self.original)
        self.assertEqual(self.vault.cards()[0].get('title'), 'Before')
        self.assertEqual(self.session.etag, '"base"')
        self.assertEqual(list(self.directory.glob('drive-*.db')), [])


class RequestTests(unittest.TestCase):
    def setUp(self):
        self.drive = GoogleDrive(binary='unused')
        self.drive._token = Mock(return_value='fake-token')

    def test_put_sets_if_match_and_uses_stable_id(self):
        response = io.BytesIO(b'{"etag":"new"}')
        opener = Mock()
        opener.open.return_value = response
        with patch('urllib.request.build_opener', return_value=opener):
            self.drive._request('file-id', data=b'fake-encrypted', etag='"revision"')
        request = opener.open.call_args.args[0]
        self.assertEqual(request.get_method(), 'PUT')
        self.assertEqual(request.get_header('If-match'), '"revision"')
        self.assertIn('/file-id?', request.full_url)
        self.assertIn('newRevision=true', request.full_url)

    def test_missing_weak_etag_refused_before_auth_or_network(self):
        for etag in (None, '', 'W/"weak"'):
            with self.subTest(etag=etag), self.assertRaises(DriveError):
                self.drive._request('file-id', data=b'fake', etag=etag)
        self.drive._token.assert_not_called()

    def test_http_precondition_failure_becomes_conflict(self):
        opener = Mock()
        opener.open.side_effect = HTTPError('https://example.invalid', 412, 'condition', {}, None)
        with patch('urllib.request.build_opener', return_value=opener), self.assertRaises(DriveConflict):
            self.drive._request('file-id', data=b'fake', etag='"revision"')

    def test_connect_requests_write_scope_only_explicitly(self):
        self.drive._run = Mock()
        self.drive.connect()
        self.assertIn('drive.readonly', self.drive._run.call_args.args[0])
        self.drive.connect(writable=True)
        args = self.drive._run.call_args.args[0]
        self.assertEqual(args[args.index('scope') + 1], 'drive')
