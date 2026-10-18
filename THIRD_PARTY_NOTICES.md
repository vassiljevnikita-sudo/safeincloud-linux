# Herkunft und Drittanbieterhinweise

## Max Schmitt: golang-safe-in-cloud

- Autor: Max Schmitt, Copyright (c) 2017 Max Schmitt.
- Ursprungsrepository: https://github.com/mxschmitt/golang-safe-in-cloud
- Verwendeter Referenzstand:
  https://github.com/mxschmitt/golang-safe-in-cloud/tree/8819e485adaa349c404735509dae0540534b6ac3
- Lizenz: MIT. Der vollständige ursprüngliche Lizenztext bleibt unverändert in
  [THIRD_PARTY_LICENSE](THIRD_PARTY_LICENSE) erhalten.

Die Arbeit am SafeInCloud-Dateiformat und die öffentliche verschlüsselte
Testdatei bauen auf diesem Projekt auf. Dieses Upstream-Projekt ist eine
Go-Bibliothek zur Verarbeitung des SafeInCloud-Datenformats. Es ist kein
Repository des proprietären Windows-Clients. Die Linux-Anwendung wird in Rust
mit GTK4 entwickelt.

## Offizielle Windows-Anwendung als Kompatibilitätsreferenz

- Anbieter: SafeInCloud SAS.
- Originalprodukt: https://www.safe-in-cloud.com/
- Downloadhinweise:
  https://support.safe-in-cloud.com/portal/en/kb/articles/download
- Originale Portable-Version:
  https://www.safe-in-cloud.com/download/SafeInCloud_Portable.zip

Das Dateiformat und ausgewählte Google-Drive-Abläufe wurden zusätzlich an einer
lokal bezogenen offiziellen Windows-Datei untersucht. Es wird kein öffentliches
Windows-Quellcode-Repository als Quelle behauptet. Die proprietäre Anwendung,
ihr dekompilierter Code und ihre Client-Konfiguration werden nicht mit diesem
Projekt verteilt. Referenzdateien und Fingerprints sind in
[docs/RESEARCH.md](docs/RESEARCH.md) dokumentiert.

## EFF Long Wordlist

Electronic Frontier Foundation, erstellt von Joseph Bonneau. Die unveränderte
Wortliste wird unter CC BY 4.0 verteilt. Quellen, Lizenzlink und Prüfsumme:
[Wortlistenhinweise](assets/wordlists/README.md).

## Schriftarten

Atkinson Hyperlegible Next, Spectral und Red Hat Mono werden unter der SIL Open
Font License 1.1 mitgeliefert. Die vollständigen ursprünglichen Lizenzhinweise
liegen neben den Schriftdateien; Quellen:
[Schriftartenhinweise](assets/fonts/README.md).

## Rust-Abhängigkeiten

Die verwendeten Bibliotheken sind in [Cargo.toml](Cargo.toml) deklariert; die
konkreten Versionen stehen in [Cargo.lock](Cargo.lock). Sie behalten ihre eigenen
Lizenzen und Copyright-Hinweise. Die MIT-Lizenz dieses Projekts ersetzt diese
Lizenzbedingungen nicht.
