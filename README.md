# SafeInCloud Linux

Experimental, unofficial **read-only** desktop viewer and CLI for local copies of
SafeInCloud encrypted databases. Runs natively on Linux without Wine or a Windows
VM. This is a working interoperability prototype, not a complete replacement for
Safe. It has no affiliation with SafeInCloud SAS.

## Fedora

```sh
sudo dnf install python3 python3-pip python3-tkinter git
git clone https://github.com/vassiljevnikita-sudo/safeincloud-linux.git
cd safeincloud-linux
python3 -m venv .venv
.venv/bin/pip install .
.venv/bin/safeincloud-linux /path/to/COPY-of-SafeInCloud.db --gui
```

The GUI prompts locally for the master password. Select a card and explicitly
choose Reveal to view its fields and notes. All fields are masked initially.
Click Lock when finished; there is no automatic lock timer yet. The app never
writes to the database, starts a server, or accesses the network.

CLI (lists titles and IDs only; password is prompted without echo):

```sh
.venv/bin/safeincloud-linux /path/to/COPY-of-SafeInCloud.db
.venv/bin/safeincloud-linux /path/to/COPY-of-SafeInCloud.db --show CARD_ID
```

`--show` deliberately prints the chosen card's values to your terminal. No password
argument, plaintext export, or persistent password storage is provided.
To try the public upstream example, use `tests/fixtures/upstream-demo.db` with the
public fixture password `foobar`.

## What has actually been verified

- Decryption and parsing of the existing MIT-licensed upstream sample database.
- Independent synthetic envelopes, wrong passwords, truncated files, invalid
  XML, oversized decompression, key verification, and Unicode password strategies.
- Desktop unlock, card selection, masking, reveal, and lock under Xvfb on Fedora 44.
- Static inspection of the official Portable executable downloaded on 2026-10-17
  confirms the envelope constants and password-byte strategies. See
  [research notes](docs/RESEARCH.md) and [format notes](docs/FORMAT.md).

**Not yet verified:** a newly created database from a running current official
client, your database, cloud synchronization, browser autofill, attachment
handling, passkeys, TOTP generation, or writing/merging databases. Unknown format
versions are rejected. A successfully opened CBC file does not prove authenticity.

Use a copy of an encrypted database for compatibility testing. You do not need to
share your master password or real vault with the project. The viewer holds
plaintext in process memory while unlocked; Python cannot guarantee secure memory
erasure. Manually locking drops application references and clears visible fields.
Input limits are 32 MiB encrypted and 64 MiB decompressed.

## Development

```sh
.venv/bin/pip install -e .
.venv/bin/python -m unittest discover -s tests -v
```

## Credits and license

MIT. The implementation is informed by Max Schmitt's
[golang-safe-in-cloud](https://github.com/mxschmitt/golang-safe-in-cloud), pinned
for this research at `8819e485adaa349c404735509dae0540534b6ac3`.
The public sample database is from that project; its original license is preserved
in [THIRD_PARTY_LICENSE](THIRD_PARTY_LICENSE). The reader adds bounds checking,
key-descriptor verification, strategy-aware password encoding, and hardened XML
parsing. No official application binaries or decompiled source are distributed.
