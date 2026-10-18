# Linux-Architektur

SafeInCloud Linux ist eine native Desktop-Anwendung in **Rust mit GTK4**.
GTK4 stellt die Widgets und die Einbindung in den Linux-Desktop bereit; die
Anwendung benötigt GTK4 ab Version 4.8. libadwaita ist keine Voraussetzung.

## Ein gemeinsamer Anwendungskern

Tresorverarbeitung, lokale Speicherung und Google-Drive-Synchronisation gehören
zur Rust-Anwendung. Die Oberfläche führt diese Funktionen in einem gemeinsamen
Arbeitsablauf zusammen. Der reguläre Start benötigt weder Python noch Qt,
WebView, Wine oder eine laufende Windows-Anwendung.

Der Tresor bleibt im vorhandenen verschlüsselten SafeInCloud-Dateiformat.
Die [Formatdokumentation](FORMAT.md) beschreibt die beobachtete Hülle und die
Grenzen der Interoperabilität. Das Format stammt nicht aus diesem Projekt;
die zugehörigen [Quellen und Lizenzen](../THIRD_PARTY_NOTICES.md) bleiben erhalten.

Die lokal bezogene offizielle Windows-Datei ist eine separate
Interoperabilitätsreferenz für die Google-Anmeldung. Sie ist kein Bestandteil
der Linux-Oberfläche und wird nicht mitgeliefert. Der Hintergrund ist in
[Google-/Windows-Protokollbeobachtungen](GOOGLE_WINDOWS.md) dokumentiert.

## Daten und Lebensdauer

Entschlüsselte Daten werden nur für die geöffnete Sitzung benötigt. Maskierung,
Zwischenablage, Sperren und die Behandlung ausstehender Änderungen müssen als
zusammenhängender Ablauf betrachtet werden. Rust allein garantiert keine
vollständige Entfernung aller Geheimnisse aus GUI, Betriebssystem,
Zwischenablage oder Swap.

Lokale Speicherung und Synchronisation verwenden verschlüsselte Tresordaten.
Ein erfolgreicher lokaler Schreibvorgang und ein erfolgreicher Cloud-Upload sind
unterschiedliche Zustände. Ein Konflikt darf keine fremde Änderung stillschweigend
überschreiben. Das vorhandene CBC-Dateiformat bietet keine kryptografische
Authentifizierung des gesamten Inhalts.

## Entwicklung und Überprüfung

Cargo verwaltet den Build und die Rust-Abhängigkeiten. Der CI-Workflow installiert
GTK4-Entwicklungsbibliotheken und prüft Formatierung, Clippy, Tests und einen
Release-Build. Start, Einrichtung und die konkreten Prüfkommandos stehen in
[Entwicklung und Betrieb](DEVELOPMENT.md).

Tests mit synthetischen Tresoren und gemockten Netzwerkantworten belegen nur die
jeweils geprüften Pfade. Sie ersetzen keinen Kompatibilitätsnachweis für jede
Funktion des offiziellen Clients und keinen Live-Test eines Google-Kontos.
Leistungsversprechen brauchen Messungen von Start, Entsperren, Listenaufbau und
Synchronisation auf der Zielmaschine.
