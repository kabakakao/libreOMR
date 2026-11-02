//! Offline, platform-independent OCR for scans without a PDF text layer.
use std::sync::{Mutex, OnceLock};

use image::GrayImage;
use rusto::{
    DetectTextResult, InitializeConfig, OcrRunOptions, OutputGranularity, RustO, TextResult,
};

use crate::model::{Staff, SymbolBounds, TextGlyph};

fn engine() -> Result<&'static Mutex<RustO>, String> {
    static ENGINE: OnceLock<Result<Mutex<RustO>, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            // RustO loads models from files. The bundled weights are materialized
            // only for initialization; page pixels never leave this process.
            let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
            for (name, bytes) in [
                (
                    "det.rten",
                    include_bytes!("../assets/ocr/det.rten").as_slice(),
                ),
                (
                    "rec.rten",
                    include_bytes!("../assets/ocr/rec.rten").as_slice(),
                ),
                (
                    "dict.txt",
                    include_bytes!("../assets/ocr/dict.txt").as_slice(),
                ),
            ] {
                std::fs::write(directory.path().join(name), bytes).map_err(|e| e.to_string())?;
            }
            let mut config = InitializeConfig::ppv6(
                directory.path().join("det.rten"),
                directory.path().join("rec.rten"),
                directory.path().join("dict.txt"),
            );
            config.det.limit_side_len = 1600;
            config.det.limit_type = "max".into();
            RustO::initialize(config)
                .map(Mutex::new)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Return positioned words in the coordinates of the supplied (deskewed) page.
pub fn recognize(image: &GrayImage, staves: &[Staff]) -> Result<Vec<TextGlyph>, String> {
    if staves.is_empty() {
        return Ok(Vec::new());
    }
    let mut engine = engine()?.lock().map_err(|e| e.to_string())?;
    let options = OcrRunOptions {
        output: OutputGranularity::Words,
        text_score: Some(0.55),
        max_side_len: Some(image.width().max(image.height()) as f32),
        ..Default::default()
    };
    let mut read = |gray: GrayImage| -> Result<Vec<TextResult>, String> {
        let rgb = image::DynamicImage::ImageLuma8(gray).into_rgb8();
        match engine
            .detect_text_rgb(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                rgb.width() as usize * 3,
                &options,
            )
            .map_err(|e| e.to_string())?
        {
            DetectTextResult::Structured(words) => Ok(words),
            DetectTextResult::Spatial(_) => Ok(Vec::new()),
        }
    };
    let mut words = read(image.clone())?;
    // Re-read the space between staves at higher detection resolution. Music
    // symbols otherwise shrink the page's small, widely spaced lyric syllables.
    let mut order: Vec<_> = staves.iter().collect();
    order.sort_by(|a, b| a.lines[0].total_cmp(&b.lines[0]));
    for (i, staff) in order.iter().enumerate() {
        let gap = staff.spacing();
        let left = (staff.left - gap).max(0.0) as u32;
        let right = (staff.right + gap).min(image.width() as f32) as u32;
        let top = (staff.lines[4] + gap * 0.8).max(0.0) as u32;
        let bottom = (staff.lines[4] + gap * 18.0)
            .min(
                order
                    .get(i + 1)
                    .map_or(image.height() as f32, |next| next.lines[0] - gap * 1.2),
            )
            .min(image.height() as f32) as u32;
        if right <= left || bottom <= top + 15 {
            continue;
        }
        let mut region =
            image::imageops::crop_imm(image, left, top, right - left, bottom - top).to_image();
        remove_extender_lines(&mut region, gap);
        let refined = read(region)?;
        words.retain(|word| {
            let baseline = word.frame.top + word.frame.height * 0.7;
            !(baseline >= top as f32
                && baseline <= bottom as f32
                && word.frame.left >= left as f32
                && word.frame.left < right as f32
                && refined.iter().any(|replacement| {
                    let frame = &replacement.frame;
                    let overlap = (word.frame.left + word.frame.width)
                        .min(frame.left + left as f32 + frame.width)
                        - word.frame.left.max(frame.left + left as f32);
                    (baseline - (frame.top + top as f32 + frame.height * 0.7)).abs() < gap * 0.8
                        && overlap > word.frame.width.min(frame.width) * 0.3
                }))
        });
        words.extend(refined.into_iter().map(|mut word| {
            word.frame.left += left as f32;
            word.frame.top += top as f32;
            word
        }));
    }
    Ok(positioned_words(words, image.width(), image.height()))
}

