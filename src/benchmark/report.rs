use std::{fmt::Write as _, fs, path::Path};

use image::{GrayImage, Rgb, RgbImage};
use imageproc::{drawing::draw_hollow_rect_mut, rect::Rect};

use super::{
    Object, Report,
    metrics::{Counts, Pair},
};

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "—".into(), |v| format!("{:.2}%", v * 100.0))
}

fn draw(image: &mut RgbImage, object: &Object, color: [u8; 3]) {
    let Some(bounds) = &object.bounds else {
        return;
    };
    let left = bounds.left.max(0.0).min(image.width() as f32 - 1.0) as i32;
    let top = bounds.top.max(0.0).min(image.height() as f32 - 1.0) as i32;
    let right = bounds.right.max(0.0).min(image.width() as f32 - 1.0) as i32;
    let bottom = bounds.bottom.max(0.0).min(image.height() as f32 - 1.0) as i32;
    if right > left && bottom > top {
        draw_hollow_rect_mut(
            image,
            Rect::at(left, top).of_size((right - left + 1) as u32, (bottom - top + 1) as u32),
            Rgb(color),
        );
        if right - left > 2 && bottom - top > 2 {
            draw_hollow_rect_mut(
                image,
                Rect::at(left + 1, top + 1)
                    .of_size((right - left - 1) as u32, (bottom - top - 1) as u32),
                Rgb(color),
            );
        }
    }
}

pub(super) fn overlay(
    image: &GrayImage,
    reference: &[Object],
    prediction: &[Object],
    pairs: &[Pair],
    path: &Path,
) -> Result<(), String> {
    let mut image = image::DynamicImage::ImageLuma8(image.clone()).to_rgb8();
    for pair in pairs {
        if let Some(i) = pair.reference {
            let color = if pair.correct {
                [0, 150, 70]
            } else if pair.prediction.is_some() {
                [230, 145, 0]
            } else {
                [220, 35, 35]
            };
            draw(&mut image, &reference[i], color);
        }
        if let Some(j) = pair.prediction {
            let color = if pair.correct {
                [0, 150, 70]
            } else if pair.reference.is_some() {
                [230, 145, 0]
            } else {
                [190, 0, 180]
            };
            draw(&mut image, &prediction[j], color);
        }
    }
    image.save(path).map_err(|e| e.to_string())
}

fn row(html: &mut String, name: &str, counts: &Counts) {
    write!(
        html,
        "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
        escape(name),
        counts.correct,
        counts.extra,
        counts.missing,
        percent(counts.precision()),
        percent(counts.recall()),
        percent(counts.f1())
    )
    .unwrap();
}

fn describe(object: Option<&Object>) -> String {
    object.map_or_else(
        || "—".into(),
        |a| {
            let properties = a
                .attributes
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            escape(&format!("{} {properties}", a.class))
        },
    )
}

