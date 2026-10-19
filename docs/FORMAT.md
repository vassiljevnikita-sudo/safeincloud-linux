# Observed SafeInCloud envelope

This describes interoperability observations, not a complete vendor specification.
Little-endian 16-bit magic is `0x0505`, followed by a strategy byte.
Each array below is encoded as one unsigned length byte plus exactly that many
bytes. The remaining file after the final header array is the encrypted payload.

1. Magic (2 bytes), strategy (1 byte).
2. Password salt (array).
3. Outer AES IV (array, 16 bytes).
4. Payload-key verification salt (array).
5. Encrypted key descriptor (array, CBC block-aligned).
6. Encrypted payload (remainder, CBC block-aligned).

Outer key: PBKDF2-HMAC-SHA1(password bytes, password salt, 10000, 32 bytes).
AES-256-CBC decrypts the descriptor. The descriptor contains three arrays:
payload IV, payload AES key, and a key-verification value.
When the verification value is present it must equal
PBKDF2-HMAC-SHA1(payload key, verification salt, 1000, 32 bytes).
Some public synthetic upstream files omit the verification value; this reader
accepts that empty legacy variant too. This value checks the key, not the payload;
it is not a MAC or proof of authenticity.

Decrypt the remaining payload using its AES key and IV. Its plaintext begins with
a zlib stream whose output is an XML `<database>` document. Existing Go code stops
at the zlib end marker and ignores the following CBC padding; this reader follows
that behavior. It does not claim to validate every padding byte.

## Password encoding: a material difference from the Go library

The official .NET `DatabaseCipher.GetStrategy` selects `Version1Strategy` when the
byte is 1, otherwise `Version2Strategy`. Their observed `GetPasswordBytes` methods:

- Strategy 1: obtain UTF-16 code units, convert each to a byte (low 8 bits only).
  Rust equivalent: `password.encode_utf16().map(|unit| unit as u8).collect::<Vec<_>>()`.
- Strategy 2: UTF-8 bytes.

The inspected writer emits strategy 1. The upstream Go reader always uses UTF-8,
which differs for non-ASCII passwords. This project handles 1 and 2 explicitly,
and rejects other values instead of copying the official catch-all dispatch.
Unicode cases are tested with independently constructed envelopes, but not yet
against a Unicode vault created by a running official client.

## XML subset displayed

The GTK interface displays direct `<card>` children excluding `template="true"` and
`deleted="true"`. It displays title, ID, direct `<field>` names/types/text, and
`<notes>`. Attachments, record merging, ghosts, and newer features are not
implemented. The editor preserves unknown elements and attributes when saving;
their semantics and complete official-client roundtrips are not established.
