# Experimental encrypted editing

`safeincloud_linux::vault::Vault` keeps XML and the password in process memory. Its
writer generates the documented AES-CBC + PBKDF2-SHA1 + zlib envelope with random
64-byte salts, fresh payload key and fresh IVs, including the payload-key
verification descriptor. It preserves the loaded strategy byte: strategy 1 uses
the low bytes of UTF-16 code units, strategy 2 UTF-8.

`Vault::open(path, password)` validates/decrypts the input; `cards()` returns
visible cards. `upsert(card_id, title, fields, notes)` edits a card or creates one
when `card_id` is `None` and returns its string ID. Fields are Rust `Field` values
with `name`, `kind` and `value`. Unknown card attributes,
non-field children, XML comments and processing instructions survive serialization.
Existing field attributes and children survive matching by name, then position.
A removed field is intentionally removed. Existing labels remain unchanged.
`delete(card_id)` moves the card to trash using `deleted="true"`; it does not
permanently erase it. `dirty` records pending API mutations.

`encrypted()` returns ciphertext without writing disk. `save()` checks
the SHA-256 baseline (`snapshot`), creates an exclusive encrypted timestamped
backup, fsyncs a mode-0600 ciphertext temporary file, checks the baseline again,
atomically replaces the destination and fsyncs its directory. The backup path
is exposed as `last_backup`. A changed file returns an error; no automatic
merge is attempted. The two checks narrow but cannot eliminate a simultaneous
uncooperative writer's race. External synchronization needs an independent
revision check before upload. `mark_saved(exact_encrypted_bytes)` updates the
baseline after successful persistence; it must not be called before that succeeds.
`reload_from_disk()` validates and replaces the in-memory tree after a pull,
refusing when dirty. It retains the current password and refreshes the snapshot.
Dropping the Rust vault wipes its password and XML values. The GUI additionally
replaces its entire process on lock to end GTK copies and background workers.
Unsaved changes cross that boundary only as ciphertext through an anonymous
descriptor and require the master password again. A background sync is
dispatched only after the local save succeeds, so lock cannot forward recovery
with an obsolete disk baseline.

## Compatibility evidence

Focused dnfile/dncil inspection of the official executable fingerprint already
recorded in RESEARCH.md establishes:

* `XItem.get_TimeStamp` uses `time_stamp`; `get_FirstStamp` falls back from
  `first_stamp` to `prev_stamp` then `time_stamp`.
* The nested XItem `Builder.SetTimeStamp()` uses current Unix milliseconds when
  greater than the old stamp, otherwise old stamp + 2. Its explicit overload
  initializes `first_stamp` with the fallback chain above.
* `MiscUtils.CurrentTimeMillis(true)` uses UTC and `ToMillis` subtracts 1970-01-01.
* XCard's nested `Builder.AddLabel` adds a `<label_id>` with integer text.
* `Builder.SetIsDeleted(true)` sets `deleted="true"`; false removes the attribute.
* `DatabaseAdapter.DeleteCard` calls `AddGhost`, which constructs `<ghost>` with
  the same ID and replaces the item through `PutGhost`. Permanent deletion and
  ghost merging are deliberately absent from this API.

IDs are generated as unused positive signed-32-bit integers, checked across all
root children. Existing IDs are preserved. Unknown XML elements, encrypted
attachments and other opaque content are retained, but their semantics are not
implemented. XML lexical details (quote style, prefixes, declaration layout,
CDATA boundaries) are not preserved. Secret field history is retained unchanged;
a full Windows-compatible history/score calculation is not claimed.

Synthetic/public-fixture tests cover encrypted roundtrip, both Unicode
strategies, metadata preservation, trash, locking, encrypted backups, permission
mode and external-change rejection. The upstream public encrypted sample is
read and reserialized; no real user vault is used. The Rust migration additionally
reads Unicode fixtures written by the predecessor Python writer, and the
predecessor reader independently accepts Rust-written strategy-1 and strategy-2
fixtures; see [fixture notes](../tests/fixtures/README.md). No Go toolchain was available
for running the separate upstream decryptor. A current official client has NOT
yet opened a writer-produced disposable vault. Until that independent test is
performed, local editing is experimental and cloud writes must not be presented
as production-compatible synchronization.
