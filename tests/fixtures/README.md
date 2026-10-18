# Öffentliche Testdaten

`upstream-demo.db` ist die öffentliche Beispieldatei von Max Schmitts
[golang-safe-in-cloud](https://github.com/mxschmitt/golang-safe-in-cloud),
Referenzstand `8819e485adaa349c404735509dae0540534b6ac3`.
Der ursprüngliche MIT-Lizenztext bleibt in `THIRD_PARTY_LICENSE` erhalten.

`python-v1-unicode.bin`, `python-v2-unicode.bin` und `interop.xml` sind neu
erzeugte, vollständig fiktive Migrationsfixtures. Beide verschlüsselten Dateien
wurden vom bisherigen Python-Writer mit dem öffentlichen Testpasswort
`päss🔒-Interop` geschrieben. Der Rust-Reader muss sie auf exakt dieselben
XML-Bytes entschlüsseln. Die Dateien enthalten keine Benutzerdaten.

`examples/format-interop.rs` erzeugt entsprechende Rust-Writer-Dateien zur
unabhängigen Prüfung mit dem bisherigen Reader. Bei der Migration wurden beide
Richtungen geprüft; daraus folgt kein vollständiger Nachweis sämtlicher
Features der proprietären Windows-Anwendung.
