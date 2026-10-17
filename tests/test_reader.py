import hashlib
import unittest
from pathlib import Path
from safeincloud_linux import DatabaseError, MAX_XML, cards, decrypt
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
import zlib

FIXTURE = Path(__file__).parent / "fixtures" / "upstream-demo.db"


def synthetic(payload, password="test", version=1, corrupt_check=False):
    # Independently construct the publicly documented two-layer CBC envelope.
    def array(b):
        return bytes([len(b)]) + b
    def enc(key, iv, b):
        pad = 16 - len(b) % 16
        ctx = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
        return ctx.update(b + bytes([pad]) * pad) + ctx.finalize()
    salt, iv, key, body_iv = b"s" * 16, b"i" * 16, b"k" * 32, b"j" * 16
    pwd = password.encode("utf-16-le")[::2] if version == 1 else password.encode("utf-8")
    derived = hashlib.pbkdf2_hmac("sha1", pwd, salt, 10000, 32)
    check = hashlib.pbkdf2_hmac("sha1", key, salt, 1000, 32)
    if corrupt_check:
        check = b"x" * 32
    wrapped = enc(derived, iv, array(body_iv) + array(key) + array(check))
    return b"\x05\x05" + bytes([version]) + array(salt) + array(iv) + array(salt) + array(wrapped) + enc(key, body_iv, zlib.compress(payload))


class ReaderTests(unittest.TestCase):
    def test_external_fixture(self):
        from defusedxml import ElementTree
        xml = decrypt(FIXTURE.read_bytes(), "foobar")
        root = ElementTree.fromstring(xml)
        self.assertEqual(root.tag, "database")
        self.assertEqual(root.find("card").get("id"), "101")
        self.assertEqual(root.find("label").get("name"), "Geschäftlich")
        self.assertTrue(all(c.get("template") != "true" for c in cards(root)))

    def test_wrong_password(self):
        with self.assertRaises(DatabaseError):
            decrypt(FIXTURE.read_bytes(), "wrong")

    def test_truncated_headers_and_payload(self):
        data = FIXTURE.read_bytes()
        for length in list(range(150)) + [len(data) - 1]:
            with self.subTest(length=length), self.assertRaises(DatabaseError):
                decrypt(data[:length], "foobar")

    def test_unknown_version(self):
        data = bytearray(FIXTURE.read_bytes())
        data[2] = 99
        with self.assertRaises(DatabaseError):
            decrypt(bytes(data), "foobar")

    def test_independent_envelope(self):
        xml = '<database><card title="Demo" id="1"><field name="Password" type="password">fake-only</field></card></database>'.encode()
        self.assertEqual(decrypt(synthetic(xml), "test"), xml)

    def test_unicode_password_strategies(self):
        for version in (1, 2):
            xml = b"<database/>"
            encrypted = synthetic(xml, "päss🔒", version)
            self.assertEqual(decrypt(encrypted, "päss🔒"), xml)

    def test_key_verification(self):
        with self.assertRaises(DatabaseError):
            decrypt(synthetic(b"<database/>", corrupt_check=True), "test")

    def test_forbidden_xml(self):
        for xml in [b'<wrong/>', b'<!DOCTYPE database [<!ENTITY x "bad">]><database>&x;</database>']:
            with self.assertRaises(DatabaseError):
                decrypt(synthetic(xml), "test")

    def test_decompression_limit(self):
        with self.assertRaises(DatabaseError):
            decrypt(synthetic(b" " * (MAX_XML + 1)), "test")


if __name__ == "__main__":
    unittest.main()
