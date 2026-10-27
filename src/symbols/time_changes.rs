//! The meter search keeps the exhaustive search's templates, positions and scores,
//! but shares geometric checks and computes correlations only at valid positions.
use std::{cell::OnceCell, collections::HashMap, sync::OnceLock};

use super::{HeaderImage, Match, SymbolBounds, TimeSignature, UNIT, meter_template, resized};

pub(super) struct MeterTemplate {
    width: u32,
    height: u32,
    squared_sum: f32,
    blocks: [u64; 16],
    ink: Vec<(u32, u32, f32)>,
}

impl MeterTemplate {
    fn new(image: &image::GrayImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            squared_sum: image.iter().map(|&value| f32::from(value).powi(2)).sum(),
            blocks: SquaredImage::new(image).blocks(0, 0, image.width(), image.height()),
            ink: image
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel[0] != 0)
                .map(|(x, y, pixel)| (x, y, f32::from(pixel[0])))
                .collect(),
        }
    }
}

pub(super) fn templates() -> &'static [(TimeSignature, Vec<MeterTemplate>)] {
    static TEMPLATES: OnceLock<Vec<(TimeSignature, Vec<MeterTemplate>)>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        [TimeSignature::Common, TimeSignature::Cut]
            .into_iter()
            .chain((1..=32).flat_map(|beats| {
                [1, 2, 4, 8, 16, 32]
                    .into_iter()
                    .map(move |beat_unit| TimeSignature::Meter { beats, beat_unit })
            }))
            .map(|meter| {
                let heights: &[f32] = match meter {
                    TimeSignature::Common => &[1.7, 1.9, 2.1, 2.3],
                    TimeSignature::Cut => &[2.8, 3.1, 3.4],
                    _ => &[3.6, 3.7, 3.8, 3.9, 4.0, 4.1, 4.2],
                };
                let template = meter_template(meter);
                (
                    meter,
                    heights
                        .iter()
                        .map(|height| MeterTemplate::new(&resized(&template, height * UNIT)))
                        .collect(),
                )
            })
            .collect()
    })
}

struct Window {
    x: u32,
    y: u32,
    squared_sum: OnceCell<f32>,
    blocks: [u64; 16],
    bounds: SymbolBounds,
}

// Integral squared intensities make empty-window and coarse energy checks O(1).
struct SquaredImage {
    stride: usize,
    sums: Vec<u64>,
}

impl SquaredImage {
    fn new(image: &image::GrayImage) -> Self {
        let stride = image.width() as usize + 1;
        let mut sums = vec![0; stride * (image.height() as usize + 1)];
        for y in 0..image.height() as usize {
            let mut row = 0;
            for x in 0..image.width() as usize {
                let value = u64::from(image.as_raw()[y * image.width() as usize + x]);
                row += value * value;
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row;
            }
        }
        Self { stride, sums }
    }

    fn rectangle(&self, x: u32, y: u32, width: u32, height: u32) -> u64 {
        let a = y as usize * self.stride + x as usize;
        let b = (y + height) as usize * self.stride + x as usize;
        self.sums[b + width as usize]
            - self.sums[b]
            - (self.sums[a + width as usize] - self.sums[a])
    }

    fn blocks(&self, x: u32, y: u32, width: u32, height: u32) -> [u64; 16] {
        std::array::from_fn(|index| {
            let column = index as u32 % 4;
            let row = index as u32 / 4;
            let left = width * column / 4;
            let top = height * row / 4;
            self.rectangle(
                x + left,
                y + top,
                width * (column + 1) / 4 - left,
                height * (row + 1) / 4 - top,
            )
        })
    }
}

fn can_reach_threshold(template: &MeterTemplate, window: &Window) -> bool {
    // Cauchy-Schwarz on 4x4 blocks bounds the full pixel correlation from above.
    // Keep a 0.01 margin below the actual 0.86 threshold. For <=4096 pixels,
    // positive f32 accumulation errors cannot bridge that margin. Larger
    // templates conservatively bypass this shortcut.
    if template.width * template.height > 4096 {
        return true;
    }
    let source_energy: u64 = window.blocks.iter().sum();
    let template_energy: u64 = template.blocks.iter().sum();
    let maximum: f64 = window
        .blocks
        .iter()
        .zip(&template.blocks)
        .map(|(&a, &b)| ((a as f64) * (b as f64)).sqrt())
        .sum();
    maximum >= 0.85 * ((source_energy as f64) * (template_energy as f64)).sqrt()
}

pub(super) struct MeterSearch<'a> {
    image: &'a HeaderImage,
    top: u32,
    bottom: u32,
    has_ink: bool,
    energy: SquaredImage,
    windows: HashMap<(u32, u32), Vec<Window>>,
}

impl<'a> MeterSearch<'a> {
    pub(super) fn new(image: &'a HeaderImage) -> Self {
        Self {
            image,
            top: (image.staff_top - 0.4 * UNIT).max(0.0) as u32,
            bottom: (image.staff_top + 4.4 * UNIT)
                .min(image.image.height() as f32)
                .max(0.0) as u32,
            has_ink: image.image.pixels().any(|pixel| pixel[0] != 0),
            energy: SquaredImage::new(&image.image),
            windows: HashMap::new(),
        }
    }

