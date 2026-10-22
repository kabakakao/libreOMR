# Öffentliche Erkennungsbenchmarks

Der Benchmark verarbeitet eine eingefrorene Auswahl öffentlicher Notenbilder
ohne Desktop-Oberfläche. Er vergleicht die erkannten Objekte mit unabhängigen
Referenzannotationen und schreibt einen lokal lesbaren HTML-Bericht, JSON,
Score-Modelle und markierte Fehlerbilder. Die Erkennung selbst wird dabei
nicht verändert.

## Vorbereitete lokale Bestände

Für den ersten Lauf wurden mit Seed 42 vorbereitet:

| Datensatz | Stichprobe | Entwicklung (`dev`) | Kontrolle (`test`) |
| --- | ---: | ---: | ---: |
| PrIMuS | 500 aus 87.678 Ausschnitten | 391 | 109 |
| DoReMi v1.0 | 40 aus 5.218 Seiten | 30 | 10 |

Manifeste: `target/omr-benchmarks/primus.json` und
`target/omr-benchmarks/doremi.json`. Die ersten Berichte liegen jeweils in
`target/omr-benchmarks/primus-baseline/` und `doremi-baseline/`.
Der Kontrollbestand wird beim normalen Aufruf nicht ausgewertet.

Der erste PrIMuS-Entwicklungslauf wertete **159/391** Beispiele aus; **232**
waren außerhalb des Adapterumfangs, **0** hatten Verarbeitungsfehler.
Die Ereignis-SER betrug **21,99 %**. Ausgewählte Eigenschafts-F1-Werte:
Tonhöhe **96,28 %**, Notenwert **83,72 %**, Vorzeichnung (`key.fifths`)
**47,69 %**. Das sind getrennte Maße auf dem unterstützten Teilbestand,
keine allgemeine Erkennungsquote. Die häufigsten Ausschlussgründe waren
C-Schlüssel und `tie`-Tokens. Der DoReMi-Lauf wertete **30/30** Seiten ohne
Verarbeitungsfehler aus; die Klassen-F1 über den ausgewerteten Symbolumfang
betrug **88,47 %** (micro). Die kompakte
[Ergebnisdatei](baseline-summary.json) hält diese Messung auch außerhalb von
`target/` fest. Sie ist eine Zusammenfassung, kein Eingabereport für `--baseline`.

Die anschließenden Änderungen an der Erkennung und ihr Vergleich mit dem
zurückgehaltenen Kontrollbestand sind in
[Erkennungsoptimierung](optimization.md) dokumentiert. Adapter, Stichprobe
und Bewertungsregeln wurden dabei beibehalten.

## Herunterladen und Stichprobe einfrieren

Benötigt werden Python **3.9+** ohne zusätzliche Pakete und die Rust-Toolchain
des Projekts. Downloads und entpackte Daten bleiben standardmäßig unter
`target/omr-data/`; dieser Ordner ist nicht Teil des Quellcodes. Beide Archive
zusammen benötigen rund 366 MiB Download und entpackt mehrere GiB.

```sh
python3 scripts/prepare_benchmark.py fetch primus
python3 scripts/prepare_benchmark.py fetch doremi

python3 scripts/prepare_benchmark.py prepare primus \
  --input target/omr-data/primus \
  --output target/omr-benchmarks/primus.json --limit 500 --seed 42

python3 scripts/prepare_benchmark.py prepare doremi \
  --input target/omr-data/doremi \
  --output target/omr-benchmarks/doremi.json --limit 40 --seed 42
```

Bereits entpackte Daten lassen sich direkt mit `prepare` verwenden. Für
Camera-PrIMuS ersetzen `fetch` und `prepare` den Datensatznamen durch
`camera-primus` und verwenden einen eigenen Ausgabe-/Datenordner.
Der Camera-PrIMuS-Download wurde hier noch nicht praktisch geprüft.

Die Auswahl ist deterministisch, unabhängig von der Reihenfolge des
Dateisystems. DoReMi-Seiten werden gleichmäßig über Quellprojekte verteilt,
damit sehr große künstliche Übungsprojekte die kleine Stichprobe nicht
dominieren. Der Entwicklungs-/Kontrollsplit wird mit SHA-256 aus Seed und
Quellgruppe bestimmt (standardmäßig 20 % der Gruppen für `test`). Bei DoReMi
gehören alle Seiten desselben Projekts zusammen, bei PrIMuS dieselben
Incipit-IDs einschließlich erkannter Schriftvarianten. Das garantiert keine
Trennung verschiedener Incipits desselben musikalischen Werks.

