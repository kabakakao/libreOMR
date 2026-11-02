//! Headless import using the same recognition and text pipeline as the desktop app.
use libreomr::{
    document_import,
    model::{ProjectDocument, ProjectPage},
    musicxml, recognition,
};
use std::{io::Cursor, path::PathBuf};

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("Usage: inspect_score INPUT [OUTPUT_DIRECTORY]")?,
    );
    let output = PathBuf::from(args.next().unwrap_or_else(|| "local/inspection".into()));
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let mut project = ProjectDocument {
        version: 2,
        pages: Vec::new(),
    };
    for (index, page) in document_import::open_document(&path)?
        .into_iter()
        .enumerate()
    {
        let text = page.text();
        eprintln!(
            "Page {}: {} positioned text characters",
            index + 1,
            text.len()
        );
        let image = page.render()?;
        let image = if text.is_empty() {
            recognition::straighten(image)
        } else {
            image
        };
        let mut score = recognition::recognize_with_text(&image, text, |progress| {
            if progress.staff.is_none() {
                eprintln!("Page {}: {}", index + 1, progress.stage);
            }
        })?;
        if let Some(previous) = project.pages.last() {
            score.inherit_part_headers(&previous.score);
        }
        score.validate()?;
        for (staff_index, staff) in score.staves.iter().enumerate() {
            eprintln!(
                "  staff {}: {:?}, {:?}, {:?}, {} notes, {} bars",
                staff_index + 1,
                staff.header.clef,
                staff.header.key_signature,
                staff.header.time_signature,
                score
                    .notes
                    .iter()
                    .filter(|n| n.staff == staff_index)
                    .count(),
                score
                    .barlines
                    .iter()
                    .filter(|b| b.staff == staff_index)
                    .count()
            );
        }
        println!(
            "Page {}: {} staves, {} notes, {} lyrics, {} part assignments",
            index + 1,
            score.staves.len(),
            score.notes.len(),
            score.lyrics.len(),
            score.staff_assignments.len()
        );
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        project.pages.push(ProjectPage {
            image_name: page.name(),
            image_png: png.into_inner(),
            score,
        });
    }
    let title = path.file_stem().unwrap_or_default().to_string_lossy();
    let export = musicxml::export_pages(
        &project.pages.iter().map(|p| &p.score).collect::<Vec<_>>(),
        &title,
    )?;
    std::fs::write(output.join("score.musicxml"), export.xml).map_err(|e| e.to_string())?;
    std::fs::write(output.join("warnings.txt"), export.warnings.join("\n"))
        .map_err(|e| e.to_string())?;
    std::fs::write(
        output.join("score.libreomr.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    println!("Project and MusicXML: {}", output.display());
    Ok(())
}
