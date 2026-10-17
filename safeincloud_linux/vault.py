"""In-memory editing and encrypted, conflict-checked local persistence."""
import copy
import hashlib
import os
from pathlib import Path
import secrets
import tempfile
import time
import xml.etree.ElementTree as ET
import zlib

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from defusedxml import ElementTree as SafeET
from . import DatabaseError, MAX_FILE, MAX_XML, cards, decrypt


class ConflictError(DatabaseError):
    """The file changed after opening; reload before saving."""


def _encrypt(key, iv, data):
    size = 16 - len(data) % 16
    cipher = Cipher(algorithms.AES(key), modes.CBC(iv)).encryptor()
    return cipher.update(data + bytes([size]) * size) + cipher.finalize()


def _pack(data):
    return bytes([len(data)]) + data


def encrypt(xml, password, version=2):
    """Generate the legacy official two-layer envelope with fresh keys and IVs."""
    if version not in (1, 2) or len(xml) > MAX_XML:
        raise DatabaseError('Unsupported version or oversized XML')
    salt, iv, key_salt = secrets.token_bytes(64), secrets.token_bytes(16), secrets.token_bytes(64)
    key, payload_iv = secrets.token_bytes(32), secrets.token_bytes(16)
    pwd = password.encode('utf-16-le', errors='surrogatepass')[::2] if version == 1 else password.encode('utf-8')
    outer = hashlib.pbkdf2_hmac('sha1', pwd, salt, 10000, 32)
    check = hashlib.pbkdf2_hmac('sha1', key, key_salt, 1000, 32)
    descriptor = _pack(payload_iv) + _pack(key) + _pack(check)
    data = (b'\x05\x05' + bytes([version]) + _pack(salt) + _pack(iv) + _pack(key_salt)
            + _pack(_encrypt(outer, iv, descriptor)) + _encrypt(key, payload_iv, zlib.compress(xml)))
    if len(data) > MAX_FILE:
        raise DatabaseError('Encrypted database exceeds file size limit')
    return data


class Vault:
    @classmethod
    def open(cls, path, password):
        path = Path(path)
        with path.open('rb') as handle:
            data = handle.read(MAX_FILE + 1)
        xml = decrypt(data, password)
        # Retain comments, processing instructions, unknown elements and attributes.
        parser = SafeET.DefusedXMLParser(target=ET.TreeBuilder(insert_comments=True, insert_pis=True), forbid_dtd=True)
        parser.feed(xml)
        root = parser.close()
        vault = cls()
        vault.path, vault.password, vault.root = path, password, root
        vault.version = data[2]
        vault.snapshot = hashlib.sha256(data).hexdigest()
        vault.dirty = False
        vault.last_backup = None
        return vault

    def _check_open(self):
        if self.root is None or self.password is None:
            raise DatabaseError('Vault is locked')

    def cards(self):
        self._check_open()
        return cards(self.root)

    def _card(self, card_id):
        return next((c for c in self.root.findall('card') if c.get('id') == str(card_id)), None)

    def _stamp(self, element):
        old = int(element.get('time_stamp', '0'))
        stamp = max(int(time.time() * 1000), old + 2)
        if not element.get('first_stamp'):
            element.set('first_stamp', element.get('prev_stamp') or str(old or stamp))
        element.set('time_stamp', str(stamp))

    def upsert(self, card_id, title, fields, notes='', label_ids=None):
        self._check_open()
        card = self._card(card_id) if card_id is not None else None
        if card_id is not None and card is None:
            raise DatabaseError('Card no longer exists')
        specs = [{k: str(f.get(k, '')) for k in ('name', 'type', 'value')} for f in fields]
        if card is None:
            used = {e.get('id') for e in self.root if e.get('id')}
            while True:
                ident = str(secrets.randbelow(2147483646) + 1)
                if ident not in used:
                    break
            card = ET.SubElement(self.root, 'card', id=ident, symbol='key', color='gray')
        card.set('title', str(title))
        existing = card.findall('field')
        replacement = []
        for index, spec in enumerate(specs):
            match = next((f for f in existing if f.get('name') == spec['name']), None)
            if match is None and index < len(card.findall('field')):
                candidate = card.findall('field')[index]
                if candidate in existing:
                    match = candidate
            field = copy.deepcopy(match) if match is not None else ET.Element('field')
            if match is not None:
                existing.remove(match)
            field.set('name', spec['name'])
            field.set('type', spec['type'])
            field.text = spec['value']
            replacement.append(field)
        old_fields = card.findall('field')
        insertion = list(card).index(old_fields[0]) if old_fields else 0
        for field in old_fields:
            card.remove(field)
        for offset, field in enumerate(replacement):
            card.insert(insertion + offset, field)
        note = card.find('notes')
        if note is None:
            note = ET.SubElement(card, 'notes')
        note.text = str(notes)
        if label_ids is not None:
            for label in card.findall('label_id'):
                card.remove(label)
            for ident in dict.fromkeys(str(x) for x in label_ids):
                ET.SubElement(card, 'label_id').text = ident
        self._stamp(card)
        self.dirty = True
        return card.get('id')

    def delete(self, card_id):
        """Move to the official deleted-card trash, preserving recoverable data."""
        self._check_open()
        card = self._card(card_id)
        if card is None:
            raise DatabaseError('Card no longer exists')
        card.set('deleted', 'true')
        self._stamp(card)
        self.dirty = True

    def serialize_encrypted(self):
        self._check_open()
        xml = ET.tostring(self.root, encoding='utf-8', xml_declaration=True)
        return encrypt(xml, self.password, self.version)

    def mark_saved(self, encrypted=None):
        """After successfully persisting EXACTLY these bytes, update the baseline."""
        if encrypted is None:
            encrypted = self.path.read_bytes()
        self.snapshot = hashlib.sha256(encrypted).hexdigest()
        self.dirty = False

    def save(self):
        self._check_open()
        data = self.serialize_encrypted()
        original = self.path.read_bytes()
        if hashlib.sha256(original).hexdigest() != self.snapshot:
            raise ConflictError('Database changed outside this app; reload before saving')
        backup = self.path.with_name(self.path.name + '.backup-' + str(time.time_ns()))
        with backup.open('xb') as handle:
            os.chmod(backup, 0o600)
            handle.write(original)
            handle.flush()
            os.fsync(handle.fileno())
        self.last_backup = backup
        fd, temp = tempfile.mkstemp(prefix='.' + self.path.name + '-', dir=self.path.parent)
        try:
            with os.fdopen(fd, 'wb') as handle:
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
            # A second check narrows, but cannot eliminate, the external-writer race.
            if hashlib.sha256(self.path.read_bytes()).hexdigest() != self.snapshot:
                raise ConflictError('Database changed while saving; original preserved')
            os.replace(temp, self.path)
            directory = os.open(self.path.parent, os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
            self.mark_saved(data)
            return backup
        finally:
            if os.path.exists(temp):
                os.unlink(temp)

    def reload_from_disk(self):
        """Reload a pulled encrypted file, refusing to discard pending edits."""
        self._check_open()
        if self.dirty:
            raise ConflictError("Unsaved changes; save or discard before reloading")
        fresh = type(self).open(self.path, self.password)
        self.root.clear()
        self.root = fresh.root
        self.version = fresh.version
        self.snapshot = fresh.snapshot
        self.dirty = False

    def close(self):
        if self.root is not None:
            self.root.clear()
        self.root = None
        self.password = None
        self.dirty = False
