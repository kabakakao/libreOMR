# Ergänzende Lizenztexte

Einige der in `Cargo.lock` fixierten Crates enthalten im veröffentlichten
Cargo-Archiv keinen eigenen Lizenztext. Hier liegen ergänzende Hinweise aus
den zugehörigen Upstream-Repositories. Die Zuordnung gilt jeweils für eine
bestimmte Paketversion.

`sources.json` dokumentiert für jede Datei die Quelle und SHA-256-Prüfsumme.
Git-Quellen verweisen auf feste Commits aus den Cargo-VCS-Metadaten oder den
zugehörigen Versionstags. Die Dateien werden unverändert übernommen.
`.gitattributes` verhindert für diesen Ordner die automatische Konvertierung
der Zeilenenden, damit die Prüfsummen auch beim Windows-Checkout erhalten bleiben.

Die älteren Pakete `block`, `dispatch` und `malloc_buf` deklarieren MIT in
ihrem Manifest, liefern aber keinen separaten Lizenztext. Für sie sind die
ursprünglichen Manifeste mit Autoren- und Lizenzangabe sowie der Standardtext
aus der SPDX License List enthalten. Für `hexf-parse` sind entsprechend die
CC0-Deklaration und der CC0-Standardtext enthalten. Platzhalter in diesen
Standardtexten stammen aus der SPDX-Vorlage; sie werden nicht als zusätzliche
Copyright-Angaben ausgegeben. Bei `r-efi` enthält `AUTHORS` die Lizenztexte.

Die Paketierung kopiert diese Dateien samt Quellenverzeichnis nach `licenses/`
und kontrolliert ihre Prüfsummen. Bei einem Abhängigkeitsupdate müssen
fehlende Hinweise für neue Versionen erneut aus der zugehörigen Quelle
übernommen und in `sources.json` vermerkt werden. Die Projektlizenz ändert
die Lizenzen dieser Komponenten nicht.