Das Manifest speichert alle ausgewählten Dateien und ihre FNV-1a/64-Prüfsummen.
Jeder Lauf prüft diese erneut; geänderte oder fehlende Dateien ergeben einen
Verarbeitungsfehler. FNV dient der Wiederholbarkeit, nicht als kryptografische
Signatur. Einen neuen Seed nur für eine bewusst neue Stichprobe verwenden;
für Erkennungsänderungen dasselbe Manifest behalten.

DoReMi verwendet unterschiedliche Dateinamen für Bild und Seitenannotation.
`prepare` ordnet sie über den Seitensuffix zu. Der offizielle Seitenexport
entfernt musikalische `DataItem`-Angaben; das Skript ergänzt vorhandene Angaben
aus `OMR_XML` anhand der Objekt-IDs in separaten Referenzkopien neben dem
Manifest. Ohne Original-XML bleibt die räumliche Klassenauswertung möglich,
die zusätzlichen Eigenschaften bleiben unbewertet. Originaldateien bleiben
unverändert. Manifest und zugehörigen `*-references`-Ordner zusammen behalten.

## Ausführen

```sh
cargo build --release --locked --no-default-features --bin omr-benchmark

target/release/omr-benchmark \
  --manifest target/omr-benchmarks/primus.json \
  --output target/omr-benchmarks/primus-run-01

target/release/omr-benchmark \
  --manifest target/omr-benchmarks/doremi.json \
  --output target/omr-benchmarks/doremi-run-01 --overlays 10
```

Unter Windows heißt das Programm `target/release/omr-benchmark.exe`.
`cargo run --locked` startet weiterhin die Desktop-App. Mit
`--no-default-features` benötigt der Benchmark keine GUI-Abhängigkeiten.

`--split dev` ist voreingestellt. `--limit 10` verkürzt einen Probelauf,
`--split test` wertet ausdrücklich den zurückgehaltenen Kontrollbestand aus.
`--split all` ist für Bestandsprüfungen verfügbar. Entwicklungs- und
Kontrollergebnisse getrennt halten; Kontrollbilder nicht zum Anpassen der
Heuristiken benutzen.

Jeder Lauf benötigt einen neuen oder leeren Ausgabeordner. Unter anderem
entstehen:

- `index.html`: Klassen-/Eigenschaftsmetriken und aufklappbare Einzelvergleiche.
- `report.json`: alle Zuordnungen, Kennzahlen, Dateiprüfsummen, nicht unterstützte
  Beispiele, Fehlergründe und ausgeschlossene DoReMi-Klassen.
- `sample-*.score.json`: unverändertes Ergebnis der Erkennung, vor Korrekturen.
- `sample-*.png`: markierte Abweichungen, standardmäßig höchstens 20 Bilder.

Grün kennzeichnet korrekte Objekte, Rot fehlende, Magenta zusätzliche und
Orange falsch klassifizierte Objekte oder falsche Eigenschaften. PrIMuS hat
keine Referenz-Pixelkoordinaten; fehlende Ereignisse erscheinen dort in der
Vergleichsliste, ohne erfundene Position im Fehlerbild. DoReMi bietet dagegen
räumlich lokalisierte Referenzobjekte.

Die Bilddekodierung einschließlich Transparenz, Größenlimits und Fax3-TIFF
ist dieselbe wie in der App. Der Benchmark richtet Bilder standardmäßig nicht
neu aus. `--straighten` aktiviert für PrIMuS denselben Ausrichtungsschritt wie
in der Desktop-App. Für DoReMi ist dieser Schalter gesperrt, weil eine Drehung
ohne Transformation der Referenzboxen den Vergleich verfälschen würde.

## Was die Kennzahlen aussagen

**PrIMuS:** Noten, Pausen und Taktstriche werden als Sequenz verglichen.
Levenshtein-Zuordnung zählt Einfügungen, Auslassungen und Ersetzungen, ohne
dass eine einzelne zusätzliche Note alle folgenden Ereignisse verschiebt.
Die Sequenzfehlerrate **SER** ist die Summe dieser Fehler geteilt durch die
Anzahl der Referenzereignisse. Ein Ereignis ist nur vollständig richtig, wenn
auch seine annotierten Eigenschaften stimmen. SER kann bei vielen zusätzlichen
Ereignissen über 100 % liegen. Schlüssel, Vorzeichnung und Taktart werden
separat zugeordnet und gehen nicht in die Ereignis-SER ein. C und 4/4 sowie
Alla breve und 2/2 werden musikalisch gleichgesetzt.

