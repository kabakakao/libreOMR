# libreOMR

<p>
  <img src="assets/libreomr-logo.png" alt="libreOMR – Musiknoten in einem offenen Scanrahmen" width="480">
</p>

libreOMR ist eine Desktop-Anwendung zur optischen Notenerkennung für Linux,
Windows und macOS. Sie wandelt Notenbilder in bearbeitbare Erkennungsvorschläge
um: Partitur laden, Noten und Symbole prüfen, Fehler korrigieren und das
Ergebnis als MusicXML weiterverwenden. Alle Dateien werden lokal verarbeitet.

**Das Projekt ist ein früher Prototyp.** Die Erkennung arbeitet mit Heuristiken
und Symbolvorlagen. Ergebnisse müssen musikalisch geprüft und bei Bedarf
korrigiert werden.

## Funktionen

- PDF, PNG, JPEG, TIFF und BMP importieren; mehrseitige PDFs und TIFFs öffnen.
- Noten, Pausen, Schlüssel, Vorzeichen und Taktangaben erkennen und bearbeiten.
- Weitere Symbole wie Artikulationen, Dynamik, Bögen und Wiederholungszeichen
  prüfen und ergänzen.
- Originalbild, erkannte Objekte und Notenvorschau direkt vergleichen.
- Korrekturen rückgängig machen und Projekte mit eingebetteten Notenbildern speichern.
- Alle Seiten gemeinsam als MusicXML 4.0 exportieren, etwa zur Weiterarbeit in MuseScore.
- Zwischen hellem und dunklem Design wechseln oder dem Systemdesign folgen.

## Einblicke

