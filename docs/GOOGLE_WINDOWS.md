# Google Drive interoperability findings

Analysis target: the official portable Windows `SafeInCloud.exe`, SHA-256
`84fc4764fce03ccb81f2d4d07128d70e517054ad51eded26841de550635ae117`.
This describes that binary, not a claim that every current SafeInCloud build has
identical behavior. Evidence is .NET metadata/CIL inspection using dnfile/dncil;
application client configuration and user tokens are deliberately omitted.

## Two distinct Google storage locations

`SafeInCloud.Clouds.GDrive.GDriveDriver.GetAuthTask` (method token `0x06001992`)
selects one Drive scope based on `IsRootFolder`:

- Root mode: `DriveService.Scope.Drive` (`https://www.googleapis.com/auth/drive`).
- App folder mode: `DriveService.Scope.DriveAppdata`
  (`https://www.googleapis.com/auth/drive.appdata`).

`GetFile` (`0x0600199B`) searches the Drive root by exact filename in root mode.
In app-folder mode it sets `spaces=appDataFolder` and searches there by filename.
`ListDatabases` (`0x060019A2`) makes the same location distinction before filtering
valid database names. Thus searching visible Drive for `SafeInCloud.db` alone
cannot discover a database stored in the official application's private folder.

Google documents `appDataFolder` as application-specific, hidden from other Drive
applications, and inaccessible from the Drive web UI. Granting our own OAuth
client `drive.appdata` accesses our app's folder, not the official app's folder.
Those files cannot be shared or moved between spaces using ordinary Drive API
operations. [Google application-data documentation](https://developers.google.com/workspace/drive/api/guides/appdata).

## Authentication used by the Windows build

The same `GetAuthTask` constructs `Google.Apis.Auth.OAuth2.ClientSecrets`, assigns
bundled desktop client configuration, and calls
`GoogleWebAuthorizationBroker.AuthorizeAsync` with the selected scope, user key
`user`, and `GDriveStore` as token datastore. No custom code receiver is passed.

The bundled `Google.Apis.Auth.dll` then follows standard installed-app OAuth:
`GoogleWebAuthorizationBroker`'s async state machine creates
`GoogleAuthorizationCodeFlow`, a default `LocalServerCodeReceiver`, and
`AuthorizationCodeInstalledApp`. `LocalServerCodeReceiver.GetRandomUnusedPort`
uses `IPAddress.Loopback` and `TcpListener`; `StartListener` starts an HTTP
listener, and `OpenBrowser` opens the authorization URL. No custom device
attestation or certificate-pinning implementation appears in this inspected
call path. This is static evidence, not a guarantee about Google's current
server-side policy or OAuth client configuration.

Google describes desktop apps as public clients that cannot keep a client
secret confidential, and recommends PKCE, state validation and loopback
redirects. The bundled desktop application identity is distinct from user
access/refresh tokens. Reusing that same locally supplied installed-app identity
with fresh, explicit user authorization is a technically plausible way to
address the same private folder; server acceptance was subsequently verified
on the user-authorized account as described below. It must not be described as the independent open-source app's own
OAuth registration. A distributed implementation should use an appropriate
vendor-supported arrangement and clearly describe the identity on its consent
screen. [Google desktop OAuth documentation](https://developers.google.com/identity/protocols/oauth2/native-app).

No client ID, client-secret value, existing access token or refresh token is
included in this repository. The local `official_oauth` implementation reads client configuration from the
user's supplied official executable into memory. A fresh browser authorization
requires explicit user consent, random state validation and PKCE S256. User
refresh tokens are saved separately in a mode-0600 local JSON file inside a
private directory; application identity is not copied into that token file.
It does not read the Windows user token store or bypass consent.

## Token storage in Windows

`GDriveStore.StoreSync` (`0x06001984`) serializes the Google token response as JSON
and calls `DatabaseConfig.SetValue`. `GetSync` (`0x06001982`) reads and deserializes
it. `DatabaseConfig.SaveConfig` (`0x060016AE`) UTF-8 encodes the JSON configuration
and calls `ProtectedData.Protect` with `DataProtectionScope.CurrentUser` before
saving; `LoadConfig` (`0x060016AD`) reverses that operation.

Legacy migration calls `AuthStorageUtils.LoadData`, which also uses Windows
`ProtectedData.Unprotect`. Copying Windows configuration files into Linux is
therefore not a portable authentication mechanism. The interoperable path is a
new explicit OAuth login and separate local token storage, not extraction of
another installation's login tokens.

## Live compatibility verification (2026-10-17)

After the user completed a fresh compatibility login, the original app-specific
Drive registration successfully listed one private Safe database, downloaded its
encrypted copy and returned its MD5 and v2 ETag. No master password was supplied to
this investigation and no plaintext database was inspected.

A separate disposable, encrypted empty test vault was created in the same
app-data space with a non-Safe database filename. A conditional v2 media update
using its current ETag succeeded; replaying the stale ETag returned HTTP 412.
The final ciphertext checksum remained correct. The disposable file was deleted
and the real password database was never modified by that check.

The managed driver was also inspected through REA 6.5.0 using
`inspect-managed-artifact` and selected `inspect-managed-members` for
`SafeInCloud.Clouds.GDrive.GDriveDriver`. REA repository revision:
`5d6029ea62a65ec34fd739ee40223c77c6fdb63b`.
Complete raw evidence stays local because the managed string heap includes
bundled client configuration; only sanitized observations are published here.

A second disposable-only end-to-end check exercised the actual `Vault` and
`DriveSession` together: autosave and upload, independent download/decrypt,
remote update and in-app reload, and preservation of both sides when concurrent
edits conflict. Its scratch file was deleted in `finally`; remembered-source
updates were disabled and the real vault ID was never accessed by that probe.
