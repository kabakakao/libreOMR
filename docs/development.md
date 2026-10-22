# Entwicklung und technische Hintergründe

Diese Dokumentation enthält Architektur, Erkennungsgrenzen und lokale
Regressionstests. Alle Shell-Beispiele werden im Projekt-Hauptordner ausgeführt.
Die Anleitung zur Nutzung steht in der [README](../README.md).

## CI-Laufzeiten

Die Bild- und Vorlagenvergleiche benötigen auch optimierte Abhängigkeiten.
Der Workflow verwendet deshalb das Profil `ci` mit `opt-level = 2` für Tests
und Builds. Im eigenen Projektcode bleiben Debug-Assertions und
Überlaufprüfungen aktiv; Abhängigkeiten verwenden dafür die Release-Einstellungen.
Reduzierte Debug-Informationen und deaktivierte inkrementelle Kompilierung begrenzen
Build- und Cache-Größe. Lokal lässt sich derselbe Lauf so ausführen:

```sh
cargo test --profile ci --locked
cargo build --profile ci --locked
```

Alle Rust-Tests und Binärdateien werden weiterhin auf Linux, Windows und
macOS geprüft. Formatierung, Clippy und die Python-Tests laufen einmal im
separaten Linux-Job; dort wird auch der Benchmark ohne Desktop-Features geprüft.
Abhängigkeiten werden pro Job/Plattform zwischengespeichert, auch bei einem
fehlgeschlagenen Lauf. Der erste Build mit einem neuen Profil benötigt einen
neuen Cache.

Die Matrix trennt das Kompilieren der Tests von ihrer Ausführung, damit die
Laufzeit beider Schritte erkennbar ist. Für Kompilierung und Testausführung
gelten Grenzen von 30 bzw. 10 Minuten; der gesamte Matrix-Job ist auf 45 Minuten
begrenzt. Neue Läufe derselben Git-Referenz brechen ältere Läufe ab.

## Gemeinfreies Regressionsbeispiel

Die [Partita BWV 1013](../examples/public-domain/README.md) ist zugleich ein
öffentlicher Regressionstest für die Schlüsselerkennung. Der Notensatz enthält
schmalere Violinschlüssel als die ursprüngliche Bravura-Vorlage. Der Vergleich
berücksichtigt deshalb weitere Breiten und Höhen. Wenn die normale Suche
scheitert, folgt ein größerer Ausschnitt mit einer Linienbereinigung, die
dünne Schlüsselkonturen an den Notenlinien besser erhält. Die Schwellen für
Trefferqualität und die Unterscheidung von Violin- und Bassschlüsseln bleiben
erhalten.

`cargo test --locked public_bach_partita` prüft alle 64 Schlüssel auf den fünf
Seiten sowie an der ersten Notenzeile, dass Schlüssel und C-Takt nicht als
Noten oder Pausen erscheinen und die erste echte Sechzehntelnote erhalten bleibt.
Diese Tests benötigen keine privaten Scans. Wie die Aufnahmen für die README
entstehen, steht unter [Screenshots](../assets/screenshots/README.md).

## Architektur

- `src/model.rs`: Noten, Pausen, Taktgrenzen, taktweise Vorzeichengeltung,
   Systemvorzeichnung, Boegen/Gabeln und versioniertes Projektformat.
- `src/rhythm.rs`: taktweise Summierung der Notenwerte und Vergleich mit der
   geltenden Taktart fuer die Validitaetsanzeige im Scan und in der Vorschau.
- `src/recognition.rs`: vorsichtige Scan-Ausrichtung, Binarisierung und Zusammenhangskomponenten mit
   `imageproc`, Linien-/Hilfslinienpruefung, Taktstriche, Hals-/Balkenauswertung, Punktierung
   und ein generiertes Demobild.
- `src/symbols.rs`: Kopfvorlagen und Hohlraumpruefung sowie skaliertes Template
    Matching fuer Systemangaben, Faehnchen und Pausen mit `imageproc`;
   SMuFL-Vorlagen werden aus der mitgelieferten Bravura-Schrift mit `ab_glyph` erzeugt.
- `src/app.rs`: native `eframe/egui`-Oberflaeche, Hintergrundverarbeitung,
  Bild-Overlay, Inspektor, Undo/Redo und lokale Projektdateien.
- `src/spans.rs`: geometrische Vorschlaege fuer Crescendo-/Decrescendo-Gabeln
   und Binde-/Halteboegen anhand von Bildkonturen und Notenpaaren.
- `src/preview.rs`: ausblendbare, zeilenbezogene Notenvorschau aus dem aktuellen
   Modell mit Bravura-Symbolen, Scrollbereich und Noten-/Pausenauswahl;
   Vorzeichen als freigestellte Schriftglyphen aus Bravura ohne Hintergrund;
   gemeinsame Bogen-/Gabeldarstellung fuer Vorschau und Scan-Overlay.
- `src/document_import.rs`: Seitenermittlung und seitenweiser PDF-/TIFF-/Bildimport;
   PDF-Rasterung mit `hayro`, 300 dpi, weissem Hintergrund und Bildgroessenpruefung.
   PDF-Seiten teilen sich ein eingelesenes Dokument, ohne alle Seiten vorab zu rastern.
- `src/tiff_import.rs`: Auflistung und Auswahl der TIFF-Bildverzeichnisse;
   Fax3-Fallback mit `hayro-ccitt` und dem Metadatenparser von `tiff`.
   Andere TIFF-Kompressionen bleiben beim `image`-Decoder, auch auf spaeteren Seiten.
- `src/image_limits.rs`: gemeinsame Bildgroessenlimits fuer Standard- und Fax3-Import
   sowie das Wiederherstellen eingebetteter Projektbilder.
- `src/main.rs`: Einstiegspunkt der Desktop-App.