Die Screenshots zeigen die erste Seite von Bachs **Partita in a-Moll für
Soloflöte, BWV 1013**, mit dem gemeinfreien Notensatz von Jacob Shapiro aus dem
[Mutopia Project](https://www.mutopiaproject.org/cgibin/piece-info.cgi?id=1528).
Das [Beispiel-PDF](examples/public-domain/bach-bwv1013.pdf) liegt zum Ausprobieren
bei; [Quelle und Rechteangabe](examples/public-domain/README.md) sind dokumentiert.
Die dargestellten Vorschläge sind kein fertig korrigiertes Notenergebnis.

**Notenbild, erkannte Objekte und Vorschau**

![libreOMR im hellen Design mit Bachs Partita, markierten Erkennungsvorschlägen und Notenvorschau](assets/screenshots/overview.png)

**Einzelne Noten prüfen und korrigieren**

![Korrekturansicht mit ausgewählter Note, Objektvorschau und bearbeitbaren Eigenschaften](assets/screenshots/correction.png)

**Dunkles Design**

![libreOMR im dunklen Design mit heller Notenseite und Korrekturwerkzeugen](assets/screenshots/dark.png)

## Installation und Start

### Fertige Pakete

Veröffentlichte Versionen stehen unter
[GitHub Releases](https://github.com/kabakakao/libreOMR/releases) bereit:

| Plattform | Paket |
| --- | --- |
| Windows x86_64 | ZIP mit `libreomr.exe` |
| Windows ARM64 | ZIP mit nativer ARM64-Version von `libreomr.exe` |
| Linux x86_64 | TAR.GZ mit `libreomr` |
| macOS Apple Silicon | ZIP mit `libreOMR.app` |

Das passende Archiv vollständig entpacken und das Programm starten. Unter
macOS kann `libreOMR.app` in den Programme-Ordner kopiert werden. Die App ist
noch nicht signiert oder notarisiert; macOS kann deshalb die Freigabe unter
**Systemeinstellungen > Datenschutz & Sicherheit** verlangen. Intel-Macs
werden nicht unterstützt.

Unter Linux wird eine grafische X11- oder Wayland-Sitzung mit den
Systembibliotheken für XKB, Wayland und EGL benötigt. Der Dateidialog verwendet
`xdg-desktop-portal` mit dem zur Desktop-Umgebung passenden Portal-Backend.

Jedes Release enthält das öffentliche Bach-Beispiel, Lizenzhinweise und
SHA-256-Prüfsummen. Solange noch kein Release veröffentlicht ist, lässt sich
libreOMR aus dem Quellcode bauen.

### Aus dem Quellcode bauen

Zum Bauen wird eine [Rust-Toolchain](https://rustup.rs/) ab Version **1.92**
benötigt. Die folgenden Befehle im Projektordner ausführen.

Unter macOS werden außerdem die Xcode Command Line Tools benötigt, unter
Windows die MSVC-Build-Tools einschließlich Windows SDK. Unter Debian/Ubuntu
lassen sich die benötigten Systempakete so installieren:

```sh
sudo apt-get install build-essential pkg-config libxkbcommon-dev libwayland-dev libegl1-mesa-dev
```

Anwendung bauen und starten:

```sh
cargo run --release --locked
```

Alternativ nur bauen:

```sh
cargo build --release --locked
```

Das Programm liegt anschließend unter `target/release/libreomr`, unter
Windows unter `target/release/libreomr.exe`.

Ein lokales macOS-App-Bundle lässt sich so erstellen und öffnen:

```sh
sh scripts/build-macos-app.sh
open target/release/libreOMR.app
```

Das Skript setzt einen Mac mit Apple Silicon voraus.

## Nutzung

### 1. Noten öffnen

Beim Start erscheint ein generiertes Notenbeispiel. Über **Datei > Bild oeffnen**
eine oder mehrere Dateien auswählen. Zum Ausprobieren eignet sich das
mitgelieferte [Bach-Beispiel](examples/public-domain/bach-bwv1013.pdf).
PDFs und mehrseitige TIFFs erhalten für jede Seite einen eigenen Reiter.
Die Seiten werden nacheinander im Hintergrund analysiert.

Über **Datei > Seiten hinzufuegen** oder das **+** neben den Reitern weitere
Seiten ergänzen. Reiter lassen sich per Drag-and-drop umordnen; ihre Reihenfolge
bestimmt auch die Reihenfolge im Export. **Einpassen**, die Zoomtasten und die
Prozentanzeige helfen bei der Navigation im Notenbild.

### 2. Erkennung prüfen und korrigieren

Ein Objekt im Notenbild, in der Objektliste oder in der Vorschau auswählen.
Rechts unter **Korrektur** lassen sich seine Eigenschaften ändern, etwa
Tonhöhe, Vorzeichen, Notenwert, Punktierung oder Artikulation. Die
Objektvorschau zeigt das ausgewählte Zeichen; die untere Notenvorschau zeigt
den aktuellen Stand der Notenzeile.

Falsch erkannte Objekte lassen sich verschieben oder löschen. Fehlende Noten,
Pausen und weitere Symbole über das Kontextmenü im Notenbild ergänzen.
Schlüssel, Tonart und Taktart stehen in den **Systemangaben**. Änderungen
lassen sich rückgängig machen und wiederholen.

Ein geprüftes Objekt über den Kreis in der Objektliste bestätigen. **F6**
springt zum nächsten ungeprüften Objekt. Die Bestätigung dokumentiert die
eigene Prüfung; sie ersetzt keine Kontrolle der übrigen Partitur.

### 3. Takte und Exporthinweise kontrollieren

Farbig markierte Taktbereiche zeigen, ob die Summe der erfassten Notenwerte
zur Taktart passt. Ein Klick auf die **Exportpruefung** öffnet die Liste der
Probleme und ungeprüften Objekte. Ein Eintrag führt zur betroffenen Stelle.
Fehlende Angaben und widersprüchliche Dauern vor dem Export korrigieren.
Auch Auftakte können als unvollständige Takte markiert werden.

### 4. Projekt speichern

**Datei > Speichern** schreibt eine `.libreomr.json`-Datei. Sie enthält alle
Seiten als eingebettete Bilder, ihre Reihenfolge und den Korrekturstand.
Mit **Datei > Projekt oeffnen** lässt sich die Arbeit später fortsetzen;
die ursprünglichen Bilddateien werden dafür nicht mehr benötigt.

**Neu erkennen** ersetzt die bisherigen Erkennungsvorschläge und Korrekturen
der aktiven Seite. Den bisherigen Stand bei Bedarf vorher speichern.

### 5. MusicXML exportieren

**Datei > MusicXML exportieren** schreibt alle Seiten in Reiterreihenfolge in
eine `.musicxml`-Datei. Diese kann anschließend in einem Notensatzprogramm
wie MuseScore geöffnet und weiterbearbeitet werden. Speichern und Exportieren
stehen bereit, sobald alle Seiten erfolgreich analysiert wurden.

Der Export verwendet den aktuellen Stand einschließlich ungeprüfter Objekte.
Hinweise nach dem Export beachten und das Ergebnis im Zielprogramm prüfen.
Ein MusicXML-Export ersetzt nicht das Speichern des libreOMR-Projekts.

## Tastenkürzel

Unter macOS gilt **Cmd**, unter Linux und Windows **Strg**.

| Kürzel | Aktion |
| --- | --- |
| Cmd/Strg + O | Notenbilder öffnen |
| Cmd/Strg + Umschalt + O | Projekt öffnen |
| Cmd/Strg + S | Projekt speichern |
| Cmd/Strg + Z | Rückgängig |
| Cmd/Strg + Umschalt + Z | Wiederholen |
| F6 | Nächstes ungeprüftes Objekt |
| Entf / Backspace | Ausgewähltes Objekt löschen |
| Escape | Kontextmenü schließen oder Auswahl aufheben |

Bei ausgewählten Noten und Pausen setzen **1, 2, 4, 8** den ganzen, halben,
Viertel- oder Achtelwert; **6, 3, 9** stehen für 1/16, 1/32 und 1/64.
**.** setzt eine einfache, **:** eine doppelte Punktierung; **0** entfernt sie.
Bei Noten setzen **#**, **b** und **=** ein Kreuz, Be oder Auflösungszeichen.
Das **?** im Korrekturbereich zeigt die Bearbeitungskürzel an.

## Aktuelle Grenzen

libreOMR eignet sich vor allem zum Erproben des Arbeitsablaufs mit klaren,
gedruckten Einzelstimmen. Schiefe oder beschädigte Scans, enge Balkengruppen,
ungewohnte Schriften und überlagerte Symbole können zu fehlenden oder falschen
Erkennungsvorschlägen führen.

Die Notenzeilen werden beim Export als **eine fortlaufende Stimme** behandelt.
Eine vollständige Rekonstruktion mehrstimmiger Klavier- oder Orchesterpartituren,
automatische Instrumentenzuordnung und Transposition sind noch nicht enthalten.
Die Vorschau zeigt den bearbeiteten Erkennungsstand; Balkengruppen und
Seitenlayout werden nicht originalgetreu rekonstruiert. Eine fehlerfreie
Transkription beliebiger Partituren wird derzeit nicht erreicht.

## Lizenz

libreOMR steht unter der **GNU General Public License v3.0 oder später**
(`GPL-3.0-or-later`); der vollständige Text steht in [LICENSE](LICENSE).
Bei Weitergabe geänderter Versionen gelten ebenfalls die Bedingungen der GPL.

Mitgelieferte Komponenten behalten ihre eigenen Lizenzen. Die Bravura-Schrift
steht unter der [SIL Open Font License 1.1](assets/Bravura-LICENSE.txt), das
[Bach-Beispiel](examples/public-domain/README.md) ist gemeinfrei.
