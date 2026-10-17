import hashlib
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
from safeincloud_linux import DatabaseError, decrypt
from safeincloud_linux.vault import Vault, ConflictError, encrypt


XML = b'<database future="yes"><!--keep--><label id="9" name="Home"/><card id="7" title="Old" time_stamp="123" future="x"><field name="Secret" type="password" unknown="yes">fake<extra foo="bar"/></field><notes>old</notes><image opaque="yes"/></card><unknown a="b"/></database>'


class VaultTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'demo.db'
        self.path.write_bytes(encrypt(XML, 'päss🔒', 2))
        self.vault = Vault.open(self.path, 'päss🔒')

    def test_edit_preserves_unknown_and_updates_stamp(self):
        self.vault.upsert('7', 'New', [dict(name='Secret', type='password', value='newfake')], 'notes', ['9'])
        root = ET.fromstring(decrypt(self.vault.serialize_encrypted(), 'päss🔒'))
        card = root.find('card')
        self.assertEqual(card.get('future'), 'x')
        self.assertEqual(card.find('field').get('unknown'), 'yes')
        self.assertIsNotNone(card.find('field/extra'))
        self.assertIsNotNone(card.find('image'))
        self.assertIsNotNone(root.find('unknown'))
        self.assertEqual(card.get('first_stamp'), '123')
        self.assertGreater(int(card.get('time_stamp')), 123)
        self.assertEqual(card.findtext('label_id'), '9')
        self.assertIn(b'<!--keep-->', decrypt(self.vault.serialize_encrypted(), 'päss🔒'))

    def test_save_backup_and_roundtrip(self):
        original = self.path.read_bytes()
        self.vault.upsert(None, 'Demo', [dict(name='Login', type='login', value='fake')])
        backup = self.vault.save()
        self.assertEqual(backup.read_bytes(), original)
        self.assertFalse(self.vault.dirty)
        self.assertEqual(len(Vault.open(self.path, 'päss🔒').cards()), 2)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.vault.snapshot, hashlib.sha256(self.path.read_bytes()).hexdigest())
        self.assertEqual(len(list(self.path.parent.iterdir())), 2)

    def test_external_change_is_never_overwritten(self):
        self.path.write_bytes(b'changed')
        with self.assertRaises(ConflictError):
            self.vault.save()
        self.assertEqual(self.path.read_bytes(), b'changed')
        self.assertEqual(len(list(self.path.parent.iterdir())), 1)

    def test_trash_preserves_card(self):
        self.vault.delete('7')
        self.assertEqual(self.vault.cards(), [])
        self.assertEqual(self.vault.root.find('card').get('deleted'), 'true')
        self.assertIsNotNone(self.vault.root.find('card/field'))

    def test_unicode_strategies_preserved(self):
        for version in (1, 2):
            self.path.write_bytes(encrypt(XML, 'päss🔒', version))
            vault = Vault.open(self.path, 'päss🔒')
            encrypted = vault.serialize_encrypted()
            self.assertEqual(encrypted[2], version)
            self.assertEqual(ET.fromstring(decrypt(encrypted, 'päss🔒')).get('future'), 'yes')
            self.assertNotEqual(encrypted, vault.serialize_encrypted())

    def test_reload_after_pull_and_unsaved_rejection(self):
        self.path.write_bytes(encrypt(b'<database><card id="4" title="Cloud"/></database>', 'päss🔒', 1))
        self.vault.reload_from_disk()
        self.assertEqual(self.vault.cards()[0].get("title"), "Cloud")
        self.assertEqual(self.vault.version, 1)
        self.vault.delete("4")
        with self.assertRaises(ConflictError):
            self.vault.reload_from_disk()
        self.assertTrue(self.vault.dirty)

    def test_lock(self):
        self.vault.close()
        self.assertIsNone(self.vault.password)
        self.assertIsNone(self.vault.root)
        with self.assertRaises(DatabaseError):
            self.vault.serialize_encrypted()

    def test_external_public_fixture_roundtrip(self):
        vault = Vault.open(Path(__file__).parent / 'fixtures/upstream-demo.db', 'foobar')
        original_cards = len(vault.cards())
        self.assertEqual(len(ET.fromstring(decrypt(vault.serialize_encrypted(), 'foobar')).findall('card')), len(vault.root.findall('card')))
        self.assertGreater(original_cards, 0)