fn remove_extender_lines(image: &mut GrayImage, gap: f32) {
    // Long lyric extenders otherwise join small vowels into one detection box.
    // Operate only on the OCR crop, preserving the original recognition image.
    for y in 0..image.height() {
        let mut start = 0;
        for x in 0..=image.width() {
            if x < image.width() && image.get_pixel(x, y)[0] < 190 {
                continue;
            }
            if (x - start) as f32 >= gap * 6.0 {
                for column in start..x {
                    image.put_pixel(column, y, image::Luma([255]));
                }
            }
            start = x + 1;
        }
    }
}

fn positioned_words(words: Vec<TextResult>, width: u32, height: u32) -> Vec<TextGlyph> {
    let mut glyphs = Vec::new();
    for word in words {
        let frame = word.frame;
        if word.text.trim().is_empty() || frame.width <= 0.0 || frame.height <= 0.0 {
            continue;
        }
        let chars: Vec<_> = word.text.chars().collect();
        let prefix_end = chars.iter().position(|c| c.is_alphabetic()).filter(|&end| {
            end >= 2
                && matches!(chars[end - 1], '.' | ')')
                && chars[..end - 1].iter().all(|c| c.is_ascii_digit())
        });
        let mut spans = Vec::new();
        let mut start = 0;
        for (i, c) in chars.iter().enumerate() {
            if prefix_end == Some(i) && start < i {
                spans.push((start, i));
                start = i;
            }
            if c.is_whitespace() || matches!(c, '-' | '\u{2010}' | '\u{2011}') {
                if start < i {
                    spans.push((start, i));
                }
                if !c.is_whitespace() {
                    spans.push((i, i + 1));
                }
                start = i + 1;
            }
        }
        if start < chars.len() {
            spans.push((start, chars.len()));
        }
        for (start, end) in spans {
            let left = (frame.left + frame.width * start as f32 / chars.len() as f32)
                .clamp(0.0, width as f32);
            let right = (frame.left + frame.width * end as f32 / chars.len() as f32)
                .clamp(0.0, width as f32);
            let bounds = SymbolBounds {
                left,
                right,
                top: frame.top.max(0.0),
                bottom: (frame.top + frame.height).min(height as f32),
            };
            let baseline = frame.top + frame.height * 0.7;
            if right <= left || bounds.bottom <= bounds.top {
                continue;
            }
            // Word boxes are already separated by OCR. An explicit space keeps
            // the PDF character tokenizer from concatenating adjacent OCR words.
            glyphs.push(TextGlyph {
                text: " ".into(),
                baseline,
                bounds: SymbolBounds {
                    left: (left - 0.5).max(0.0),
                    right: left.max(0.1),
                    ..bounds.clone()
                },
            });
            glyphs.push(TextGlyph {
                text: chars[start..end]
                    .iter()
                    .map(|&c| {
                        if matches!(c, '\u{2010}' | '\u{2011}') {
                            '-'
                        } else {
                            c
                        }
                    })
                    .collect(),
                baseline,
                bounds,
            });
        }
    }
    glyphs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_word_boundaries_prefixes_and_unicode_survive_positioning() {
        let words = ["1.Ev'-ry", "Fr\u{00fc}h"]
            .into_iter()
            .enumerate()
            .map(|(i, text)| TextResult {
                text: text.into(),
                score: 0.99,
                box_points: [(0.0, 0.0); 4],
                frame: rusto::types::Frame::new(80.0, 30.0, 50.0, 10.0 + i as f32 * 100.0),
            })
            .collect();
        let glyphs = positioned_words(words, 300, 100);
        let text: Vec<_> = glyphs
            .iter()
            .filter(|g| !g.text.trim().is_empty())
            .map(|g| g.text.as_str())
            .collect();
        assert_eq!(text, vec!["1.", "Ev'", "-", "ry", "Fr\u{00fc}h"]);
        crate::model::Score {
            source_text: glyphs.clone(),
            ..Default::default()
        }
        .validate()
        .unwrap();
        assert_eq!(glyphs.iter().filter(|g| g.text == " ").count(), 5);
    }

    #[test]
    fn bundled_rust_models_recognize_synthetic_lyrics_without_os_services() {
        let image = image::load_from_memory(include_bytes!("../tests/fixtures/ocr-lyrics.png"))
            .unwrap()
            .to_luma8();
        let staff = Staff {
            lines: [0.0, 8.0, 16.0, 24.0, 32.0],
            left: 0.0,
            right: 479.0,
            header: Default::default(),
        };
        let glyphs = recognize(&image, &[staff]).unwrap();
        let text = glyphs
            .iter()
            .map(|g| g.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("Come") && text.contains("Liza"), "{text}");
        assert!(
            text.contains("Fr\u{00fc}h") && text.contains("Morgen"),
            "{text}"
        );
    }
}
