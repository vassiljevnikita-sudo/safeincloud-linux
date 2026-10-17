# Research record — 2026-10-17

## Sources

- Official Portable app: https://www.safe-in-cloud.com/download/SafeInCloud_Portable.zip
- Vendor download instructions: https://support.safe-in-cloud.com/portal/en/kb/articles/download
- Existing implementation: https://github.com/mxschmitt/golang-safe-in-cloud
  at `8819e485adaa349c404735509dae0540534b6ac3` (MIT).
- Ghidra: https://github.com/NationalSecurityAgency/ghidra/releases/tag/Ghidra_12.1.4_build

## Download fingerprints

SHA-256:

| Artifact | SHA-256 |
| --- | --- |
| Official Portable ZIP | `9d955d26f3373af5c9b41c20168dfd72b702413a305ed762c0ae14241112d94d` |
| Official SafeInCloud.exe | `84fc4764fce03ccb81f2d4d07128d70e517054ad51eded26841de550635ae117` |
| Ghidra 12.1.4 ZIP | `ddac49f903da9d5bac833e5cc79395098b9c33cfd3279be5f31bd00387d2d4db` |

The Ghidra ZIP hash matched the hash published in the GitHub release body.
The Portable archive lists its executable with timestamp 2026-10-09. This is
archive metadata, not proof of a product version or release date.

## Tools and results

Ghidra 12.1.4 was downloaded and unpacked locally. Its headless importer ran
successfully with a local Temurin JDK 21. Import was explicitly `-noanalysis`;
this is not a claim that Ghidra recovered the managed application's logic.
The missing external `MSCOREE.DLL` warning did not prevent import.

The executable is a PE32 .NET assembly named `SafeInCloud`. Python `dnfile` and
`dncil` were used for focused metadata and IL inspection without executing the
application. The relevant methods are:

- `SafeInCloud.Database.DatabaseCipher.GetEncryptStream`, `GetDecryptStream`,
  `GetSecretKey`, `ReadByteArray`, `WriteByteArray`, `GetStrategy`.
- `Version1Strategy.GetPasswordBytes` and `Version2Strategy.GetPasswordBytes`.

Observed constants include magic 1285 (`0x0505`), outer KDF iterations 10000,
verification KDF iterations 1000, key length 32, block size 128 bits, AES key size
256 bits, CBC mode and PKCS7 padding enum values. The writer generates a 64-byte
password salt, a 32-byte payload key and a 64-byte verification salt.
The reader's compression/XML path is additionally supported by the public Go
implementation and its encrypted sample. No claim of full binary equivalence is
made. SHA1 follows the legacy .NET KDF constructor and public interoperable code.

## Validation and next milestone

Nine core unittest cases pass on Fedora 44 / Python 3.14. A local Xvfb GUI smoke
check exercised unlock with the public sample, select, mask, reveal and lock.
Tk was loaded from locally extracted Fedora RPMs for that check; no privileged
system installation was performed.

The next useful compatibility check is a newly created disposable vault with
fake entries, an ASCII password and then a Unicode password, exported/backed up
by the current official client. Test opening those locally before considering
real data. Cloud writes and merging require a separate design and compatibility
suite and are intentionally absent from this prototype.
