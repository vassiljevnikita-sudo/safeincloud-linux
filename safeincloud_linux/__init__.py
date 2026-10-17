"""Experimental read-only interoperability with the legacy SafeInCloud format."""
import hashlib
import hmac
import io
import zlib
from pathlib import Path

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from defusedxml import ElementTree

MAX_FILE = 32 * 1024 * 1024
MAX_XML = 64 * 1024 * 1024


class DatabaseError(ValueError):
    """Invalid, unsupported, damaged database or incorrect password."""


def _array(stream):
    size = stream.read(1)
    if not size:
        raise DatabaseError("Truncated database header")
    value = stream.read(size[0])
    if len(value) != size[0]:
        raise DatabaseError("Truncated database header")
    return value


def _decrypt(key, iv, data):
    if len(iv) != 16 or len(key) not in (16, 24, 32) or not data or len(data) % 16:
        raise DatabaseError("Incorrect password or damaged database")
    ctx = Cipher(algorithms.AES(key), modes.CBC(iv)).decryptor()
    return ctx.update(data) + ctx.finalize()


def decrypt(data, password):
    """Return XML bytes in memory. Never writes a file or performs network I/O.

    This legacy CBC format is not authenticated; successful parsing does not
    establish authenticity. Unknown header versions are rejected.
    """
    if len(data) > MAX_FILE:
        raise DatabaseError("Database exceeds 32 MiB limit")
    if data[:2] != b"\x05\x05" or len(data) < 3 or data[2] not in (1, 2):
        raise DatabaseError("Unsupported database format/version")
    try:
        stream = io.BytesIO(data[3:])
        salt, iv = _array(stream), _array(stream)
        if not salt or len(iv) != 16:
            raise DatabaseError("Invalid outer header")
        key_salt = _array(stream)  # Payload-key verification salt.
        wrapped = _array(stream)
        password_bytes = (password.encode("utf-16-le", errors="surrogatepass")[::2]
                          if data[2] == 1 else password.encode("utf-8"))
        key = hashlib.pbkdf2_hmac("sha1", password_bytes, salt, 10000, 32)
        inner = io.BytesIO(_decrypt(key, iv, wrapped))
        payload_iv, payload_key = _array(inner), _array(inner)
        key_check = _array(inner)
        if key_check:
            expected = hashlib.pbkdf2_hmac("sha1", payload_key, key_salt, 1000, 32)
            if not hmac.compare_digest(expected, key_check):
                raise DatabaseError("Incorrect password or damaged key descriptor")
        compressed = _decrypt(payload_key, payload_iv, stream.read())
        decoder = zlib.decompressobj()
        xml = decoder.decompress(compressed, MAX_XML + 1)
        if len(xml) > MAX_XML or not decoder.eof:
            raise DatabaseError("Invalid or oversized compressed payload")
        # Legacy implementations ignore CBC padding after the zlib stream.
        root = ElementTree.fromstring(xml, forbid_dtd=True)
        if root.tag != "database":
            raise DatabaseError("Unexpected XML document")
        return xml
    except DatabaseError:
        raise
    except Exception:
        raise DatabaseError("Incorrect password, damaged or unsupported database") from None


def load(path, password):
    with Path(path).open("rb") as handle:
        data = handle.read(MAX_FILE + 1)
    return ElementTree.fromstring(decrypt(data, password), forbid_dtd=True)


def cards(root):
    return [c for c in root.findall("card")
            if c.get("template", "false").lower() != "true"
            and c.get("deleted", "false").lower() != "true"]
