# README-Screenshots

Echte Aufnahmen der nativen libreOMR-Oberfläche, erstellt am 22. Oktober 2026.
Alle drei Bilder zeigen die erste Seite der unveränderten
[gemeinfreien Bach-Partitur](../../examples/public-domain/README.md).
Die Erkennungsvorschläge wurden für die Aufnahmen nicht musikalisch korrigiert.
In der Korrekturansicht sind zwei Notenvorschläge zur Demonstration als geprüft
markiert und der zweite ausgewählt.

Die Aufnahmen lassen sich im Projekt-Hauptordner mit einer grafischen Sitzung
und dem eingebauten Screenshot-Modus erstellen. Dieser beendet die Anwendung
nach der Aufnahme automatisch:

```sh
cargo build --locked --features ui-screenshot --bin libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_THEME=light \
EFRAME_SCREENSHOT_TO=assets/screenshots/overview.png \
target/debug/libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_THEME=light \
LIBREOMR_REVIEW_SCREENSHOT=details \
EFRAME_SCREENSHOT_TO=assets/screenshots/correction.png \
target/debug/libreomr

LIBREOMR_SCREENSHOT_IMAGE=examples/public-domain/bach-bwv1013.pdf \
LIBREOMR_SCREENSHOT_THEME=dark \
EFRAME_SCREENSHOT_TO=assets/screenshots/dark.png \
target/debug/libreomr
```

Die Befehle verwenden die Shell-Syntax von macOS/Linux. Unter Windows heißt
die ausführbare Datei `target/debug/libreomr.exe`; Umgebungsvariablen müssen
in der Syntax der verwendeten Shell gesetzt werden.