pub(super) fn write_html(report: &Report, path: &Path) -> Result<(), String> {
    let mut html = String::from(
        "<!doctype html><html lang=\"de\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>libreOMR Benchmark</title><style>body{font:16px system-ui,sans-serif;max-width:1200px;margin:32px auto;padding:0 20px;color:#203040;background:#fafbfc}table{border-collapse:collapse;width:100%;margin:16px 0}th,td{text-align:left;border-bottom:1px solid #d7dfe7;padding:8px;overflow-wrap:anywhere}th{background:#eaf0f6}details{background:white;border:1px solid #d7dfe7;border-radius:6px;padding:12px;margin:10px 0}summary{cursor:pointer;overflow-wrap:anywhere}img{max-width:100%;height:auto;border:1px solid #ddd}.error{color:#a02020}.ok{color:#06773d}.wrong{background:#fff5db}code{overflow-wrap:anywhere}.legend{padding:12px;background:#edf2f7}</style><h1>libreOMR Benchmark</h1>",
    );
    write!(html, "<p>Datensatz: <strong>{}</strong> · Bestand: <strong>{}</strong> · Manifest: <code>{}</code> · Ausrichtung: {}</p>", escape(&report.dataset), escape(&report.split), escape(&report.manifest_fingerprint), report.straighten).unwrap();
    write!(html, "<p>{} ausgewählt · {} ausgewertet · {} nicht unterstützt · {} Fehler</p><p>Sequenzfehlerrate (SER): {} · vollständig richtige Ereignissequenzen: {}/{}. Die Sequenz umfasst Noten, Pausen und Taktstriche; Systemangaben werden separat bewertet.</p>", report.coverage.selected, report.coverage.evaluated, report.coverage.unsupported, report.coverage.failed, percent(report.metrics.symbol_error_rate()), report.metrics.exact_sequences, report.metrics.sequence_samples).unwrap();
    write!(
        html,
        "<p>F1 über alle Symbolklassen (micro): {}</p>",
        percent(report.metrics.class_totals().f1())
    )
    .unwrap();
    html.push_str("<p>Klassenwerte messen Erkennung und Klassifikation. Eigenschaftswerte messen musikalische Angaben auf zugeordneten Objekten; fehlende Objekte zählen als fehlende Eigenschaften. Werte ohne Nenner bleiben undefiniert (—). Nicht unterstützte Beispiele zählen ausschließlich zur Abdeckung.</p>");
    for (title, counts) in [
        ("Symbolklassen", &report.metrics.classes),
        ("Musikalische Eigenschaften", &report.metrics.properties),
    ] {
        write!(html, "<h2>{title}</h2><table><tr><th>Klasse / Eigenschaft</th><th>Richtig</th><th>Zusätzlich</th><th>Fehlend</th><th>Precision</th><th>Recall</th><th>F1</th></tr>").unwrap();
        for (name, counts) in counts {
            row(&mut html, name, counts);
        }
        html.push_str("</table>");
    }
    html.push_str("<h2>Ausgeschlossene DoReMi-Klassen</h2><p>Diese Annotationen gehen nicht in die Kennzahlen ein. DoReMi misst derzeit gewöhnliche Notenköpfe, Pausen, Violin-/Bassschlüssel und zuordenbare Dynamikzeichen. PrIMuS misst keine Artikulationen oder Dynamik.</p><ul>");
    for (name, count) in &report.ignored_classes {
        write!(html, "<li>{}: {count}</li>", escape(name)).unwrap();
    }
    html.push_str("</ul><h2>Einzelbilder</h2><p class=\"legend\">Grün: korrekt · Rot: übersehen · Magenta: zusätzlich · Orange: falsche Klasse/Eigenschaft. PrIMuS liefert keine Pixelpositionen für Referenzereignisse: übersehene Ereignisse stehen dort nur in der Vergleichsliste. Die Zahl erzeugter Fehlerbilder ist begrenzt; alle Vergleiche stehen in report.json.</p>");
    for result in &report.samples {
        let errors = result.pairs.iter().filter(|p| !p.correct).count();
        write!(
            html,
            "<details><summary>{} · {} · {} Abweichungen · {} ms</summary>",
            escape(&result.sample.id),
            escape(&result.status),
            errors,
            result.elapsed_ms
        )
        .unwrap();
        write!(
            html,
            "<p>Gruppe: {} · {}</p>",
            escape(&result.sample.group),
            escape(&result.sample.split)
        )
        .unwrap();
        if let Some(reason) = &result.reason {
            write!(html, "<p class=\"error\">{}</p>", escape(reason)).unwrap();
        }
        if let Some(score) = &result.score {
            write!(
                html,
                "<p><a href=\"{}\">Erkanntes Score-Modell</a></p>",
                escape(score)
            )
            .unwrap();
        }
        if let Some(overlay) = &result.overlay {
            write!(html, "<a href=\"{}\"><img loading=\"lazy\" src=\"{}\" alt=\"Markierte Erkennungsfehler\"></a>", escape(overlay), escape(overlay)).unwrap();
        }
        if !result.pairs.is_empty() {
            html.push_str("<table><tr><th>Referenz</th><th>Erkennung</th></tr>");
            for pair in &result.pairs {
                write!(
                    html,
                    "<tr class=\"{}\"><td>{}</td><td>{}</td></tr>",
                    if pair.correct { "ok" } else { "wrong" },
                    describe(pair.reference.map(|i| &result.reference[i])),
                    describe(pair.prediction.map(|j| &result.prediction[j]))
                )
                .unwrap();
            }
            html.push_str("</table>");
        }
        html.push_str("</details>");
    }
    html.push_str("</html>");
    fs::write(path, html).map_err(|e| e.to_string())
}
