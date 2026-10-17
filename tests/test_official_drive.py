import io
import json
import tempfile
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs, urlparse
from safeincloud_linux import MAX_FILE
from safeincloud_linux.drive import DriveError, DrivePermissionError
from safeincloud_linux.official_drive import OfficialDrive


class OfficialDriveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.oauth = Mock()
        self.oauth.access_token.return_value = 'fake-official-token'
        self.oauth.configured.return_value = True
        self.drive = OfficialDrive(config=self.directory / 'rclone.conf', oauth=self.oauth)

    def test_discovers_private_app_folder_and_all_pages(self):
        self.drive._get = Mock(side_effect=[
            json.dumps({'files': [{'id': 'one', 'name': 'SafeInCloud.db', 'size': '100'},
                                  {'id': 'no', 'name': 'other.db', 'size': '100'}], 'nextPageToken': 'next'}).encode(),
            json.dumps({'files': [{'id': 'two', 'name': 'SafeInCloud-work.db', 'size': str(MAX_FILE)},
                                  {'id': 'big', 'name': 'SafeInCloud-big.db', 'size': str(MAX_FILE + 1)}]}).encode()])
        rows = self.drive.discover()
        self.assertEqual({r['id'] for r in rows}, {'one', 'two'})
        first, second = self.drive._get.call_args_list
        self.assertEqual(first.args[0], 'files')
        self.assertEqual(first.args[1]['spaces'], 'appDataFolder')
        self.assertEqual(first.args[1]['q'], 'trashed = false')
        self.assertNotIn('pageToken', first.args[1])
        self.assertEqual(second.args[1]['pageToken'], 'next')

    def test_download_addresses_stable_id_not_filename_and_limits_bytes(self):
        self.drive._get = Mock(return_value=b'fake-ciphertext')
        path = self.directory / 'private.db'
        self.drive._download_id('file/id', path)
        self.drive._get.assert_called_once_with('files/file%2Fid', {'alt': 'media'}, MAX_FILE)
        self.assertEqual(path.read_bytes(), b'fake-ciphertext')

    def test_original_oauth_provider_used_without_rclone(self):
        self.drive._run = Mock(side_effect=AssertionError('rclone must not run'))
        self.assertTrue(self.drive.configured())
        self.assertTrue(self.drive.writable())
        self.drive.connect(writable=True)
        self.oauth.connect.assert_called_once_with()
        self.assertEqual(self.drive._token(), 'fake-official-token')
        self.drive._run.assert_not_called()
        self.assertFalse(self.drive.config.exists())

    def test_media_transport_uses_original_provider_and_enforces_response_bound(self):
        opener = Mock()
        opener.open.return_value = io.BytesIO(b'12345')
        with patch('safeincloud_linux.official_drive.build_opener', return_value=opener), self.assertRaises(DriveError):
            self.drive._get('files/id', {'alt': 'media'}, 4)
        req = opener.open.call_args.args[0]
        self.assertEqual(req.get_header('Authorization'), 'Bearer fake-official-token')
        self.assertEqual(parse_qs(urlparse(req.full_url).query), {'alt': ['media']})

    def test_upload_inherits_conditional_if_match(self):
        opener = Mock()
        opener.open.return_value = io.BytesIO(b'{"etag":"new"}')
        with patch('urllib.request.build_opener', return_value=opener):
            self.drive._request('stable-id', data=b'fake-encrypted', etag='"old"')
        req = opener.open.call_args.args[0]
        self.assertEqual(req.get_method(), 'PUT')
        self.assertEqual(req.get_header('If-match'), '"old"')
        self.assertEqual(req.get_header('Authorization'), 'Bearer fake-official-token')
        self.assertIn('/stable-id?', req.full_url)
        self.oauth.access_token.assert_called_once_with()

    def test_private_folder_permission_error_does_not_fall_back_to_root(self):
        from urllib.error import HTTPError
        opener = Mock()
        opener.open.side_effect = HTTPError('https://example.invalid', 403, 'denied', {}, None)
        with patch('safeincloud_linux.official_drive.build_opener', return_value=opener), self.assertRaises(DrivePermissionError):
            self.drive.discover()
        self.assertEqual(opener.open.call_count, 1)