Die Trennung erlaubt spaeter, die heuristische Erkennung durch einen
Rust-kompatiblen Modellrunner (z. B. ONNX) zu ersetzen, ohne den Korrektureditor
neu zu schreiben. Dafuer muessen Modell, Symbolvokabular, Lizenz und Datensatz
zuerst ausgewertet werden. Es sind derzeit keine Modellgewichte enthalten.

Die unveraenderte Bravura-Schrift stammt aus
[steinbergmedia/bravura, Commit 37b1943](https://github.com/steinbergmedia/bravura/tree/37b194378b710cc40e406ab6c4b07608bb9548ae)
und steht unter SIL Open Font License 1.1. Die mitgelieferte Lizenz steht in
[assets/Bravura-LICENSE.txt](../assets/Bravura-LICENSE.txt) und muss bei Weitergabe
der Schrift bzw. eines Pakets mit dieser Schrift mitgeliefert werden. Fuer die
Vorlagen werden keine Ausschnitte einer Benutzerpartitur gespeichert.

## Grenzen und naechste Schritte

Die Baseline kann mit schiefen Scans, kurzen Systemen, dicht ueberlagerten
Balken, Akkorden, beschaedigten/offenen Ringkonturen, Text und Symbolen scheitern
oder falsche Noten erzeugen. Getrennte Koepfe an einem gemeinsamen Hals werden
gesucht, aber noch nicht als Akkord oder mehreren Stimmen zugeordnet.
Die Vorschau zeigt den Modellstand, keine validierte Neusetzung der Partitur.
Abstaende werden fuer die Lesbarkeit angepasst; kurze Noten von 1/8 bis 1/64 erscheinen
mit Faehnchen statt rekonstruierter Balkengruppen. Stimmen, rhythmische
Vollstaendigkeit und eine vollstaendige Notensatz-/Umbruchlogik fehlen weiterhin.
Sie setzt mindestens etwa 45 % Bildbreite fuer Notenlinien voraus. Die neue
Schluesselerkennung ist auf den Anfang jedes Systems begrenzt und kann
bei anderen Notenschriften oder beschaedigten Zeichen scheitern. Zu schwache
oder mehrdeutige Vorlagentreffer bleiben unbekannt; Fehlklassifikationen bleiben
trotzdem moeglich. Die Vorschau und der MusicXML-Export fuehren die Taktart
bis zur naechsten gedruckten oder manuell ergaenzten Taktangabe fort. Das Modell
behandelt die Systeme als fortlaufende Stimme; eine Instrument-/Stimmen-Zuordnung
fehlt. Vorzeichnungen werden nicht zwischen Systemen uebertragen. Eine leere
Vorzeichnung wird nur bei einer beobachteten freien Luecke zwischen erkanntem
Schluessel und Taktzeichen bestaetigt; sonst bleibt sie unbekannt und muss
gegebenenfalls manuell gesetzt werden.
Pausenerkennung ist auf gerade, weitgehend isolierte Standardformen beschraenkt;
Kurze Pausen werden auch bis zu zwei Notenabstaende tiefer gesucht, etwa unter
einem Balken, und in mehreren Druckgroessen geprueft. Vollstaendige Pausen mit
mehr Haekchen haben bei vergleichbarer Trefferqualitaet Vorrang vor Teiltreffern
mit weniger Haekchen. Dadurch werden Sechzehntelpausen nicht auf ihren
Achtel-Teil reduziert.
Punktierte Pausen und eine mehrstimmige rhythmische Zuordnung sind nicht modelliert.
Mehrtaktpausen benoetigen einen mittigen langen Balken, zwei Endstriche und
eine lesbare, zentrierte Zahl von 2 bis 99 darueber. Andere Formen, fehlende
Zahlen und eine automatische zeitliche Aufteilung der pausierten Takte
sind nicht implementiert. Die Zahlenerkennung ist heuristisch und korrigierbar.
Sehr kurze Scanluecken im Balken (bis 0,08 Notenabstaende) werden ueberbrueckt;
groessere Unterbrechungen bleiben ausgeschlossen. Pausenzahlen benoetigen eine
vollstaendige Kontur; Mehrlochformen werden zusaetzlich ueber ihre Innenflaechen
geprueft. Die Mindestqualitaet des Vorlagenvergleichs bleibt unveraendert.
Die automatische Triolenerkennung ist auf pausenfreie Dreiergruppen gleicher
Grundwerte mit gemeinsamem Balken und sichtbarer 3 beschraenkt. Triolenklammern,
gemischte Werte, punktierte Triolentoene und sonstige N-Tolen werden nicht
automatisch zugeordnet. Werte kuerzer als Vierundsechzigstel und die
zeitliche Takt-/Stimmenzuordnung sind weiterhin nicht implementiert.
Gabeln und Boegen werden heuristisch und noch nicht vollstaendig erkannt.
Boegen zwischen unmittelbar benachbarten, gleich hohen Noten mit gleichem
Vorzeichen und ohne Pause dazwischen werden als Haltebogen vorgeschlagen;
die musikalisch mehrdeutige Typzuordnung bleibt manuell korrigierbar.
Verschachtelte, stark ueberlagerte und zeilenuebergreifende Boegen sowie
Stimmenzuordnung sind nicht gezielt unterstuetzt. Text wie `cresc.`/`dim.`
wird nicht gelesen. Die Symbole speichern Bildkoordinaten, keine festen
Notenverknuepfungen: Nach Notenkorrekturen ihre Endpunkte ebenfalls pruefen.
Eine Haltebogen-Wiedergabe oder rhythmische Zusammenfassung ist nicht implementiert.
Artikulationen werden anhand von Form, Groesse und Bravura-Vorlagen in der
Nahe einer Notenachse gesucht. Getrennte Bestandteile von Fermaten und
kombinierten Zeichen werden gemeinsam verglichen. Die Linienbereinigung
prueft echte durchgehende Linien, damit kopfnahe Zeichen erhalten bleiben.
Ueberlagerungen mit Haelsen oder Boegen, sehr nahe Taktstriche, Text und
abweichende Notenschriften koennen weiterhin zu fehlenden oder falschen
Vorschlaegen fuehren. Pro Note gibt es eine Artikulationsauswahl; diese enthaelt
auch die oben genannten Kombinationen. Eine Stimmenzuordnung fehlt.
Dynamikbuchstaben muessen weitgehend isoliert ober- oder unterhalb des Systems
stehen (Suchbereich sechs Notenabstaende). Zusammenhaengende Buchstabengruppen
werden als Ganzes verglichen, um etwa pp nicht auf p zu verkuerzen. Textzusatz,
Kollisionen, starke Beschaedigungen und andere Schriften koennen die Erkennung
verhindern. Die Zuordnung zwischen zwei Systemen beruecksichtigt den Abstand zum
Systemrand mit Vorrang fuer die uebliche Position unterhalb eines Systems. Die Vorschlaege bleiben ungeprueft. MusicXML ordnet Dynamikangaben
anhand ihrer horizontalen Position im bestehenden Einzelstimmenmodell ein;
eine unabhaengige rhythmische Zeitrekonstruktion entsteht dadurch nicht.
Schluesselwechsel innerhalb einer Zeile fehlen noch. Gedruckte Taktwechsel
werden nach erkannten Taktstrichen sowie am Zeilenanfang gesucht. Unterstuetzt
sind C, Alla breve und Zahlentakte mit Zaehlern von 1 bis 32 und Nennern
1, 2, 4, 8, 16 oder 32. Enge Doppelstriche neben einer Taktangabe werden bei
der Randpruefung beruecksichtigt; weitere Ziffern bleiben ausgeschlossen.
Suchanker werden auch geprueft, wenn eine Taktziffer zunaechst wie eine Note
wirkt. Schwache Schluesseltreffer werden zusaetzlich an der lokalen Lage der
Notenlinien ausgerichtet. Leicht gekruemmte Zeilenenden werden ueber mehrere
Linien verfolgt, damit dort angekuendigte Taktwechsel nicht abgeschnitten werden.
Innere Vorzeichnungswechsel werden an erkannten
Taktgrenzen als kompakte, geordnete Gruppen mit mindestens zwei Zeichen
vorgeschlagen. Unterstuetzt sind Bes, Kreuze und passende vorangestellte
Aufloesungsgruppen, auch fuer den Wechsel zu einer Vorzeichnung ohne Zeichen.
Alle Zeichen muessen zu den Schluesselpositionen passen; Qualitaetsschwellen
werden dafuer nicht abgesenkt. Einzelzeichen, unvollstaendige Gruppen,
stark versetzter Druck und Wechsel ohne erkannte Grenze koennen fehlen.
Einzelne Notenvorzeichen sollen nicht zu Tonartwechseln werden.
Wechsel gelten nur im jeweiligen System; eine automatische Uebernahme in
folgende Systeme ohne gedruckte Vorzeichnung ist nicht implementiert.
Taktgrenzen benoetigen weitgehend durchgehende, gerade senkrechte Striche
ueber die gesamte Systemhoehe. Unterbrochene, kurze, schraege oder sehr nahe
an Noten liegende Striche koennen uebersehen werden und sind manuell korrigierbar.
Die Typzuordnung ist heuristisch: Wiederholungen benoetigen zwei kleine,
ausgerichtete Punkte an den mittleren Zwischenraeumen; ein einzelner Punkt
genuegt nicht. Gleich starke Doppelstriche und duenn/dicke Schlussstriche
werden ueber ihre Strichbreiten unterschieden. Beschaedigte Punkte oder
uneindeutige Breiten koennen zu Fehlklassifikationen fuehren. Wiederholungs-
Abspiellogik und Wiederholungsanzahl sind nicht implementiert. Segno/Coda
verwenden Bravura-Vorlagen einschliesslich aufrechter Druckvarianten.
Volten benoetigen ein langes, duennes, weitgehend waagerechtes Dach mit linkem
Abstrich und abgesetzter Beschriftung. Ziffern werden mit Musikschrift-Vorlagen
verglichen; abweichende Textschriften koennen eine manuelle Nummernkorrektur
erfordern. Ueber Zeilenumbrueche fortgesetzte Klammern und ausgeschriebene
Sprunganweisungen wie D.S. al Coda werden noch nicht rekonstruiert.
Einzelvorzeichen gelten pro System und exakter diatonischer Tonhoehe mit Oktave,
nicht fuer andere Oktaven oder Systeme. Am Systemanfang wird neu begonnen;
ueber Zeilenumbrueche geteilte Takte und widerspruechliche gleichzeitige
Vorzeichen verschiedener Stimmen werden noch nicht aufgeloest.
Doppelvorzeichen, geklammerte Warnvorzeichen und dicht versetzte Akkordvorzeichen
sind nicht gezielt unterstuetzt. Die Erkennung kann Zeichen uebersehen oder
fehlklassifizieren; alle Treffer bleiben ungeprueft.
Artikulationen sind eine Eigenschaft einer Note; Dynamikangaben sind eigene,
frei verschiebbare Bildobjekte.

Priorisierte Weiterentwicklung:

1. Reale, lizenzierte Testpartituren und messbare Erkennungsmetriken.
2. Entzerrung, Perspektivkorrektur und Layoutanalyse fuer mehrspaltige Scans.
3. Symbolerkennung mit geeignetem Modell und unsicheren Kandidaten zur Pruefung.
4. Erweitertes musikalisches Modell fuer Schluesselwechsel, Tonarten, Takte, Stimmen, Akkorde,
   mehrstimmige Pausen, semantische Notenverknuepfungen und rhythmische Plausibilitaet.
5. MusicXML-Export fuer mehrstimmige Partituren ausbauen sowie MIDI-Wiedergabe.
   Der vorhandene Einzelstimmenexport ersetzt keine Rhythmus-/Stimmenrekonstruktion.
6. Installierbare Linux-, Windows- und macOS-Pakete.

## Bestehende OMR-Engines

Recherche vom 14.10.2026: In den crates.io-Suchen nach `optical music recognition`
und `omr` wurde keine etablierte, direkt einsetzbare Rust-OMR-Engine gefunden.
Das ist keine vollstaendige Marktuebersicht und kein Nachweis, dass es keine gibt.
Die hier verwendeten Bibliotheken `imageproc` und `ab_glyph` liefern Bildverarbeitung
und Schriftrasterung, aber keine musikalische OMR-Semantik.

- [Audiveris](https://github.com/Audiveris/audiveris): etablierte OMR-Engine mit
   interaktiver Korrektur und MusicXML-Export fuer Linux, Windows und macOS;
   Java, AGPL-3.0. Eine optionale Anbindung waere moeglich, waere aber kein reines
   Rust-Backend und ist hier nicht implementiert.
- [homr](https://github.com/liebharc/homr): modellbasierte OMR-Pipeline mit
   Segmentierung, Transformer und MusicXML-Ausgabe; Python, AGPL-3.0. Die
   Bildpositionsangaben sind laut Projekt nur angenaehert. Ebenfalls keine
   direkt einbindbare Rust-Bibliothek.
- [ort](https://github.com/pykeio/ort): Rust-Schnittstelle fuer ONNX-Inferenz,
   standardmaessig mit nativer ONNX Runtime; keine OMR-Engine und keine enthaltenen
   OMR-Gewichte. Ein geeignetes Modell, dessen Lizenz, Symbolvokabular und die
   Zuordnung der Ausgabe zu editierbaren Bildpositionen muessten gesondert
   evaluiert werden.

Fuer einen leistungsfaehigen Rust-Editor ist ein austauschbares, modellbasiertes
Erkennungsbackend die naechste sinnvolle Ausbaustufe. Es werden weder Java/Python
noch ein externer Dienst oder Modellgewichte automatisch installiert.

## Entwicklung

### Private Testdaten und generierte Dateien

Eigene Notenscans und Testpartituren liegen lokal unter `local/scores/`.
Der gesamte Ordner `local/` ist in `.gitignore` ausgeschlossen und wird auch
nicht in Cargo-Pakete aufgenommen. Lokale Designnotizen liegen unter
`local/design/`. Diese Dateien werden nicht mit dem Projekt ausgeliefert.
Neue private Scans, Exporte und Screenshots ebenfalls unter `local/` ablegen.
Notenbilder und Musikexporte direkt im Projekt-Hauptordner werden zusaetzlich
von Git ignoriert.

Builds, heruntergeladene Benchmark-Datensaetze, Fehlerbilder und generierte
Berichte bleiben unter `target/`; fertige Release-Archive koennen unter
`dist/` abgelegt werden. Beide Ordner sind von Git ausgeschlossen. Die
kompakten Benchmark-Zusammenfassungen unter `benchmarks/` bleiben versioniert.
`Cargo.lock`, die benoetigten Bilder/Schrift samt Lizenz unter `assets/` und
die selbst erzeugte PDF unter `tests/fixtures/` gehoeren zum Projekt.

Die normalen Tests und die GitHub-CI benoetigen keine privaten Noten.
Fuer die optionalen Originaltests die unten gezeigten `LIBREOMR_*`-Variablen
auf die Dateien unter `local/scores/` setzen. Ein Beispiel:

```sh
LIBREOMR_TEST_IMAGE=local/scores/Cassiopeia_-_BaritoneBf-1.tif \
cargo test --locked external_note_values -- --ignored
```

### Automatische Tests

```sh
cargo test --locked
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
```

`cargo test --locked articulation` prueft die oben aufgefuehrten Artikulationen ober- und
unterhalb von Noten, mehrere Skalierungen, kopfnahe Zeichen, eindeutige
Notenzuordnung, Fremdformen und JSON-Erhalt. Der separate Test
`recognizes_staccato_without_confusing_augmentation_dots` grenzt Staccato gegen
Verlaengerungspunkte ab. Diese synthetischen Tests belegen keine allgemeine
Erkennungsquote auf Originalpartituren.

`cargo test --locked dynamic` prueft alle zwanzig Dynamikangaben ober- und
unterhalb eines Systems bei drei Skalierungen, den Ausschluss von Ziffern und
Artikulationen, die Systemzuordnung, Notenkandidaten, alte Projekte, Validierung,
JSON-Roundtrip und MusicXML einschliesslich Mehrtaktpausen. Die Oberflaechentests
pruefen Auswahl und Vorschau aller Dynamiktypen sowie Einfuegen, Verschieben,
Loeschen und Undo/Redo bei kleinem und grossem Fenster.
Die Glyphenzuordnung folgt den [SMuFL-Metadaten](https://github.com/w3c/smufl/blob/gh-pages/metadata/glyphnames.json).

Ein lokales Original-TIFF einschliesslich Erkennung pruefen, ohne die Datei als
permanenten Testdatensatz vorauszusetzen:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zum/scan.tif cargo test imports_external_tiff -- --ignored --nocapture
```

Der Test ist in normalen Testlaeufen deaktiviert, da er ein vom Benutzer
bereitgestelltes Bild benoetigt.

Fuer einen lokalen Bassschluessel-Scan mit explizitem C-Takt am ersten System
prueft eine weitere Regression die erkannten Symboltypen und Positionen sowie
den Ausschluss dieser Bereiche aus den Notenkandidaten:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zum/scan.tif cargo test recognizes_external_bass_common_time_header -- --ignored --nocapture
```

Mit dem bereitgestellten Cassiopeia-Scan werden elf Bassschluessel, ein
C-Takt und in allen elf Systemen ein Kreuz erkannt. Eine weitere lokale
Regression prueft konkrete Achtel-, Viertel- und Halbpausen und den Ausschluss
erkannter Pausen aus den Notenkandidaten:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_rests_and_signature -- --ignored --nocapture
```

Das sind Regressionen fuer diesen Scan, keine allgemeine Qualitaetsmetrik oder
Vollstaendigkeitsgarantie. Der Scan wird nicht als mitgelieferter Testdatensatz
vorausgesetzt. Synthetische Tests pruefen alle sieben Pausenwerte bei mehreren
Skalierungen, Kreuz-/B-Vorzeichnungen und den Vorrang manueller Einzelvorzeichen.

Fuer denselben Scan prueft `external_note_values` konkrete ganze, halbe,
Viertel-, Achtel- und Sechzehntelnoten, zwei Punktierungen und alle zwoelf Noten
der ersten Zeile. Die Tempo-Note oberhalb des Systems darf dort nicht als
Partiturnote erscheinen:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_note_values -- --ignored --nocapture
```

Die synthetischen Notentests decken offene/gefuellte Koepfe, beide
Halsrichtungen, mehrere Skalierungen, schraege einfache bis vierfache Balken,
Faehnchen, gemeinsame Haelse und Punktierungen im Gegensatz zu Staccato ab.
Weitere optionale Originaltests pruefen alle neun Triolennoten der zweiten
Zeile sowie genau acht kurze Noten ohne falsche halbe Noten im dritten Takt
der vierten Zeile:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_triplets_in_second_staff -- --ignored
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_short_notes_do_not_create_half_notes -- --ignored
```

Offene Koepfe benoetigen neben der hellen Innenflaeche eine passende lokale
Kopfkontur. So gelten eingeschlossene Zwischenraeume zwischen Noten, Haelsen
und Balken nicht allein wegen ihrer hellen Flaeche als halbe Noten.

`external_eighth_notes_in_sixth_staff` prueft die beiden einzeln befaehnten
Achtel der sechsten Zeile ohne zusaetzliche halbe Noten. Geschlossene
Faehnchen-Hohlraeume am Hals eines gefuellten Kopfes werden vor der
Balkenauswertung ausgeschlossen. `external_note_accidentals` prueft sechs
konkrete B-/Aufloesungszeichen in den letzten beiden Zeilen einschliesslich
JSON-Roundtrip und keine erfundenen Einzelvorzeichen in den ersten neun Zeilen:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_eighth_notes_in_sixth_staff -- --ignored
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_note_accidentals -- --ignored
```

`external_eighth_in_tenth_staff_is_not_duplicated` prueft exakt vier echte
Noten im vorletzten Takt der zehnten Zeile, insbesondere die Achtel
ohne zusaetzliche halbe Note. Die Hohlraumpruefung beruecksichtigt auch den
Halsbereich eines gefuellten Kopfes, wenn die aeussere Faehnchenkontur selbst
faelschlich als entgegengesetzt gerichteter Hals erkannt wird:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_eighth_in_tenth_staff_is_not_duplicated -- --ignored
```

Synthetische Einzelvorzeichentests pruefen Kreuz, B, Aufloesung und das Fehlen
eines Zeichens bei verschiedenen Skalierungen, Linien-/Zwischenraumpositionen
und beiden Halsrichtungen. Ein freier Rand beim Vorlagenvergleich verhindert,
dass Balken-/Halsfragmente allein als Vorzeichen gelten. Erkannte Zeichen
werden nicht als Notenkoepfe oder Balken mitgezaehlt.

`external_barlines_and_accidental_carry` prueft die Anzahl der Taktgrenzen
in allen elf Zeilen des Originalscans, konkrete Grenzen der zehnten Zeile
sowie die Weitergeltung von B und Aufloesungszeichen an nachfolgende Noten:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_barlines_and_accidental_carry -- --ignored
```

Synthetische Tests unterscheiden Taktstriche von langen Notenhaelsen und
fassen Doppelstriche bei mehreren Skalierungen zusammen. Modelltests pruefen
Taktwechsel, Oktav-/Systemtrennung, neue explizite Zeichen, geaenderte oder
entfernte Grenzen, Loeschen einer Vorzeichennote und JSON-Roundtrip.
Ein simulierter Editor-Klicktest prueft Taktstrich-Loeschen mit sofortiger
Vorzeichenaktualisierung und Undo/Redo bei 760 x 520 und 1280 x 820 Pixeln.
Die Typentests pruefen alle sechs Formen bei drei Skalierungen, einzelne
Stoerpunkte und notenkopfaehnliche Flecken als Negativfaelle. Der Originaltest
prueft zusaetzlich das Wiederholungsende am Ende des Scans. Ein weiterer Klicktest
waehlt alle Typen im kleinen und grossen Fenster und prueft jeweils
JSON-Roundtrip sowie Undo/Redo. Alte Taktstriche ohne Typfeld bleiben lesbar.

`external_double_before_38_and_both_repeats` prueft den Doppelstrich vor Takt 38
ohne falsche Note auf der Taktzahl sowie den Wiederholungsanfang vor Takt 54
und das Wiederholungsende. Abgesetzte Ziffernkoepfe ober-/unterhalb des Systems
duerfen echte Taktstriche nicht blockieren; verbundene Notenhaelse bleiben
ausgeschlossen. Die Punktpruefung beruecksichtigt auch groessere, eng am Strich
gedruckte Wiederholungspunkte, verlangt aber weiterhin zwei ausgerichtete,
runde Komponenten:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_double_before_38_and_both_repeats -- --ignored
```

`external_multimeasure_rest_at_38_is_not_a_note` prueft die Acht-Takte-Pause
ab Takt 38 ohne Notenvorschlag auf der Ziffer. Synthetische Tests pruefen
2, 3, 5, 7, 8, 9, 12 und 16 Takte bei drei Skalierungen sowie kurze/grosse
Balkenluecken und fehlende Endstriche/Zahlen.
Modell- und Editor-Tests pruefen alte Pausen ohne Taktanzahl, JSON-Roundtrip,
ungueltige Zahlen sowie die Mehrtaktpausensteuerung mit Undo/Redo bei beiden
Fenstergroessen:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_multimeasure_rest_at_38_is_not_a_note -- --ignored
```

`external_hairpins_and_bows` prueft die drei Gabeln und ausgewaehlte Boegen
im Originalscan sowie vier Zeilen ohne Bogen-/Gabelvorschlaege. Es ist kein
Nachweis vollstaendiger Erkennung aller Boegen auf der Seite. Synthetische Tests
decken alle vier Typen bei drei Skalierungen, obere/untere Boegen und Negative
mit Linien, Balken und einzelnen Schraegstrichen ab. Weitere Tests pruefen
Haltebogen-Abgrenzung, alte Projekte, JSON-Roundtrip, ungueltige Geometrie,
Editoraktionen mit Undo/Redo und die zeilenbezogene Vorschau bei beiden Fenstergroessen:

```sh
LIBREOMR_TEST_IMAGE=/pfad/zur/Cassiopeia-Datei.tif cargo test external_hairpins_and_bows -- --ignored
```

Bekannte Abweichung in diesem optionalen Test: Der Bogen der ersten Zeile bei
x ~ 908 wird als Bindebogen statt als Haltebogen klassifiziert, weil die beiden
Noten derzeit unterschiedliche Tonhoehen erhalten. Der Fehler tritt auch ohne
Dynamikerkennung auf; der Test bleibt unveraendert streng und schlaegt an dieser
Stelle fehl. Die neue Dynamik-/Artikulationserkennung korrigiert diese bestehende
Tonhoehenabweichung nicht.

Der zweite Scan `Cassiopeia_-_BassCf-1.tif` hat eigene Regressionen fuer die
tiefer gesetzte Achtelpause nach zwei Achtelnoten in Takt 30, die ausserhalb
des bisherigen Ausschnitts liegende Decrescendo-Gabel in Takt 37 und die
Dreitaktpause ab Takt 62 mit minimal unterbrochenem Balken. Weitere synthetische
Tests pruefen versetzte Achtelpausen und beide Gabelrichtungen fuenf
Notenabstaende ausserhalb des Systems. Die Gabelsuche umfasst nun sechs
Notenabstaende ober-/unterhalb; ihre Konturpruefung wurde nicht gelockert.

```sh
LIBREOMR_BASS_TEST_IMAGE=local/scores/Cassiopeia_-_BassCf-1.tif cargo test external_bass_ -- --ignored
```

Beim dritten Scan `Atlantic_Avenue_02_Oboe.tif` ragten mehrere Violinschluessel
links ueber die geschaetzte Systemkante. Der normale Headerausschnitt bleibt
unveraendert; nur bei unbekanntem Schluessel wird mit 0,75 Notenabstaenden
zusaetzlichem linken Rand erneut geprueft. Feinere Violinschluessel-Skalen
schliessen Groessenluecken, ohne die Qualitaetsschwellen abzusenken. Damit werden
alle zehn Violinschluessel, der anfaengliche 12/8-Takt und die zwei Bes erkannt;
die erste Zeile hat 17 Noten und 14 Achtelpausen ohne falsche Header-Noten.
Die fuenf Mehrtaktpausen sind als 4, 8, 8, 8 und 8 Takte abgesichert.

Zusaetzliche gefuellte Schein-Koepfe an Balken-Hals-Verbindungen werden vor
der Werteanalyse entfernt, wenn zwei entgegengesetzte Halsdeutungen und eine
lange dicke Balkenfortsetzung sie belegen. Dadurch bleiben die echten Achtel
erhalten und werden nicht durch die falschen Kopfmasken zu Vierteln. Der
Originaltest prueft drei solche Stellen sowie die zugehoerigen echten Achtel;
synthetische Tests pruefen Schluesselueberstand/12-8 bei mehreren Skalierungen
und Balkenfortsetzungen gegen duenne Linien und einzelne Koepfe.

Die vier inneren Tonartwechsel dieses Scans werden als eigene Objekte erkannt:
in System 3 zu vier Bes, System 5 zu einem B, System 6 zu vier Bes und System 8
zu zwei Bes. Wechselzeichen einschliesslich der Aufloesungsgruppen werden vor
der erneuten Notenanalyse ausgeschlossen, damit sie weder Notenkoepfe noch
gewoehnliche Einzelvorzeichen vortaeuschen. Regressionen pruefen genau diese
vier Wechsel, ihre Positionen und die folgende Vorzeichengeltung; auf den
beiden Cassiopeia-Seiten duerfen keine inneren Wechsel entstehen. Synthetische
Tests decken beide Schluessel, drei Skalierungen, B-/Kreuz-/Null-Vorzeichnungen
und Gegenbeispiele mit Einzelzeichen, falscher Reihenfolge oder grossen
Abstaenden ab. Die Tests sind keine vollstaendige Transkription der Seite;
auch eine vollstaendige musikalische Bogenverknuepfung ist damit nicht geloest.

```sh
LIBREOMR_OBOE_TEST_IMAGE=local/scores/Atlantic_Avenue_02_Oboe.tif \
LIBREOMR_SAX_TEST_IMAGE=local/scores/Atlantic_Avenue_10_AltoSax2_Es-1.tif \
cargo test external_atlantic_ -- --ignored
```

Bei `Atlantic_Avenue_10_AltoSax2_Es-1.tif` liess die leichte Schraeglage nur vier
von sieben Systemen erkennen; bei `a_Little_Concert_Suite_-_BaritoneB-1.tif`
fehlten alle elf. Die Ausrichtung stellt alle sieben beziehungsweise elf Systeme
mit Notenvorschlaegen wieder her. Die neuen Originaltests pruefen insbesondere
die oberen Zeilen und die Anfangsangaben: Violinschluessel, 12/8 und ein Kreuz
beim Altsaxophon sowie Violinschluessel, 4/4 und ein B bei der Suite.
Synthetische Tests decken beide Drehrichtungen bei drei Groessen, passende
Notenkoordinaten, unveraenderte gerade/unvollstaendige Seiten und das pixelgenaue
Laden eingebetteter Projektbilder ab. Das ist kein Nachweis vollstaendig korrekter
Noten-, Stimmen- oder Bogenrekonstruktion dieser Seiten.

Der Originaltest fuer `A_Holy_Night_-_BassCf-1.tif` prueft alle zehn Wechsel
zwischen 2/4 und 4/4, einschliesslich der beiden Ankuendigungen am Zeilenende.
Alle zwoelf Bassschluessel muessen erkannt werden, vor ihnen darf kein
Taktstrich stehen. Die echten Doppelstriche vor den Wechseln bleiben erhalten;
Taktziffern duerfen nicht als Noten erscheinen.

Bei weit auseinanderliegenden Koepfen am gemeinsamen Hals wird die Halsrichtung
an der Ansatzseite bevorzugt. Dadurch wird der obere Kopf eines gebalkten
Oktavpaars nicht zum unteren Kopf hin verfolgt und faelschlich als Viertel
erkannt. Ein Originaltest prueft beide Koepfe aller acht Sechzehntelpaare
sowie Achtel, punktierte Werte, Viertel und Halbe. Synthetische Tests pruefen
beide Halsrichtungen und bis zu vier Balken bei drei Bildskalierungen.
Scheinbare Hohlkoepfe zwischen kurzen Teilbalken und Notenlinien werden vor
der Balkenzaehlung verworfen. Der erste Takt in System 5 von O Holy Night
wird damit korrekt als 16/16 statt 17/16 geprueft; der Originaltest prueft
auch die Akkordgruppierung und Notenwerte im MusicXML-Export.

```sh
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif cargo test external_holy_night -- --ignored
```

```sh
LIBREOMR_TEST_IMAGE=local/scores/Cassiopeia_-_BaritoneBf-1.tif \
LIBREOMR_BASS_TEST_IMAGE=local/scores/Cassiopeia_-_BassCf-1.tif \
LIBREOMR_OBOE_TEST_IMAGE=local/scores/Atlantic_Avenue_02_Oboe.tif \
LIBREOMR_SAX_TEST_IMAGE=local/scores/Atlantic_Avenue_10_AltoSax2_Es-1.tif \
LIBREOMR_SUITE_TEST_IMAGE=local/scores/a_Little_Concert_Suite_-_BaritoneB-1.tif \
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif \
LIBREOMR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
LIBREOMR_KULTTOUR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
cargo test --locked -- --include-ignored
```

`external_dynamics_and_marcato` prueft alle neun Dynamikangaben des
Cassiopeia-Bariton-Scans und das Marcato-Huetchen in Takt 37 sowie Symbolpositionen,
Notenausschluss und JSON-Erhalt. Die Holy-Night-Regression prueft zusaetzlich
mf, f, mf und ff an den vier zugehoerigen Systemen. Das ff direkt neben einem
Akzent der naechsten Zeile bleibt eine eigenstaendige Buchstabengruppe.

```sh
LIBREOMR_TEST_IMAGE=local/scores/Cassiopeia_-_BaritoneBf-1.tif cargo test external_dynamics_and_marcato -- --ignored
```

Die sechs Originalscans ergaenzen die synthetischen Tests, sind aber kein
Nachweis fehlerfreier Erkennung beliebiger Partituren.

Die Regressionstests fuer kurze Notenwerte pruefen 1/32 und 1/64 mit einzelnen
Faehnchen und schraegen Balken in beiden Halsrichtungen. Pausentests decken
1/16, 1/32 und 1/64 bei drei Bildskalierungen, drei Druckgroessen und zwei
vertikalen Positionen ab. MusicXML verwendet 192 Einheiten pro Viertelnote,
damit auch doppelt punktierte 1/64-Triolen ohne Rundungsverlust exportiert werden.
Bei manuell zugewiesenen n-Tolen wird die Zeitaufloesung anhand der Verhaeltnisse
exakt erweitert; auch Septolen werden ohne Rundungsfehler gezaehlt und exportiert.
Modell-, Rhythmus-, Export- und Oberflaechentests pruefen die neuen Verhaeltnisse,
Vorschlagsnoten ohne Taktdauer, unabhaengige Kopfgroessen, alte Projekte und Undo/Redo.

Taktwechseltests pruefen C, Alla breve, 3/4, 6/8, 5/4, 7/8 und 12/8 bei drei
Bildskalierungen, unvollstaendige oder versetzte Angaben, die Auswahl in der
Vorschau, Projektrundlauf, Undo/Redo und angepasste MusicXML-Taktlaengen ueber
Zeilenumbrueche hinweg. Der Oberflaechentest prueft auch lange Bildnamen in
der oberen Leiste bei schmalen und breiten Fenstern.

Optionaler nativer Fenstercheck mit Screenshot (grafische Sitzung erforderlich):
Die Vorschautests pruefen Musikglyphen, Noten-/Pausenauswahl, aktive Zeile,
Live-Aenderungen, Undo/Redo, Aus-/Einblenden und leere Ergebnisse bei
760 x 520 und 1280 x 820 Pixeln mit simulierten egui-Eingaben.
Eine Pixelpruefung kontrolliert ausserdem den sichtbaren Halsanschluss aller
acht Fahnen fuer Notenwerte von 1/8 bis 1/64, die native Rastergroesse und die Ausrichtung
von Halslinien bei einfacher, 1,5-facher und doppelter Pixeldichte.
Weitere Vorschautests pruefen die Halsrichtung auf und neben der Mittellinie
im Bass-/Violinschluessel sowie identische Schriftproportionen fuer Koepfe,
Fahnen und Vorzeichen bei diesen drei Pixeldichten.
Musikglyphen werden aus den Bravura-Schriftkonturen mit tiny-skia als
transparente Masken in nativer Displayaufloesung gerendert. Das umgeht
Rasterfehler von ab_glyph_rasterizer bei bestimmten gebrochenen Groessen.
Eine Regression prueft Transparenz und Deckkraft der tatsaechlichen Pixel,
einschliesslich der fehlerausloesenden Groesse aus der ersten Zeile von
`80er_KULTtour_2_-_Bass2Cf.pdf`. Ein optionaler Originaltest prueft auch den
Wechsel zur zweiten Zeile und zurueck:

```sh
LIBREOMR_TEST_PDF=/pfad/zu/80er_KULTtour_2_-_Bass2Cf.pdf cargo test --locked external_pdf_first_staff_accidentals -- --ignored
```

```sh
EFRAME_SCREENSHOT_TO=target/native-smoke.png cargo run --locked --features ui-screenshot
LIBREOMR_COMPACT_SCREENSHOT=1 EFRAME_SCREENSHOT_TO=target/native-compact.png cargo run --locked --features ui-screenshot
LIBREOMR_SCREENSHOT_THEME=dark EFRAME_SCREENSHOT_TO=target/native-dark.png cargo run --locked --features ui-screenshot
LIBREOMR_SCREENSHOT_IMAGE=/pfad/zum/scan.tif EFRAME_SCREENSHOT_TO=target/original-scan.png cargo run --locked --features ui-screenshot
LIBREOMR_PREVIEW_ACCIDENTALS_SCREENSHOT=1 LIBREOMR_SCREENSHOT_THEME=dark EFRAME_SCREENSHOT_TO=target/preview-accidentals.png cargo run --locked --features ui-screenshot
LIBREOMR_PROBLEMS_SCREENSHOT=1 EFRAME_SCREENSHOT_TO=target/export-problems.png cargo run --locked --features ui-screenshot
```

Der eingebaute Screenshot-Modus von `eframe` beendet das Programm nach den
ersten Renderdurchlaeufen. Das Feature `ui-screenshot` erkennt die Demo vor
dem ersten Bild synchron, damit Korrekturwerkzeuge und Vorschau sichtbar sind.
Der Check prueft das native Rendern, ersetzt aber keine
interaktiven Tests von Dateidialogen, Mausaktionen und Korrekturen.

Die GitHub-Actions-Konfiguration fuehrt Tests, Clippy und Builds auf Linux,
Windows und macOS aus. Die Plattformunterstuetzung basiert auf den verwendeten
Rust-Bibliotheken; ein lokaler macOS-Build alleine bestaetigt nicht die
Lauffaehigkeit auf den anderen Plattformen. Interaktive GUI-Abnahme bleibt
zusaetzlich erforderlich.

Die Tests fuer Segno/Coda und Volten pruefen drei Skalierungen, offene und
geschlossene Klammern, Notenausschluss, Projektrundlauf, Export und Auswahl
in beiden Vorschaugroessen. Der Originaltest prueft die beiden Segnos und
beiden Codas in O Holy Night; ein Negativtest verwirft die eingerahmten
Proben-/Taktzahlen in Cassiopeia und A Little Concert Suite. Volten sind
bisher mit synthetischen Bildern abgesichert.

```sh
LIBREOMR_HOLY_NIGHT_TEST_IMAGE=local/scores/A_Holy_Night_-_BassCf-1.tif \
cargo test --locked external_repeat_marks -- --ignored
```

Fuer den Negativtest zusaetzlich `LIBREOMR_TEST_IMAGE` und
`LIBREOMR_SUITE_TEST_IMAGE` wie oben setzen.

PDF-/TIFF-Importtests (`cargo test --locked document_import`) pruefen eine selbst
erzeugte zweiseitige PDF mit unterschiedlichen Seitengroessen, Beschnitt und
Drehung, mehrseitige TIFFs mit verschiedenen Farbtiefen, Transparenz und
Kompressionen sowie BigTIFF, beide TIFF-Bytefolgen und mehrseitiges Fax3.
Weitere Tests pruefen Seitenermittlung waehrend laufender Analyse, Reiter- und
Warteschlangenreihenfolge, seitenbezogene Wiederholung nach Fehlern sowie
Projektrundlauf und gemeinsamen MusicXML-Export nach dem Verschieben der Reiter.

Beim PDF `80er_KULTtour_2_-_Bass2Cf.pdf` wurde die linke Systemkante auf
Seite 2 innerhalb des Bassschluessels angesetzt. Nach erfolglosen normalen
Schluesselsuchen wird die nahe linke Kante anhand von mindestens vier der
fuenf Notenlinien mit vertikaler Toleranz erneut gesucht. Die zusaetzliche
Suche ist auf drei Notenabstaende begrenzt; die Geometrie fuer die
Notenerkennung bleibt erhalten. Synthetische Tests pruefen stark
angeschnittene Bass- und Violinschluessel bei drei Skalierungen.
Der optionale PDF-Test prueft alle 43 Zeilenanfaenge auf
den vier Seiten einschliesslich der Wechsel zwischen drei Bes und einem B:

```sh
LIBREOMR_KULTTOUR_TEST_PDF=local/scores/80er_KULTtour_2_-_Bass2Cf.pdf \
cargo test --release --locked --no-default-features external_kulttour_headers_on_all_pages -- --ignored
```
