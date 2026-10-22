# Erkennungsoptimierung vom 19. Oktober 2026

Die Änderungen betreffen die eigentliche Erkennung in
[recognition.rs](../src/recognition.rs) und [symbols.rs](../src/symbols.rs).
Die Benchmark-Adapter, Referenzdateien, Stichproben, Ausschlüsse und
Bewertungsregeln wurden unverändert gelassen.

## Ursachen und Änderungen

- **Vorzeichnungen:** Zeichen nach einem sicher erkannten ersten Kreuz/B
  dürfen schwächer ausfallen, wenn sie dicht anschließen und an der
  vorgeschriebenen Notenposition stehen. Die Entfernung der Notenlinien
  beschädigt dieselbe Zeichenform je nach Position unterschiedlich.
  Ein fehlendes/unlesbares erstes Zeichen wird dadurch nicht als gültige
  Vorzeichnung übernommen.
- **Keine Vorzeichen:** Nur eine tatsächlich leere Lücke zwischen erkanntem
  Schlüssel und erkanntem Taktzeichen bestätigt null Kreuze/Bes. Fremde oder
  unlesbare Tinte sowie fehlende Systemangaben bleiben unbekannt.
- **Balken:** Lange senkrechte Striche zählen nicht mehr als Balken.
  Eine Verbindung ausschließlich über eine Notenlinie genügt ebenfalls
  nicht: Das verhinderte zuvor die Unterscheidung von benachbarten Taktstrichen
  oder Arpeggio-Zeichen und einem echten Balken.
- **Fähnchen:** Bei abwärts gerichteten Hälsen werden beide seitlichen
  Platzierungen geprüft. Der Treffer muss am eigenen Halsende liegen;
  mehrfache Fähnchen behalten die strengere Mindestqualität. Eine klar
  durchgehende Balkenverbindung schließt zusätzliche Fähnchen aus.
- **Zusätzliche Notenköpfe:** Bestätigte Fähnchen werden bei der Abweisung
  scheinbarer Köpfe am gegenüberliegenden Ende desselben Halses
  berücksichtigt. Ergebnisse der Fähnchensuche werden pro Hals wiederverwendet.
- **Hohe Akkorde:** Die Halssuche kann länger als fünf Notenabstände verfolgen,
  damit auch der oberste Kopf den gemeinsamen Balken erreicht. Weißraum
  beendet die Suche weiterhin.

## Prüfverfahren

Die Entwicklung erfolgte anhand des `dev`-Bestands. Nach Abschluss der
Änderungen wurde derselbe Code auf den zuvor zurückgehaltenen `test`-Bestand
angewandt. Der Zustand vor den Änderungen wurde dafür mit dem erhaltenen
ursprünglichen Benchmark-Programm ebenfalls auf `test` ausgeführt.
Kontrollbilder wurden nicht für weitere Anpassungen verwendet.

Die ursprünglichen Entwicklungsberichte bleiben unter
`target/omr-benchmarks/primus-baseline/` und `doremi-baseline/` erhalten.
Die endgültigen neuen Berichte liegen unter `primus-optimized-dev/`,
`doremi-optimized-dev/`, `primus-optimized-test/` und `doremi-optimized-test/`
im selben Verzeichnis. Die Vergleichsläufe verwenden `--baseline` ohne
erhöhte Regressionstoleranz; etwaige Rückschritte einzelner Kennzahlen
bleiben in den jeweiligen `regressions.json` sichtbar.

Zusätzliche Regressionstests prüfen benachbarte Taktstriche/Ornamente,
lange Akkordhälse, beide Fähnchenpositionen, beschädigte Folgezeichen und
die Abgrenzung einer leeren Vorzeichnung von unbekannten Zeichen.
Die lokalen Cassiopeia-Tests für Notenwerte, Einzelvorzeichen und die
Triolen der zweiten Zeile werden separat ausgeführt.

## Messungen

Die vollständigen aggregierten Ergebnisse und Quellenprüfsummen stehen in
[optimization-summary.json](optimization-summary.json). F1 und SER beziehen
sich jeweils auf den unterstützten Teilbestand, nicht auf alle Notationsarten.
Die Notenwert-F1 von DoReMi bewertet die klingende Dauer einschließlich
Punkten und Triolen.

| Messung | Vorher | Nachher |
| --- | ---: | ---: |
| PrIMuS Entwicklung: Ereignis-SER ↓ | 21,99 % | 6,96 % |
| PrIMuS Kontrolle: Ereignis-SER ↓ | 19,67 % | 7,18 % |
| PrIMuS Entwicklung: Vorzeichnungs-F1 ↑ | 47,69 % | 95,08 % |
| PrIMuS Kontrolle: Vorzeichnungs-F1 ↑ | 55,56 % | 97,50 % |
| PrIMuS Entwicklung: Notenwert-F1 ↑ | 83,72 % | 94,97 % |
| PrIMuS Kontrolle: Notenwert-F1 ↑ | 81,19 % | 93,70 % |
| DoReMi Entwicklung: Notenwert-F1 ↑ | 57,20 % | 67,51 % |
| DoReMi Kontrolle: Notenwert-F1 ↑ | 60,95 % | 71,77 % |

PrIMuS: unverändert 159 auswertbare Entwicklungs- und 41 Kontrollbeispiele;
232 beziehungsweise 68 Beispiele außerhalb des Adapterumfangs. DoReMi:
30 Entwicklungs- und 10 Kontrollseiten, alle auswertbar. Kein
Verarbeitungsfehler in den vier endgültigen Läufen.

Beide Kontrollläufe und der PrIMuS-Entwicklungslauf bestehen den strengen
Vergleich ohne Kennzahlenrückschritt. Im DoReMi-Entwicklungsbestand steigt
die Zahl korrekter Notendauern von 902 auf 1.065. Gleichzeitig entsteht ein
zusätzlicher falscher Halbnotentreffer (98 → 99); die Klassen-F1 sinkt dadurch
leicht von 88,47 % auf 88,45 %. Tonhöhen-F1 sinkt von 73,62 % auf 73,60 %,
Alterations-F1 von 83,45 % auf 83,36 %. Diese Rückschritte bleiben ausdrücklich
als Regressionen ausgewiesen; die Toleranz und der Bewertungsumfang wurden
nicht angehoben.

Die Messungen belegen eine Verbesserung auf diesen festen Ausschnitten und
Seiten. Sie belegen keine allgemeine Genauigkeit auf anderen Schriften, alten
Scans, Fotos oder bislang ausgeschlossenen Notationsarten.

## Abschließende Prüfungen

- `cargo test --locked`: 95 Bibliotheks- und 49 Desktop-Tests bestanden;
  27 lokale Originaltests regulär deaktiviert.
- Drei dieser Originaltests zusätzlich mit dem Cassiopeia-Bariton-Scan
  ausgeführt: Notenwerte, Einzelvorzeichen und Triolen; alle bestanden.
- `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo fmt --all --check` und der Desktop-Build bestanden.

Das vollständige Testprotokoll liegt unter
`target/omr-benchmarks/tests-final-verified.log`; die separaten Originaltests
unter `original-note-tests-optimized.log` und `original-triplet-test-optimized.log`
im selben Ordner.