Unterstützt werden G2-/F4-Schlüssel, gewöhnliche Noten/Pausen bis 1/64,
Noten und Pausen mit bis zu zwei Punkten und einfache Tonhöhenalterationen. Beispiele
mit C-/anderen Schlüsseln, Vorschlägen, `tie`-Tokens, `multirest`-Tokens,
nicht abbildbaren Werten oder inneren Systemangaben werden
als **nicht unterstützt** ausgewiesen. Sie zählen nicht als erfolgreiche
Erkennung und nicht zum SER-Nenner. Diese Einschränkung betrifft den Adapter,
auch wenn libreOMR manche dieser Zeichen bereits teilweise erkennen kann.

**DoReMi:** Gewöhnliche schwarze/halbe/ganze Notenköpfe, die sieben
Pausenwerte, Violin-/Bassschlüssel und zuordenbare Dynamikklassen werden
räumlich verglichen. Ein vorhergesagter Mittelpunkt muss innerhalb der
Referenzbox liegen, erweitert um 25 % der kleineren Boxkante (mindestens
2 Pixel). Die Zuordnung maximiert die Anzahl eindeutiger Paare innerhalb
derselben Symbolfamilie; Distanz ordnet die Kandidaten. Eine falsche Unterklasse
bleibt eine Fehlklassifikation, ein doppelter Treffer ein zusätzliches Objekt.
Das ist ein Diagnosemaß für die aktuelle Erkennung, kein COCO-mAP-Benchmark.

Mit ergänzten Originalmetadaten werden zusätzlich diatonische Tonhöhe,
Alteration und klingende Dauer geprüft. Die Dauer wird in 192 Einheiten pro
Viertelnote normalisiert, um Rundungen in den DoReMi-Dezimalzahlen abzufangen.
Stimmenzuordnung und Einsatzzeit werden noch nicht geprüft. Unbekannte
DoReMi-Klassen sind im Bericht gezählt; für Vorzeichen als eigene Bildobjekte,
Artikulationen, Bögen, Gabeln und numerische Taktzeichen existiert hier noch
keine räumliche Auswertung.

**Beide:** Precision/Recall/F1 werden pro Klasse und musikalischer Eigenschaft
ausgewiesen. Klassentreffer messen Erkennung und Klassifikation unabhängig
von den separat bewerteten Eigenschaften. Fehlende Objekte erzeugen auch
fehlende Eigenschaften. Nicht annotierte Eigenschaften werden nicht bewertet.
Werte ohne Nenner sind `null` beziehungsweise „—“. Fehler und nicht
unterstützte Beispiele werden stets neben der Zahl ausgewerteter Beispiele
angegeben; ein Lauf ohne ausgewertetes Beispiel gilt als fehlgeschlagen.

Die Bilder sind überwiegend gesetzte Noten. Eine gute Quote hier belegt noch
keine entsprechende Robustheit auf Fotos oder alten Scans. Die kleine erste
DoReMi-Auswahl dient der Fehlersuche, nicht einer allgemeinen Qualitätsaussage.

## Änderungen gegen einen Referenzlauf prüfen

```sh
target/release/omr-benchmark \
  --manifest target/omr-benchmarks/primus.json \
  --output target/omr-benchmarks/primus-run-02 \
  --baseline target/omr-benchmarks/primus-run-01/report.json
```

Der Vergleich akzeptiert nur dieselben Manifeste, ausgewählten Dateien,
Splits und Ausrichtungseinstellungen. Weniger auswertbare Bilder, schlechtere
Klassen-/Eigenschafts-F1-Werte und höhere SER ergeben eine Regression.
`--max-regression 0.5` erlaubt eine Verschlechterung um 0,5 Prozentpunkte.
Kleine Klassen bleiben bewusst sichtbar und können entsprechend empfindlich
reagieren. Die Vergleichsliste wird zusätzlich als `regressions.json` gespeichert.

Exitcodes: **0** abgeschlossen, **1** Regression, **2** Eingabe-/Datei-/
Verarbeitungsfehler oder kein auswertbares Beispiel. Nicht unterstützte
Beispiele allein erzeugen keinen Fehlercode, solange andere ausgewertet wurden.

## Automatische Tests

```sh
cargo test --locked
cargo test --locked --no-default-features --lib benchmark
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Die CI prüft die Offline-Logik ohne öffentliche Downloads: Sequenzzuordnung,
räumliche Mehrdeutigkeiten und doppelte Treffer, Parser, Referenzerweiterung,
reproduzierbare Auswahl, Split-Trennung, Dateiintegrität und Regressionen.
Die großen Korpora werden separat lokal ausgeführt.

Quellen: [PrIMuS und Camera-PrIMuS](https://grfia.dlsi.ua.es/primus/),
[DoReMi-Dokumentation](https://github.com/steinbergmedia/DoReMi) und
[DoReMi v1.0](https://github.com/steinbergmedia/DoReMi/releases/tag/v1.0).
