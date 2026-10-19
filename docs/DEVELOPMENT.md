# Entwicklung und Betrieb

Die Anwendung besteht aus Rust und GTK4. Python, Qt, Electron und eine WebView
sind für den Start nicht erforderlich. Die bisherigen verschlüsselten lokalen
Arbeitskopien und die separat gespeicherte Google-Anmeldung bleiben kompatibel.

## Voraussetzungen

- Rust ab **1.92**, Cargo, rustfmt und Clippy; `rust-toolchain.toml` nutzt stable.
- GTK4 ab **4.8**, Fontconfig und die üblichen C-Buildwerkzeuge.

Fedora:

```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel fontconfig-devel
```

Ubuntu 24.04 / Debian mit GTK4 ab 4.8:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libfontconfig1-dev
```

Rust kann über [rustup](https://rustup.rs/) installiert werden.

## Bauen und starten

```sh
git clone https://github.com/vassiljevnikita-sudo/safeincloud-linux.git
cd safeincloud-linux
cargo build --locked --release
cargo run --locked --release
```

Eine lokale Datei öffnen:

```sh
target/release/safeincloud-linux --vault /pfad/zu/SafeInCloud.db
```

`--demo` öffnet ausschließlich fiktive, temporäre Beispieldaten. In der Demo
können Website-Icons die öffentlichen Beispiel-Websites kontaktieren; die
automatisierte Prüfung `--smoke-test` arbeitet ohne diese Anfragen.

```sh
target/release/safeincloud-linux --demo
./scripts/install.sh
```

Das Installationsskript schreibt standardmäßig nach `~/.local`. Ein anderes
Ziel ist mit `PREFIX=/ziel ./scripts/install.sh` möglich. Das native Programm,
der Menüeintrag, die eingebetteten Originalschriftarten und die Lizenzhinweise
werden gemeinsam installiert. Der Buildpfad berücksichtigt `CARGO_TARGET_DIR`.

Auf Nikitas Laptop bleibt die Installation unter
`/data-ssd/codex-artifacts/safeincloud-linux/installed/native/`; der bestehende
App-Menüeintrag und `~/.local/bin/safeincloud-linux` zeigen auf das Rust-Programm.
Die frühere Python-Installation bleibt lokal als Rückfallkopie erhalten.

## Vollständige Abläufe

- **Entsperren:** bestehende verschlüsselte Arbeitskopie oder lokale `.db` wählen
  und das Masterpasswort lokal eingeben.
- **Bearbeiten:** Eintrag wählen, **Bearbeiten**, Doppelklick oder Strg+E;
  **Speichern** aktualisiert den gleichen verschlüsselten Eintrag samt Backup.
- **Generator:** Strg+G oder **Generieren** am Passwortfeld. Vier Modi,
  einstellbare Länge und explizites Übernehmen. „Unvergesslich“ verwendet
  standardmäßig sechs zufällige englische Wörter der EFF-Wortliste.
- **Bewertung:** lokale zxcvbn-Musterschätzung. Angezeigte Knackzeiten verwenden
  ein Modell mit 10.000 Versuchen pro Sekunde; Werte über 72 Zeichen werden
  ausdrücklich nicht bewertet.
- **Website-Icons:** direkt per HTTPS von der Website und ihren angegebenen
  Bildhosts, ohne zentralen Favicon-Dienst. Gespeicherte URL-Zugangsdaten,
  Pfade und Abfrageparameter werden nicht weitergegeben. SVG-only-Icons und
  nicht erreichbare Websites behalten den Buchstaben als Platzhalter.
- **Sperren:** Strg+L, **Sperren** oder fünf Minuten ohne Eingabe. Die Anwendung
  ersetzt den gesamten Sitzungsprozess, auch laufende Netzwerkarbeit. Ausstehende
  Änderungen werden ausschließlich als verschlüsselte Wiederherstellung über
  einen anonymen Dateideskriptor übernommen und erfordern erneut das Passwort.
- **Kopieren:** die eigene Zwischenablage wird nach 30 Sekunden oder beim Sperren
  geleert; inzwischen von anderen Anwendungen kopierte Inhalte bleiben erhalten.

## Google Drive

Ohne Dateiparameter öffnet die App eine bekannte verschlüsselte Arbeitskopie
auch offline. Nach dem Entsperren wird Drive im Hintergrund geprüft. Lokale
Änderungen werden zuerst verschlüsselt gespeichert; ein Versionskonflikt führt
zu einer sichtbaren Meldung und keinem erzwungenen Überschreiben.

Für eine neue Anmeldung **Google Drive verbinden** wählen. Falls noch keine
Originaldatei hinterlegt ist, die offizielle, lokal heruntergeladene
`SafeInCloud.exe` auswählen. Die Anwendung liest ausschließlich die benötigte
Client-Konfiguration, führt die Datei nicht aus und startet eine neue
Browser-Anmeldung mit PKCE und geprüftem Loopback-Callback. Hintergründe stehen
in [GOOGLE_WINDOWS.md](GOOGLE_WINDOWS.md). Proprietäre Zugangskonfigurationen
werden nicht mit der Linux-App verteilt.

Die vorhandenen XDG-Pfade werden weiterverwendet:

- Konfiguration: `$XDG_CONFIG_HOME/safeincloud-linux/`, sonst `~/.config/...`.
- Token: `official-google-token.json`, private Berechtigungen 0600.
- Originaldateipfad: `official-client.json`; Drive-Dateiidentität: `source.json`.
- Verschlüsselte Arbeitskopien und Backups:
  `$XDG_DATA_HOME/safeincloud-linux/vaults/`, sonst `~/.local/share/...`.
- Icon-Cache: ausschließlich Sitzungs-RAM. Die lokal eingebetteten Schriftarten
  dürfen als öffentliche Assets im XDG-Cache liegen.

## Prüfungen

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo run --locked --no-default-features --example session-boundary
```

Die Kernbibliothek lässt sich ohne GTK-Entwicklungsbibliotheken prüfen:

```sh
cargo test --locked --no-default-features --lib
```

Für die native Oberfläche in einem separaten Testdisplay, zusätzlich `xvfb`
beziehungsweise Fedoras `xorg-x11-server-Xvfb` installieren:

```sh
cargo build --locked
xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
  target/debug/safeincloud-linux --smoke-test
```

Die GTK-Prüfung bedient Editor und Generator mit fiktiven Daten und prüft
Speichern, erneutes Öffnen, Kopieren und Sperren. Der separate Prozess-Test
prüft das tatsächliche `exec`: alter Heap und alter Worker verschwinden,
verschlüsselte ausstehende Änderungen benötigen erneut das Masterpasswort.
Die Drive-Tests nutzen synthetische Antworten eines lokalen Testservers,
keine echten Token oder Benutzerdateien.

Die Rust-Portierung wurde nicht an der echten Drive-Datei schreibend getestet.
Ein vollständiger Roundtrip aller möglichen XML-Features mit einer laufenden
aktuellen Windows-Anwendung ist weiterhin nicht belegt; siehe
[WRITING.md](WRITING.md) und [FORMAT.md](FORMAT.md).
