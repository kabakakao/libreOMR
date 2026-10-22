# Builds und Releases

Die normale CI erstellt herunterladbare Entwicklungsstände. Versionierte
Releases entstehen durch einen Git-Tag und bleiben zunächst Entwürfe.
Es werden Windows x86_64 und ARM64, Linux x86_64 und macOS Apple Silicon unterstützt.

## Entwicklungsstände

Nach erfolgreichen Tests und Builds lädt `Build and Test` für jede Plattform
ein Artefakt hoch. Es enthält ein natives Archiv und dessen SHA-256-Prüfsumme;
es bleibt 14 Tage im Actions-Lauf verfügbar. Diese Pakete verwenden das
optimierte Profil `ci`. Dateiname und `build-info.json` nennen den Commit.
Pull Requests erhalten ebenfalls Artefakte, benötigen dafür aber keine
Schreibrechte auf Repository-Inhalte.

Die Matrix verwendet Ubuntu 22.04, `windows-latest` für Windows x86_64,
`windows-11-arm` für Windows ARM64 und `macos-15` für Apple Silicon.
Windows ARM64 wird mit der nativen Rust-Toolchain
`stable-aarch64-pc-windows-msvc` gebaut und getestet. Beide Windows-Varianten
erhalten eigene ZIP-Archive; die Architektur steht im Dateinamen, etwa
`libreomr-0.1.0-aarch64-pc-windows-msvc.zip` für ARM64.
Der feste Ubuntu-Stand begrenzt die benötigte glibc-Version; es handelt sich
weiterhin um ein dynamisch gelinktes Programm mit Desktop-Systembibliotheken.
Es sind portable Archive, keine Installer oder vollständigen Linux-Bundles.
Das macOS-Bundle ist nicht signiert oder notarisiert.

## Einen Release vorbereiten

1. Version in `Cargo.toml` setzen, z. B. `0.1.0` oder `0.1.0-beta.1`.
2. Mit `cargo check --locked` prüfen. Nach einer Versionsänderung zunächst
   `cargo check` ausführen, damit `Cargo.lock` die neue Projektversion erhält.
3. Versionshinweise unter `docs/release-notes/<Version>.md` vorbereiten,
   z. B. `docs/release-notes/0.1.0.md`. Der Workflow prüft, dass die Datei
   vorhanden und nicht leer ist, und übernimmt sie in den Release-Entwurf.
4. Änderungen einschließlich `Cargo.lock` und Versionshinweisen committen und nach `main` pushen.
   Den erfolgreichen regulären CI-Lauf abwarten.
5. Den gewünschten Commit mit exakt passender Version markieren und pushen:

```sh
git tag -a v0.1.0 -m "libreOMR 0.1.0"
git push origin v0.1.0
```

Für eine Vorabversion beispielsweise `v0.1.0-beta.1` verwenden. Ein Tag allein
veröffentlicht noch keinen Release. Der `Release`-Workflow prüft zuerst die
Übereinstimmung mit `Cargo.toml`, danach laufen alle Qualitätsprüfungen und
Tests erneut für genau den markierten Stand. Erst dann entstehen mit
`cargo build --release --locked` die vier Plattformpakete.

Zusätzlich wird ein Quellarchiv aus den versionierten Projektdateien und den
über `Cargo.lock` fixierten Cargo-Abhängigkeiten erstellt. `cargo vendor`
legt diese unter `vendor/` ab; eine mitgelieferte Cargo-Konfiguration erlaubt
den Offline-Build mit installierter Rust-Toolchain und Systembibliotheken.
Dieses Archiv gehört zu den Binärpaketen des GPL-Releases. Private und
ignorierte Dateien werden nicht aus dem Arbeitsverzeichnis übernommen.

Nach erfolgreichen Builds erstellt der Workflow einen **GitHub-Release-Entwurf**
mit den vorbereiteten Versionshinweisen, den fünf Archiven und `SHA256SUMS`.
Tags mit einem Bindestrich werden als Vorabversion markiert. Schreibrechte
auf Repository-Inhalte besitzt nur der letzte Job zum Anlegen des Entwurfs.
Für die übrigen Jobs reichen Leserechte; ein persönlicher Token ist nicht nötig.

Den Entwurf unter **Releases** öffnen, Versionshinweise ergänzen und die
Pakete auf den Zielplattformen ausprobieren. Anschließend dort veröffentlichen.
Die Download-Links in der README führen zur Release-Übersicht.

Ein erneuter Lauf überschreibt keinen bereits angelegten Release. Falls nur
das Anlegen des Entwurfs fehlgeschlagen ist, diesen Job erneut starten. Existiert
der Entwurf bereits, dessen Dateien prüfen; für geänderten Code eine neue
Version verwenden und Tags veröffentlichter Versionen nicht verschieben.

## Paketinhalt und Lizenzen

`scripts/package_release.py` verwendet eine ausdrückliche Dateiauswahl für die
Binärpakete: Anwendung, Benchmark-Programm, Nutzungsanleitung, öffentliche
Beispielpartitur, Bilder der Anleitung sowie Lizenz- und Copyright-Hinweise.
Private Scans, lokale Exporte, Testdaten und Build-Verzeichnisse gehören
nicht dazu. `build-info.json` enthält Version, Commit, Plattform und Build-Profil.

Der Projektcode steht unter `GPL-3.0-or-later`. `LICENSE` enthält den
vollständigen GPLv3-Text, `COPYRIGHT` die Freigabe für Version 3 oder später.
Die deklarierten Lizenzen des aufgelösten Cargo-Abhängigkeitsgraphen stehen in
`THIRD_PARTY_NOTICES.md`; verfügbare Originalhinweise werden unter `licenses/`
mitgeliefert. Fehlende Texte aus Cargo-Archiven werden aus den
[ergänzenden Lizenzhinweisen](../assets/dependency-licenses/README.md) übernommen;
Quell-URLs und Prüfsummen bleiben im Paket erhalten. Bei Abhängigkeitsupdates
müssen diese versionsgebundenen Kopien gegebenenfalls ergänzt werden.
Die Liste enthält auch Build-, Test- und andere
Plattformabhängigkeiten und behauptet keine Verwendung aller Komponenten
in jedem Programm. Fonts und die Beispielpartitur behalten ihre eigenen
Lizenz- beziehungsweise Gemeinfreiheitsangaben. Im macOS-Bundle liegen die
Lizenztexte zusätzlich unter `Contents/Resources`, damit sie beim Kopieren
der App erhalten bleiben.

## Lokal überprüfen

Python 3.9 oder neuer wird zum Paketieren benötigt. Nach einem Build kann ein
lokales Paket erzeugt werden:

```sh
cargo build --profile ci --locked
python3 scripts/package_release.py --profile ci
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Die Ausgabe liegt unter dem ignorierten Ordner `dist/`. Nicht markierte Builds
tragen `dev`, die Commit-Kennung und bei lokalen Änderungen zusätzlich `dirty`
im Dateinamen. Ein offizielles Paket setzt einen sauberen Checkout, das
Release-Profil und eine passende Versionsangabe voraus:

```sh
python3 scripts/package_release.py --check-tag v0.1.0
cargo build --release --locked
python3 scripts/package_release.py --profile release --release-tag v0.1.0
python3 scripts/package_release.py --source-tag v0.1.0
```

Auf macOS wird das Apple-Silicon-App-Bundle automatisch erstellt. Für ein
Bundle ohne Archiv bleibt `sh scripts/build-macos-app.sh` verfügbar.