    // The predicate depends only on the window, not on the proposed meter.
    // Keep its first result for all templates with the same dimensions.
    pub(super) fn find(
        &mut self,
        templates: &[MeterTemplate],
        accept: impl Fn(&SymbolBounds) -> bool,
    ) -> Option<Match> {
        if !self.has_ink {
            return None;
        }
        let image = self.image;
        let stride = image.image.width() as usize;
        let source = image.image.as_raw();
        let mut best: Option<Match> = None;
        for template in templates {
            if template.width > image.image.width()
                || self.bottom <= self.top
                || template.height > self.bottom - self.top
            {
                continue;
            }
            let windows = self
                .windows
                .entry((template.width, template.height))
                .or_insert_with(|| {
                    let mut windows = vec![];
                    for y in self.top..=self.bottom - template.height {
                        for x in 0..=image.image.width() - template.width {
                            if self.energy.rectangle(x, y, template.width, template.height) == 0 {
                                continue;
                            }
                            let bounds = SymbolBounds {
                                left: image.left + x as f32 / image.scale,
                                top: image.top + y as f32 / image.scale,
                                right: image.left + (x + template.width) as f32 / image.scale,
                                bottom: image.top + (y + template.height) as f32 / image.scale,
                            };
                            if !accept(&bounds) {
                                continue;
                            }
                            windows.push(Window {
                                x,
                                y,
                                squared_sum: OnceCell::new(),
                                blocks: self.energy.blocks(x, y, template.width, template.height),
                                bounds,
                            });
                        }
                    }
                    windows
                });
            for window in windows {
                if !can_reach_threshold(template, window) {
                    continue;
                }
                let squared_sum = window.squared_sum.get_or_init(|| {
                    // Retain the dense matcher's exact row-ordered f32 sum,
                    // but calculate it only if a template can reach the threshold.
                    let mut sum = 0.0;
                    for dy in 0..template.height {
                        let row = (window.y + dy) as usize * stride + window.x as usize;
                        for &value in &source[row..row + template.width as usize] {
                            let value = f32::from(value);
                            sum += value * value;
                        }
                    }
                    sum
                });
                let mut dot = 0.0;
                // Zero template pixels contribute nothing. Nonzero pixels remain in
                // row order, preserving the original f32 accumulation and tie breaks.
                for &(x, y, value) in &template.ink {
                    let source = source[(window.y + y) as usize * stride + (window.x + x) as usize];
                    dot += f32::from(source) * value;
                }
                let quality = dot / (squared_sum * template.squared_sum).sqrt();
                if quality >= 0.86
                    && quality.is_finite()
                    && best.as_ref().is_none_or(|best| quality > best.quality)
                {
                    best = Some(Match {
                        quality,
                        bounds: window.bounds.clone(),
                    });
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    #[test]
    fn integral_energies_include_clipped_edges_and_partition_the_window() {
        let image = GrayImage::from_fn(7, 9, |x, y| Luma([(x * 23 + y * 7) as u8]));
        let energy = SquaredImage::new(&image);
        for x in 0..=image.width() {
            for y in 0..=image.height() {
                let width = image.width() - x;
                let height = image.height() - y;
                let expected: u64 = (y..y + height)
                    .flat_map(|row| (x..x + width).map(move |column| (column, row)))
                    .map(|(x, y)| u64::from(image.get_pixel(x, y)[0]).pow(2))
                    .sum();
                assert_eq!(energy.rectangle(x, y, width, height), expected);
                assert_eq!(
                    energy.blocks(x, y, width, height).iter().sum::<u64>(),
                    expected
                );
            }
        }
    }

    #[test]
    fn impossible_matches_skip_the_dense_energy_and_pixel_comparison() {
        let source = GrayImage::from_fn(16, 32, |_, y| Luma([if y < 8 { 255 } else { 0 }]));
        let image = HeaderImage {
            image: source,
            left: 0.0,
            top: 0.0,
            scale: 1.0,
            staff_top: 4.0,
        };
        let templates = [MeterTemplate::new(&GrayImage::from_pixel(
            16,
            32,
            Luma([255]),
        ))];
        let mut search = MeterSearch::new(&image);
        assert!(search.find(&templates, |_| true).is_none());
        assert!(!search.windows.is_empty());
        assert!(
            search
                .windows
                .values()
                .flatten()
                .all(|w| w.squared_sum.get().is_none())
        );
    }

    #[test]
    fn sparse_meter_scores_and_ties_match_the_dense_matcher() {
        // Include gray antialiasing values, repeated maxima and rejected maxima.
        for seed in [1, 17, 83] {
            for frequency in [2, 3, 4, 5, 11] {
                let source = GrayImage::from_fn(45, 55, |x, y| {
                    Luma([((x * 113 + y * 79 + seed) % 256) as u8])
                });
                let image = HeaderImage {
                    image: source,
                    left: 120.0,
                    top: 50.0,
                    scale: 0.7,
                    staff_top: 4.0,
                };
                let raw = GrayImage::from_fn(17, 30, |x, y| {
                    Luma([if (x + y) % frequency == 0 {
                        0
                    } else {
                        ((x * 113 + y * 79 + seed) % 256) as u8
                    }])
                });
                let prepared = [MeterTemplate::new(&resized(&raw, 30.0))];
                let accept = |bounds: &SymbolBounds| bounds.left > 122.0 && bounds.top > 52.0;
                let reference = image
                    .find_filtered(&raw, &[3.0], [0.0, 0.0, 45.0, 52.0], accept)
                    .filter(|m| m.quality >= 0.86);
                let actual = MeterSearch::new(&image).find(&prepared, accept);
                if frequency >= 5 {
                    assert!(actual.is_some());
                }
                assert_eq!(
                    actual.as_ref().map(|m| m.quality),
                    reference.as_ref().map(|m| m.quality)
                );
                assert_eq!(
                    actual.map(|m| serde_json::to_value(m.bounds).unwrap()),
                    reference.map(|m| serde_json::to_value(m.bounds).unwrap())
                );
            }
        }
    }
}
