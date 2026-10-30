# SafeInCloud Linux

Ein inoffizieller Linux-Client für den Passwortmanager [SafeInCloud](https://www.safe-in-cloud.com/),
geschrieben in Rust mit GTK4. Er öffnet einen vorhandenen verschlüsselten
SafeInCloud-Tresor, lokal oder aus Google Drive, durchsucht und bearbeitet die
Einträge und erzeugt neue Passwörter. Das Projekt ist eigenständig und steht in
keiner Verbindung zu SafeInCloud SAS.

![SafeInCloud Linux mit fiktiven Beispieldaten](docs/images/ui-preview.png)

Das Bild zeigt die Demo mit fiktiven Beispieldaten.

## Ausprobieren

Voraussetzungen: Rust ab 1.92 mit Cargo, GTK4 ab 4.8, Fontconfig und die
üblichen C-Buildwerkzeuge.

```sh
# Fedora
sudo dnf install gcc pkgconf-pkg-config gtk4-devel fontconfig-devel
# Ubuntu 24.04 / Debian mit GTK4 ab 4.8
sudo apt install build-essential pkg-config libgtk-4-dev libfontconfig1-dev
```

Bauen und die Demo starten:

```sh
git clone https://github.com/vassiljevnikita-sudo/safeincloud-linux.git
cd safeincloud-linux
cargo build --locked --release
target/release/safeincloud-linux --demo
```

`--demo` öffnet nur fiktive, temporäre Beispieldaten. Für die Website-Icons
kontaktiert die Demo die öffentlichen Beispiel-Websites. Eine eigene Datei
öffnet `target/release/safeincloud-linux --vault /pfad/zu/SafeInCloud.db`.
`./scripts/install.sh` installiert Programm und Menüeintrag nach `~/.local`.
Google Drive, Prüfungen und alle Abläufe stehen in [Entwicklung und Betrieb](docs/DEVELOPMENT.md).

## Status und Grenzen

Experimentell (Version 1.0.0). Bekannte Grenzen:

- Die Rust-Portierung wurde nicht schreibend an der echten Drive-Datei getestet.
- Ein vollständiger Roundtrip aller XML-Features mit der aktuellen
  Windows-Anwendung ist nicht belegt ([WRITING.md](docs/WRITING.md), [FORMAT.md](docs/FORMAT.md)).
  Unbekannte XML-Elemente und Anhänge bleiben erhalten, ihre Bedeutung ist
  aber nicht umgesetzt.
- Für die Google-Drive-Anmeldung braucht die App die offizielle, lokal
  heruntergeladene `SafeInCloud.exe`. Sie liest daraus nur die Client-Konfiguration
  und führt die Datei nicht aus ([GOOGLE_WINDOWS.md](docs/GOOGLE_WINDOWS.md)).
- Die Oberfläche ist deutsch.
- Kein externes Sicherheitsaudit.

## Technik

Rust (Edition 2021) mit gtk4-rs für die Oberfläche. Der Tresor bleibt im
SafeInCloud-Format (AES-CBC, PBKDF2-SHA1, zlib); die App liest und schreibt es
selbst. Google Drive läuft über OAuth mit PKCE und `reqwest` mit rustls, die
Passwortbewertung lokal über zxcvbn. CI auf GitHub Actions prüft Formatierung,
Clippy, Tests, einen GTK-Oberflächentest unter Xvfb und den Release-Build.
Mehr in der [Linux-Architektur](docs/LINUX_STACK.md) und der [Formatdokumentation](docs/FORMAT.md).

## Meine Rolle

Nikita Vassiljev ([GitHub](https://github.com/vassiljevnikita-sudo)). Ich habe
das Projekt angelegt und mit KI-Coding-Agenten gebaut. Ihre Arbeitsanweisungen
stehen in [AGENTS.md](AGENTS.md). Das Projekt ist komplett in einer Nacht gevibecodet, weil mir mein Passwortmanager auf Linux gefehlt hat. Er funktioniert wie die Windows native Version und das ist alles was für mich gezählt hat.

## Herkunft und Lizenz

Die Formatimplementierung und die öffentliche Testdatei bauen auf
[Max Schmitts golang-safe-in-cloud](https://github.com/mxschmitt/golang-safe-in-cloud)
auf ([Referenzstand 8819e485](https://github.com/mxschmitt/golang-safe-in-cloud/tree/8819e485adaa349c404735509dae0540534b6ac3),
Copyright © 2017 Max Schmitt, MIT). Der vollständige ursprüngliche Lizenztext bleibt
in [THIRD_PARTY_LICENSE](THIRD_PARTY_LICENSE) erhalten. Die proprietäre
Windows-Anwendung dient separat als Interoperabilitätsreferenz und wird nicht
mitgeliefert. Weitere Quellen und Lizenzen: [Drittanbieterhinweise](THIRD_PARTY_NOTICES.md).

Der eigene Code steht unter der MIT-Lizenz ([LICENSE](LICENSE)).

## Für Coding-Agenten

Glossar, Designphilosophie und Arbeitsprinzipien für die Agenten stehen in [AGENTS.md](AGENTS.md).
