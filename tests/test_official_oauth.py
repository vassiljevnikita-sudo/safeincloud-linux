import base64
import hashlib
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import urlopen
from safeincloud_linux.official_oauth import APPDATA_SCOPE, OfficialOAuth, OAuthError, extract_client


class OfficialOAuthTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.oauth = OfficialOAuth(Path(self.tmp.name) / 'config')

    def test_not_configured_and_missing_client(self):
        self.assertFalse(self.oauth.configured())
        with self.assertRaises(OAuthError):
            self.oauth._client()
        with self.assertRaises(OAuthError):
            self.oauth.access_token()

    def test_private_atomic_tokens_preserve_refresh(self):
        self.oauth._save({'access_token':'fake-access','refresh_token':'fake-refresh','expires_in':60}, {}, APPDATA_SCOPE)
        self.assertEqual(self.oauth.token_file.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.oauth.config_dir.stat().st_mode & 0o777, 0o700)
        tokens = self.oauth._save({'access_token':'replacement','expires_in':3600}, self.oauth._read_tokens())
        self.assertEqual(tokens['refresh_token'], 'fake-refresh')
        self.assertEqual(self.oauth.scope, APPDATA_SCOPE)
        self.assertEqual(self.oauth.access_token(), 'replacement')

    def test_reject_public_token_permissions_and_symlink(self):
        self.oauth._save({'access_token':'fake'}, {})
        self.oauth.token_file.chmod(0o644)
        self.assertFalse(self.oauth.configured())
        self.oauth.token_file.unlink()
        other = Path(self.tmp.name)/'other'
        other.write_text('{}')
        self.oauth.token_file.symlink_to(other)
        with self.assertRaises(OAuthError):
            self.oauth._read_tokens()

    def test_refresh_in_memory_no_client_config_persisted(self):
        self.oauth._save({'access_token':'old','refresh_token':'fake-refresh','expires_in':1}, {})
        with patch.object(self.oauth, '_client', return_value={'client_id':'fake-id','client_secret':'fake-secret'}), patch('safeincloud_linux.official_oauth._token_request', return_value={'access_token':'renewed','expires_in':3600}) as request:
            self.assertEqual(self.oauth.access_token(), 'renewed')
            self.assertEqual(request.call_args.args[0]['grant_type'], 'refresh_token')
        stored = self.oauth.token_file.read_text()
        self.assertNotIn('fake-secret', stored)
        self.assertNotIn('fake-id', stored)

    def test_oauth_pkce_bad_state_rejected_then_valid(self):
        callbacks = []
        threads = []
        captured = {}
        def open_browser(url):
            query = parse_qs(urlsplit(url).query)
            captured.update({k:v[0] for k,v in query.items()})
            def callback():
                redirect = captured['redirect_uri']
                self.assertTrue(redirect.startswith('http://127.0.0.1:'))
                try:
                    urlopen(redirect+'?'+urlencode({'code':'fake-code','state':'wrong'}), timeout=3)
                except HTTPError as exc:
                    callbacks.append(exc.code)
                    exc.close()
                with urlopen(redirect+'?'+urlencode({'code':'fake-code','state':captured['state']}), timeout=3) as response:
                    callbacks.append(response.status)
            thread = threading.Thread(target=callback)
            threads.append(thread)
            thread.start()
            return True
        def exchange(fields):
            challenge = base64.urlsafe_b64encode(hashlib.sha256(fields['code_verifier'].encode()).digest()).rstrip(b'=').decode()
            self.assertEqual(challenge, captured['code_challenge'])
            self.assertEqual(fields['grant_type'], 'authorization_code')
            return {'access_token':'fake-access','refresh_token':'fake-refresh','expires_in':3600}
        with patch.object(self.oauth, '_client', return_value={'client_id':'fake-id','client_secret':'fake-secret'}), patch('safeincloud_linux.official_oauth.webbrowser.open', side_effect=open_browser), patch('safeincloud_linux.official_oauth._token_request', side_effect=exchange):
            self.assertTrue(self.oauth.connect())
        for thread in threads:
            thread.join(3)
        self.assertEqual(callbacks, [400,200])
        self.assertEqual(captured['scope'], APPDATA_SCOPE)
        self.assertEqual(captured['code_challenge_method'], 'S256')
        self.assertNotIn('fake-secret', self.oauth.token_file.read_text())

    def test_extract_only_expected_client_initialization(self):
        method = NS(Name='GetAuthTask', Rva=1)
        owner = NS(TypeName='ClientSecrets')
        members = [NS(Name='set_ClientId', Class=NS(row=owner)), NS(Name='set_ClientSecret', Class=NS(row=owner))]
        typedef = NS(TypeName='GDriveDriver', TypeNamespace='SafeInCloud.Clouds.GDrive', MethodList=[NS(row=method)])
        strings = {1:'fake.apps.googleusercontent.com', 2:'fixture-secret'}
        fake_pe = NS(net=NS(mdtables=NS(TypeDef=[typedef], MemberRef=NS(rows=members)), user_strings=NS(get=strings.get)), get_data=lambda *_:b'fake')
        instructions = [NS(opcode=op, operand=NS(value=value)) for op,value in
                        [('ldstr',0x70000001), ('callvirt',0x0a000001), ('ldstr',0x70000002), ('callvirt',0x0a000002)]]
        with patch('dnfile.dnPE', return_value=fake_pe), patch('dncil.cil.body.CilMethodBody', return_value=NS(instructions=instructions)):
            self.assertEqual(extract_client('/fixture.exe'), {'client_id':'fake.apps.googleusercontent.com','client_secret':'fixture-secret'})

    def test_extraction_sanitizes_error(self):
        with self.assertRaises(OAuthError) as context:
            extract_client('/nonexistent/private-secret-token')
        self.assertNotIn('private-secret-token', str(context.exception))

    def test_scope_rejected_without_browser(self):
        with self.assertRaises(OAuthError):
            self.oauth.connect('unexpected')


if __name__ == '__main__':
    unittest.main()
