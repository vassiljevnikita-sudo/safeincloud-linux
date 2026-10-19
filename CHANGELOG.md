# 1.0.0 — Rust + GTK4

- Replace the Python/Qt implementation with a native Rust/GTK4 application.
  Keep the existing encrypted-file format, local cache and private Drive identity.
- Port original-client metadata extraction, PKCE login, token refresh and
  ETag-conditional synchronization to Rust; no Python subprocess is required.
- Preserve existing-entry editing, labels, favorites, encrypted backups,
  website icons, four generator modes and local password strength assessment.
- Replace the secret-bearing process on lock; encrypted unsaved recovery uses
  an anonymous descriptor and requires the master password again.
- Follow the desktop's appearance through the Settings portal, including live
  changes. Embed the original local fonts without a network font service.
- Restructure the German README around the Life OS design philosophy, retain
  the product image and original-product link, and preserve upstream attribution.
- Add independent Unicode interoperability fixtures, synthetic Drive tests,
  native GTK workflow checks, a real exec boundary probe and Rust CI.

# 0.6.0

- Add a local password generator with four modes, a length slider, character
  options and explicit application from the entry editor. Memorable passwords
  use six random words by default from the bundled, attributed EFF Long Wordlist.
- Show pattern-aware zxcvbn password strength and labeled crack-time estimates
  in entry details, editor and generator. No password is sent to a service.
- Fetch website icons asynchronously over HTTPS directly from the site, with
  declared icon/CDN discovery, bounded requests and an in-memory session cache.
  Website icons can be disabled, and pending requests are cancelled on lock.
- Open existing entry editors with a double-click, retaining the Bearbeiten
  button and Ctrl+E. Verify encrypted edit/reopen and generator lock boundaries.
- Make Max Schmitt's upstream repository and the separate official Windows
  interoperability reference explicit. Ship the complete upstream MIT license
  and source notices in the package.

# 0.5.2

- Replace the secret-bearing desktop process on lock, including failed saves,
  open dialogs and unfinished Drive work. Wait for its exit before opening a
  fresh locked window; pass only encrypted recovery and source metadata.
- Recover unsaved changes using a newly entered master password, without
  retaining a live Vault or password. If recovery encryption itself fails,
  release the session and warn that pending changes were discarded.
- Disable process core dumps while preserving GNOME portal integration.
- Remove credential-capturing clipboard timers and clear aliased XML elements.

# 0.5.1

- Revoke credential access immediately when locking, including failed local
  saves and unfinished Drive synchronization. Preserve pending edits behind
  master-password reauthentication and block stale dialogs and shortcuts.
- Clear credential views, open editors and the tracked clipboard when locking.
- Keep an explicit desktop portal appearance preference authoritative over
  conflicting Qt notifications; restore Qt fallback when the preference clears.

# 0.5.0

- Follow the system's light/dark appearance automatically, including changes
  while the app is open, using Qt notifications and the Linux Settings portal.
- Adapt credential groups, lists, avatars, icons, focus states, setup, unlock and
  editor dialogs together. Switching appearance keeps open edits intact.

# 0.4.1

- Click a username or password value directly to copy it, including masked
  password dots. Keyboard activation with Enter/Space is also supported.
- Keep the existing copy feedback and 30-second clipboard clearing; copying a
  masked password does not reveal it on screen.

# 0.4.0

- Adopt the chosen light Adwaita three-pane layout from design 1, with the
  Atkinson Hyperlegible Next and Spectral typography from design 2.
- Bundle the fonts and their licenses for consistent offline rendering; retain
  Red Hat Mono for password values.
- Compact entry rows, grouped credential details, clearer copy actions, favorite
  switching, and keyboard shortcuts for search, add, edit and lock.
- Apply the same palette and fonts to setup, unlock and the entry editor.
- Retain the existing encryption, encrypted backups and Google Drive sync logic.

# 0.3.0

- Native Qt interface with searchable entries, labels/favorites, masked details,
  copy actions and a simple password editor.
- Local encrypted autosave, backups and persistent offline working copies.
- Automatic discovery in the official Safe private Drive folder, using a fresh
  explicitly authorized login and locally supplied original-client configuration.
- Automatic checks on unlock and after edits; conditional uploads reject remote
  conflicts and preserve local pending work.
- Immediate visual lock during network operations, clipboard clearing and idle lock.

This remains a smaller, experimental interoperability client. Vendor application
binaries and client configuration values are not distributed with the source.
