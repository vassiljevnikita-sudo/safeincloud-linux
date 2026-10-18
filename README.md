# SafeInCloud Linux

## 1\. Was ist SafeInCloud Linux?

**SafeInCloud Linux ist ein unabhängiger, nativer Linux-Client für [SafeInCloud](https://www.safe-in-cloud.com/).**

Die App verbindet den verschlüsselten Tresor, die Suche, die Bearbeitung und die
Synchronisation zu einem zusammenhängenden Arbeitsablauf. Passwörter sollen schnell
auffindbar und einfach nutzbar sein, während der User die Kontrolle über seine
Daten behält.

Die Linux-Anwendung wird in **Rust mit GTK4** entwickelt. Sie ist ein eigenständiges
Community-Projekt und steht in keiner Verbindung zu SafeInCloud SAS.

![SafeInCloud Linux mit fiktiven Beispieldaten](docs/images/ui-preview.png)

## 2\. Designphilosophie

**Ein System, in dem alles zusammenpasst.**

- **Deep Integration:** Tresor, Suche, Details, Editor und Synchronisation gehören zu einem gemeinsamen Ablauf. Die App soll sich wie ein einziges System anfühlen.
- **Linux First:** Native GTK4-Widgets und die Konventionen des Linux-Desktops bilden die Grundlage der Oberfläche.
- **Interaction First:** Jede Funktion wird als vollständiger, intuitiver Nutzerfluss gedacht. Sichtbare Aktionen müssen eine klare Wirkung und verständliche Rückmeldung haben.
- **Blazing Fast:** Schnelle Reaktionen und effiziente Datenflüsse sind grundlegende Anforderungen. Konkrete Leistungsversprechen brauchen Messungen.
- **Simple Code:** Klarer, wartbarer und typsicherer Rust-Code mit möglichst wenig unnötiger Komplexität.
- **User Control:** Geheimnisse bleiben verborgen, bis der User sie benötigt. Änderungen, Synchronisation und Konflikte müssen nachvollziehbar sein; sensible oder irreversible Aktionen brauchen eine bewusste Entscheidung.

## 3\. Glossar

- **Du:** Du bist der Agent, der an diesem Projekt arbeitet. Willkommen bei SafeInCloud Linux.
- **Ich:** Ich bin Nikita, der Schaffer dieses Projekts.
- **User:** Der Benutzer der App. Seine vorhandenen Daten und gewohnten Arbeitsabläufe bestimmen die Anforderungen.
- **Subagenten:** Andere Agenten, die nach den geltenden Projektvorgaben bei abgegrenzten Aufgaben mitarbeiten können.
- **System:** SafeInCloud Linux als Ganzes: Oberfläche, Tresor, lokale Speicherung und Synchronisation.
- **Tresor:** Die verschlüsselte SafeInCloud-Datenbank mit Einträgen, Feldern und Labels.
- **Bots:** Automatisierte Prüfungen und Reviews in GitHub Pull Requests.

## 4\. Arbeitsprinzipien

- Verstehe vor Änderungen das bestehende System und die Absicht hinter der Anforderung.
- Denke in vollständigen Nutzerflüssen, nicht in isolierten Features.
- Erweitere bestehende Strukturen, statt parallele Lösungen zu schaffen.
- Bevorzuge einfache, robuste und performante Lösungen gegenüber unnötigen Abstraktionen.
- Erhalte vorhandene Tresordaten und überprüfe Änderungen mit geeigneten Tests.
- Mache Fehler, ausstehende Änderungen und Synchronisationskonflikte sichtbar.
- Optimiere immer das Gesamtsystem, nicht nur einzelne Komponenten.

**Die zentrale Frage bei jeder Änderung lautet: Macht sie SafeInCloud Linux zu einem besseren, verlässlichen Passwortmanager für Linux?**

Installation, Start und Prüfungen stehen in [Entwicklung und Betrieb](docs/DEVELOPMENT.md).
Die [Linux-Architektur](docs/LINUX_STACK.md) und die
[Formatdokumentation](docs/FORMAT.md) beschreiben die technischen Grundlagen.

Die Formatimplementierung und die öffentliche Testdatei bauen auf
[Max Schmitts golang-safe-in-cloud](https://github.com/mxschmitt/golang-safe-in-cloud)
auf ([Referenzstand 8819e485](https://github.com/mxschmitt/golang-safe-in-cloud/tree/8819e485adaa349c404735509dae0540534b6ac3),
Copyright © 2017 Max Schmitt, MIT). Der vollständige ursprüngliche Lizenztext bleibt
in [THIRD_PARTY_LICENSE](THIRD_PARTY_LICENSE) erhalten. Die proprietäre
Windows-Anwendung dient separat als Interoperabilitätsreferenz und wird nicht
mitgeliefert. Weitere Quellen und Lizenzen: [Drittanbieterhinweise](THIRD_PARTY_NOTICES.md).
