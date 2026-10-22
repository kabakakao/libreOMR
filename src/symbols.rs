use std::sync::OnceLock;

use ab_glyph::{Font, FontRef};
use image::{
    GrayImage, Luma,
    imageops::{self, FilterType},
};
use imageproc::template_matching::{MatchTemplateMethod, match_template};

use crate::model::{
    Articulation, Barline, Clef, Duration, Dynamic, DynamicKind, KeyChange, KeySignature, Rest,
    Score, Staff, StaffHeader, SymbolBounds, TimeChange, TimeSignature,
};

const UNIT: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HeadKind {
    Filled,
    Open,
    Whole,
}

pub(crate) struct Notehead {
    pub x: f32,
    pub y: f32,
    pub kind: HeadKind,
}

fn trimmed_glyph(character: char) -> GrayImage {
    trim_template(&glyph(character))
}

fn trim_template(image: &GrayImage) -> GrayImage {
    let mut bounds = (image.width(), image.height(), 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[0] > 127 {
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
    }
    imageops::crop_imm(
        image,
        bounds.0,
        bounds.1,
        bounds.2 - bounds.0 + 1,
        bounds.3 - bounds.1 + 1,
    )
    .to_image()
}

// Intersection over union penalizes both missing strokes and extra ink.
fn ink_similarity(source: &GrayImage, template: &GrayImage) -> f32 {
    let template = imageops::resize(
        template,
        source.width(),
        source.height(),
        FilterType::Triangle,
    );
    let mut best = 0.0_f32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let mut intersection = 0.0_f32;
            let mut union = 0.0_f32;
            // Include the shifted border so no missing stroke disappears from the union.
            for y in -1..source.height() as i32 + 1 {
                for x in -1..source.width() as i32 + 1 {
                    let ink = x >= 0
                        && y >= 0
                        && x < source.width() as i32
                        && y < source.height() as i32
                        && source.get_pixel(x as u32, y as u32)[0] < 128;
                    let tx = x - dx;
                    let ty = y - dy;
                    let expected = tx >= 0
                        && ty >= 0
                        && tx < template.width() as i32
                        && ty < template.height() as i32
                        && template.get_pixel(tx as u32, ty as u32)[0] > 127;
                    intersection += u8::from(ink && expected) as f32;
                    union += u8::from(ink || expected) as f32;
                }
            }
            best = best.max(intersection / union.max(1.0));
        }
    }
    best
}

pub(crate) fn recognize_repeat_marks(
    binary: &GrayImage,
    score: &Score,
) -> Vec<crate::model::RepeatMark> {
    use crate::model::{RepeatMark, RepeatMarkKind};
    if score.staves.is_empty() {
        return vec![];
    }
    let labels = imageproc::region_labelling::connected_components(
        binary,
        imageproc::region_labelling::Connectivity::Eight,
        Luma([255]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut components = vec![(u32::MAX, u32::MAX, 0, 0); maximum + 1];
    for (x, y, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let b = &mut components[label[0] as usize];
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
    let templates: Vec<_> = [RepeatMarkKind::Segno, RepeatMarkKind::Coda]
        .into_iter()
        .map(|kind| (kind, trimmed_glyph(kind.glyph().unwrap())))
        .chain([
            (RepeatMarkKind::Segno, trimmed_glyph('\u{f404}')),
            (RepeatMarkKind::Coda, trimmed_glyph('\u{f405}')),
        ])
        // Engraving fonts differ in the slant of the sign itself.
        .flat_map(|(kind, template)| {
            let mut padded = GrayImage::new(template.width() * 2, template.height() * 2);
            imageops::overlay(
                &mut padded,
                &template,
                i64::from(template.width() / 2),
                i64::from(template.height() / 2),
            );
            [-30.0_f32, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0].map(|angle| {
                let rotated = imageproc::geometric_transformations::rotate_about_center(
                    &padded,
                    angle.to_radians(),
                    imageproc::geometric_transformations::Interpolation::Bilinear,
                    Luma([0]),
                );
                (kind, trim_template(&rotated))
            })
        })
        .collect();
    let mut found: Vec<RepeatMark> = vec![];
    for (staff_index, staff) in score.staves.iter().enumerate() {
        let gap = staff.spacing();
        // Navigation signs and endings belong above the closest following staff.
        let top = (staff.lines[0] - gap * 9.0).max(0.0);
        let bottom = staff.lines[0] - gap * 0.3;
        let parts: Vec<_> = components
            .iter()
            .copied()
            .skip(1)
            .filter(|&(x0, y0, x1, y1)| {
                x0 as f32 >= staff.left - gap
                    && x1 as f32 <= staff.right + gap
                    && y0 as f32 >= top
                    && (y1 as f32) < bottom
                    && !score.staves.iter().any(|other| {
                        other.lines[0] < staff.lines[0] && other.lines[4] > y0 as f32 - gap * 0.5
                    })
            })
            .collect();
        for &(x0, y0, x1, y1) in &parts {
            if ((y1 - y0 + 1) as f32) < gap * 0.7
                || (y1 - y0 + 1) as f32 > gap * 4.0
                || (x1 - x0 + 1) as f32 > gap * 4.0
            {
                continue;
            }
            let mut b = (x0, y0, x1, y1);
            // Segno's two dots are disconnected from its central stroke.
            for &(a, c, d, e) in &parts {
                let dx = (a as f32 - x1 as f32).max(x0 as f32 - d as f32);
                let dy = (c as f32 - y1 as f32).max(y0 as f32 - e as f32);
                if dx <= gap * 0.55
                    && dy <= gap * 0.4
                    && (d - a + 1) as f32 <= gap * 4.0
                    && (e - c + 1) as f32 <= gap * 4.0
                {
                    b = (b.0.min(a), b.1.min(c), b.2.max(d), b.3.max(e));
                }
            }
            let crop =
                imageops::crop_imm(binary, b.0, b.1, b.2 - b.0 + 1, b.3 - b.1 + 1).to_image();
            let aspect = crop.width() as f32 / crop.height() as f32;
            let best = templates
                .iter()
                .filter_map(|(kind, template)| {
                    let ratio = aspect / (template.width() as f32 / template.height() as f32);
                    if !(0.72..=1.35).contains(&ratio) {
                        return None;
                    }
                    let quality = ink_similarity(&crop, template);
                    let dots = parts
                        .iter()
                        .filter(|&&(a, c, d, e)| {
                            a >= b.0
                                && c >= b.1
                                && d <= b.2
                                && e <= b.3
                                && (d - a + 1) as f32 <= gap * 0.7
                                && (e - c + 1) as f32 <= gap * 0.7
                                && (0.6..=1.6).contains(&((d - a + 1) as f32 / (e - c + 1) as f32))
                        })
                        .count();
                    (quality
                        >= if *kind == RepeatMarkKind::Segno && dots == 2 {
                            0.56
                        } else {
                            0.59
                        })
                    .then_some((*kind, quality))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((kind, _)) = best {
                let x = (b.0 + b.2) as f32 * 0.5;
                let y = (b.1 + b.3) as f32 * 0.5;
                if x < staff.left
                    || x > staff.right
                    || found
                        .iter()
                        .any(|mark| mark.staff == staff_index && (mark.x - x).abs() < gap)
                {
                    continue;
                }
                found.push(RepeatMark {
                    staff: staff_index,
                    x,
                    y,
                    kind,
                    end_x: x,
                    numbers: String::new(),
                    closed: false,
                    bounds: Some(SymbolBounds {
                        left: b.0 as f32,
                        top: b.1 as f32,
                        right: (b.2 + 1) as f32,
                        bottom: (b.3 + 1) as f32,
                    }),
                    confirmed: false,
                });
            }
        }
        let ink = |x: i32, y: i32| {
            x >= 0
                && y >= 0
                && x < binary.width() as i32
                && y < binary.height() as i32
                && binary.get_pixel(x as u32, y as u32)[0] == 0
        };
        // Long thin roofs with a downward left hook distinguish voltas from beams/hairpins.
        for row in top.ceil() as i32..bottom.floor() as i32 {
            let mut column = staff.left.ceil() as i32;
            while column < staff.right as i32 {
                if !ink(column, row) {
                    column += 1;
                    continue;
                }
                let left = column;
                let mut right = column;
                let tolerance = (gap * 0.12).ceil() as i32;
                while column <= staff.right as i32 {
                    if ink(column, row) {
                        right = column;
                    } else if column - right > tolerance {
                        break;
                    }
                    column += 1;
                }
                let width = (right - left) as f32;
                if width < gap * 3.0
                    || found.iter().any(|mark| {
                        mark.staff == staff_index
                            && mark.kind == RepeatMarkKind::Ending
                            && (mark.y - row as f32).abs() < gap * 0.5
                            && mark.x <= right as f32
                            && mark.end_x >= left as f32
                    })
                {
                    continue;
                }
                let hook = |edge: i32| {
                    (-tolerance..=tolerance).any(|dx| {
                        (1..=10)
                            .filter(|step| {
                                ink(edge + dx, row + (gap * *step as f32 / 12.0).round() as i32)
                            })
                            .count()
                            >= 9
                    })
                };
                if !hook(left) {
                    continue;
                }
                let middle = (left + right) / 2;
                let thickness = (row..row + (gap * 0.4).ceil() as i32)
                    .take_while(|y| ink(middle, *y))
                    .count();
                if thickness as f32 > gap * 0.22 {
                    continue;
                }
                // A printed ending label is required; unlabelled roofs may be beams.
                let digits: Vec<_> = parts
                    .iter()
                    .copied()
                    .filter(|&(x0, y0, x1, y1)| {
                        x0 as f32 > left as f32 + gap * 0.15
                            && (x1 as f32) < (left as f32 + gap * 5.0).min(right as f32)
                            && y0 as f32 > row as f32 + gap * 0.1
                            && (y1 as f32) < row as f32 + gap * 2.0
                            && (y1 - y0 + 1) as f32 >= gap * 0.45
                            && (x1 - x0 + 1) as f32 <= gap * 1.4
                    })
                    .collect();
                if digits.is_empty() {
                    continue;
                }
                let mut digits = digits;
                digits.sort_by_key(|b| b.0);
                let mut numbers = String::new();
                for (index, (x0, y0, x1, y1)) in digits.iter().enumerate() {
                    let crop =
                        imageops::crop_imm(binary, *x0, *y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
                    let best = (0..=9)
                        .flat_map(|digit| {
                            [0xe080, 0xe880].map(|base| {
                                let template = trimmed_glyph(char::from_u32(base + digit).unwrap());
                                (digit, ink_similarity(&crop, &template))
                            })
                        })
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .unwrap();
                    if best.1 < 0.62 {
                        numbers.clear();
                        break;
                    }
                    if index > 0 {
                        let previous = digits[index - 1];
                        let separator = parts.iter().any(|&(a, c, d, e)| {
                            a > previous.2
                                && d < *x0
                                && e >= previous.3.saturating_sub((gap * 0.3) as u32)
                                && c as f32 >= row as f32 + gap * 0.3
                                && (e - c + 1) as f32 <= gap * 0.4
                        });
                        if separator {
                            numbers.push(',');
                        } else if *x0 as f32 - previous.2 as f32 > gap * 0.25 {
                            numbers.clear();
                            break;
                        }
                    }
                    numbers.push(char::from_digit(best.0, 10).unwrap());
                }
                if !numbers.is_empty()
                    && numbers
                        .split(',')
                        .any(|n| !n.parse::<u8>().is_ok_and(|value| value > 0))
                {
                    numbers.clear();
                }
                found.push(RepeatMark {
                    staff: staff_index,
                    x: left as f32,
                    y: row as f32,
                    kind: RepeatMarkKind::Ending,
                    end_x: right as f32,
                    numbers,
                    closed: hook(right),
                    confirmed: false,
                    bounds: Some(SymbolBounds {
                        left: left as f32,
                        top: row as f32,
                        right: (right + 1) as f32,
                        bottom: digits
                            .iter()
                            .map(|b| b.3 as f32 + 1.0)
                            .fold(row as f32 + gap, f32::max),
                    }),
                });
            }
        }
    }
    found.sort_by(|a, b| a.staff.cmp(&b.staff).then(a.x.total_cmp(&b.x)));
    found
}

pub(crate) fn recognize_dynamics(binary: &GrayImage, score: &Score) -> Vec<Dynamic> {
    if score.staves.is_empty() {
        return vec![];
    }
    let templates: Vec<_> = DynamicKind::ALL
        .into_iter()
        .map(|kind| (kind, trimmed_glyph(kind.glyph())))
        .collect();
    let accents = [trimmed_glyph('\u{e4a0}'), trimmed_glyph('\u{e4a1}')];
    let labels = imageproc::region_labelling::connected_components(
        binary,
        imageproc::region_labelling::Connectivity::Eight,
        Luma([255]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut components = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32); maximum + 1];
    for (x, y, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let b = &mut components[label[0] as usize];
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
        b.4 += 1;
    }
    let mut found: Vec<Dynamic> = vec![];
    for (staff_index, staff) in score.staves.iter().enumerate() {
        let gap = staff.spacing();
        let mut groups: Vec<_> = components
            .iter()
            .copied()
            .skip(1)
            .filter(|&(x0, y0, x1, y1, area)| {
                let height = (y1 - y0 + 1) as f32 / gap;
                x0 as f32 >= staff.left
                    && x1 as f32 <= staff.right
                    && (((y1 as f32) < staff.lines[0] - gap * 0.35
                        && y0 as f32 > staff.lines[0] - gap * 6.0)
                        || (y0 as f32 > staff.lines[4] + gap * 0.35
                            && (y1 as f32) < staff.lines[4] + gap * 6.0))
                    && (0.45..=3.0).contains(&height)
                    && area as f32 >= gap * gap * 0.06
            })
            .filter(|&(x0, y0, x1, y1, _)| {
                let width = (x1 - x0 + 1) as f32;
                let height = (y1 - y0 + 1) as f32;
                // An accent from the next row can sit immediately beside a
                // dynamic. Keep it out of the letter group.
                if height <= gap * 1.5 && width / height >= 1.2 {
                    let crop =
                        imageops::crop_imm(binary, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
                    return !accents
                        .iter()
                        .any(|template| ink_similarity(&crop, template) >= 0.55);
                }
                true
            })
            .collect();
        groups.sort_by_key(|b| b.0);
        // Join letters sharing a baseline, including repeated p/f and sfz.
        let mut changed = true;
        while changed {
            changed = false;
            'merge: for i in 0..groups.len() {
                for j in i + 1..groups.len() {
                    let a = groups[i];
                    let b = groups[j];
                    let horizontal = (b.0 as f32 - a.2 as f32).max(a.0 as f32 - b.2 as f32);
                    let overlap = a.3.min(b.3) as f32 - a.1.max(b.1) as f32;
                    let min_height = (a.3 - a.1 + 1).min(b.3 - b.1 + 1) as f32;
                    if horizontal <= gap * 0.55
                        && overlap >= min_height * 0.35
                        && (a.2.max(b.2) - a.0.min(b.0)) as f32 <= gap * 9.0
                    {
                        groups[i] = (
                            a.0.min(b.0),
                            a.1.min(b.1),
                            a.2.max(b.2),
                            a.3.max(b.3),
                            a.4 + b.4,
                        );
                        groups.remove(j);
                        changed = true;
                        break 'merge;
                    }
                }
            }
        }
        for (x0, y0, x1, y1, _) in groups {
            let bounds = SymbolBounds {
                left: x0 as f32,
                top: y0 as f32,
                right: (x1 + 1) as f32,
                bottom: (y1 + 1) as f32,
            };
            let x = (bounds.left + bounds.right) * 0.5;
            let y = (bounds.top + bounds.bottom) * 0.5;
            // Prefer the usual below-staff placement when two nearby rows compete.
            let edge_distance = |other: &Staff| {
                if y < other.lines[0] {
                    other.lines[0] - y + other.spacing() * 2.0
                } else {
                    (y - other.lines[4]).max(0.0)
                }
            };
            if score.staves.iter().enumerate().any(|(index, other)| {
                index != staff_index
                    && x >= other.left
                    && x <= other.right
                    && (edge_distance(other) < edge_distance(staff)
                        || (edge_distance(other) == edge_distance(staff) && index < staff_index))
            }) {
                continue;
            }
            if score
                .repeat_marks
                .iter()
                .filter(|mark| mark.staff == staff_index)
                .filter_map(|mark| mark.bounds.as_ref())
                .any(|bounds| bounds.contains(x, y))
            {
                continue;
            }
            if [&staff.header.clef_bounds, &staff.header.time_bounds]
                .into_iter()
                .flatten()
                .chain(staff.header.key_bounds.iter())
                .chain(
                    score
                        .rests
                        .iter()
                        .filter(|rest| rest.staff == staff_index)
                        .filter_map(|rest| rest.bounds.as_ref()),
                )
                .any(|other| other.contains(x, y))
            {
                continue;
            }
            let crop = imageops::crop_imm(binary, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
            let aspect = crop.width() as f32 / crop.height() as f32;
            if let Some((kind, _)) = templates
                .iter()
                .filter_map(|(kind, template)| {
                    let ratio = aspect / (template.width() as f32 / template.height() as f32);
                    if !(0.75..=1.3).contains(&ratio) {
                        return None;
                    }
                    let quality = ink_similarity(&crop, template);
                    (quality >= 0.56).then_some((*kind, quality))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                found.push(Dynamic {
                    staff: staff_index,
                    x,
                    y,
                    kind,
                    bounds: Some(bounds),
                    confirmed: false,
                });
            }
        }
    }
    found.sort_by(|a, b| a.staff.cmp(&b.staff).then(a.x.total_cmp(&b.x)));
    found
}

pub(crate) fn recognize_articulations(binary: &GrayImage, score: &mut Score) {
    if score.notes.is_empty() {
        return;
    }
    let normalized =
        imageproc::contrast::threshold(binary, 170, imageproc::contrast::ThresholdType::Binary);
    let binary = &normalized;
    let templates: Vec<_> = Articulation::ALL
        .into_iter()
        .filter(|kind| {
            !matches!(
                kind,
                Articulation::None | Articulation::Staccato | Articulation::Tenuto
            )
        })
        .flat_map(|kind| {
            [false, true].map(move |below| (kind, trimmed_glyph(kind.glyph(below).unwrap())))
        })
        .chain(
            ['\u{e4a8}', '\u{e4a9}']
                .into_iter()
                .map(|character| (Articulation::Staccatissimo, trimmed_glyph(character))),
        )
        .collect();
    let mut proposals = vec![None; score.notes.len()];
    for (staff_index, staff) in score.staves.iter().enumerate() {
        let gap = staff.spacing();
        let left = staff.left.max(0.0) as u32;
        let top = (staff.lines[0] - gap * 6.0).max(0.0) as u32;
        let right = staff.right.min(binary.width() as f32) as u32;
        let bottom = (staff.lines[4] + gap * 6.0).min(binary.height() as f32) as u32;
        if right <= left || bottom <= top {
            continue;
        }
        let mut image =
            imageops::crop_imm(binary, left, top, right - left, bottom - top).to_image();
        for (column, row, pixel) in image.enumerate_pixels_mut() {
            let source_x = left + column;
            let source_y = top + row;
            let on_line = staff
                .lines
                .iter()
                .any(|line| (*line - source_y as f32).abs() <= (gap * 0.12).max(1.0));
            let offset = (gap * 0.2).ceil() as u32;
            let crossing = source_y >= offset
                && source_y + offset < binary.height()
                && binary.get_pixel(source_x, source_y - offset)[0] == 0
                && binary.get_pixel(source_x, source_y + offset)[0] == 0
                && binary.get_pixel(source_x, source_y - 1)[0] == 0
                && binary.get_pixel(source_x, source_y + 1)[0] == 0;
            let reach = (gap * 2.0).ceil() as u32;
            let continuous = source_x >= reach
                && source_x + reach < binary.width()
                && binary.get_pixel(source_x - reach, source_y)[0] == 0
                && binary.get_pixel(source_x + reach, source_y)[0] == 0;
            if on_line && continuous && !crossing {
                pixel[0] = 255;
            }
        }
        let labels = imageproc::region_labelling::connected_components(
            &image,
            imageproc::region_labelling::Connectivity::Eight,
            Luma([255]),
        );
        let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
        let mut components = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32); maximum + 1];
        for (column, row, label) in labels.enumerate_pixels() {
            if label[0] == 0 {
                continue;
            }
            let bounds = &mut components[label[0] as usize];
            bounds.0 = bounds.0.min(column);
            bounds.1 = bounds.1.min(row);
            bounds.2 = bounds.2.max(column);
            bounds.3 = bounds.3.max(row);
            bounds.4 += 1;
        }
        // Fermatas and combined articulations contain disconnected strokes/dots.
        // Compare their complete vertical groups before considering individual parts.
        let mut candidates: Vec<_> = components.iter().copied().skip(1).collect();
        for (index, &(x0, y0, x1, y1, area)) in components.iter().enumerate().skip(1) {
            for (other_index, &(ox0, oy0, ox1, oy1, other_area)) in
                components.iter().enumerate().skip(index + 1)
            {
                let vertical_gap = (oy0 as f32 - y1 as f32).max(y0 as f32 - oy1 as f32);
                let centers = ((x0 + x1) as f32 - (ox0 + ox1) as f32).abs() * 0.5;
                if centers <= gap * 0.6
                    && vertical_gap <= gap * 0.65
                    && (x1.max(ox1) - x0.min(ox0)) as f32 <= gap * 2.0
                    && (y1.max(oy1) - y0.min(oy0)) as f32 <= gap * 2.0
                {
                    candidates.push((
                        x0.min(ox0),
                        y0.min(oy0),
                        x1.max(ox1),
                        y1.max(oy1),
                        area + other_area,
                    ));
                    let pair = (
                        x0.min(ox0),
                        y0.min(oy0),
                        x1.max(ox1),
                        y1.max(oy1),
                        area + other_area,
                    );
                    for &(tx0, ty0, tx1, ty1, third_area) in components.iter().skip(other_index + 1)
                    {
                        let centers = ((pair.0 + pair.2) as f32 - (tx0 + tx1) as f32).abs() * 0.5;
                        let vertical_gap =
                            (ty0 as f32 - pair.3 as f32).max(pair.1 as f32 - ty1 as f32);
                        if centers <= gap * 0.6
                            && vertical_gap <= gap * 0.65
                            && (pair.2.max(tx1) - pair.0.min(tx0)) as f32 <= gap * 2.0
                            && (pair.3.max(ty1) - pair.1.min(ty0)) as f32 <= gap * 2.0
                        {
                            candidates.push((
                                pair.0.min(tx0),
                                pair.1.min(ty0),
                                pair.2.max(tx1),
                                pair.3.max(ty1),
                                pair.4 + third_area,
                            ));
                        }
                    }
                }
            }
        }
        for (min_x, min_y, max_x, max_y, area) in candidates {
            let width = (max_x - min_x + 1) as f32 / gap;
            let height = (max_y - min_y + 1) as f32 / gap;
            if min_x == 0 || min_y == 0 || max_x + 1 == image.width() || max_y + 1 == image.height()
            {
                continue;
            }
            let bounds = SymbolBounds {
                left: (left + min_x) as f32,
                top: (top + min_y) as f32,
                right: (left + max_x + 1) as f32,
                bottom: (top + max_y + 1) as f32,
            };
            let center_x = (bounds.left + bounds.right) / 2.0;
            let center_y = (bounds.top + bounds.bottom) / 2.0;
            if score
                .repeat_marks
                .iter()
                .filter(|mark| mark.staff == staff_index)
                .filter_map(|mark| mark.bounds.as_ref())
                .any(|bounds| bounds.contains(center_x, center_y))
            {
                continue;
            }
            if [&staff.header.clef_bounds, &staff.header.time_bounds]
                .into_iter()
                .flatten()
                .chain(staff.header.key_bounds.iter())
                .chain(
                    score
                        .rests
                        .iter()
                        .filter(|rest| rest.staff == staff_index)
                        .filter_map(|rest| rest.bounds.as_ref()),
                )
                .chain(
                    score
                        .dynamics
                        .iter()
                        .filter(|dynamic| dynamic.staff == staff_index)
                        .filter_map(|dynamic| dynamic.bounds.as_ref()),
                )
                .chain(
                    score
                        .time_changes
                        .iter()
                        .filter(|change| change.staff == staff_index)
                        .flat_map(|change| &change.bounds),
                )
                .chain(
                    score
                        .key_changes
                        .iter()
                        .filter(|change| change.staff == staff_index)
                        .flat_map(|change| &change.bounds),
                )
                .any(|other| {
                    center_x >= other.left
                        && center_x <= other.right
                        && center_y >= other.top
                        && center_y <= other.bottom
                })
                || score
                    .barlines
                    .iter()
                    .any(|bar| bar.staff == staff_index && (bar.x - center_x).abs() < gap * 1.8)
                || score.notes.iter().any(|note| {
                    (note.x - center_x).abs() < gap * 0.8 && (note.y - center_y).abs() < gap * 0.65
                })
            {
                continue;
            }
            let density = area as f32 / ((max_x - min_x + 1) * (max_y - min_y + 1)) as f32;
            let detected = if (0.12..=0.5).contains(&width)
                && (0.12..=0.5).contains(&height)
                && (0.65..=1.5).contains(&(width / height))
                && (0.02..=0.2).contains(&(area as f32 / (gap * gap)))
                && density >= 0.55
            {
                if components.iter().skip(1).any(
                    |&(other_left, other_top, other_right, other_bottom, other_area)| {
                        other_area as f32 > area as f32 * 1.5
                            && other_left as f32 <= max_x as f32 + gap * 0.5
                            && other_right as f32 >= min_x as f32 - gap * 0.5
                            && other_top as f32 <= max_y as f32 + gap * 0.6
                            && other_bottom as f32 >= min_y as f32 - gap * 0.6
                            && !score.notes.iter().any(|note| {
                                note.x >= (left + other_left) as f32
                                    && note.x <= (left + other_right + 1) as f32
                                    && note.y >= (top + other_top) as f32
                                    && note.y <= (top + other_bottom + 1) as f32
                            })
                    },
                ) {
                    continue;
                }
                Some((Articulation::Staccato, 0.85))
            } else if (0.65..=1.5).contains(&width)
                && (0.07..=0.25).contains(&height)
                && width / height >= 3.0
                && density >= 0.8
            {
                Some((Articulation::Tenuto, 0.9))
            } else {
                templates
                    .iter()
                    .filter_map(|(kind, template)| {
                        let size_ok = match kind {
                            Articulation::Accent => {
                                (0.65..=1.9).contains(&width) && (0.25..=1.2).contains(&height)
                            }
                            Articulation::Staccatissimo => {
                                (0.1..=0.8).contains(&width) && (0.35..=1.6).contains(&height)
                            }
                            Articulation::Marcato | Articulation::MarcatoStaccato => {
                                (0.4..=1.5).contains(&width) && (0.45..=2.0).contains(&height)
                            }
                            _ => (0.5..=2.0).contains(&width) && (0.3..=2.0).contains(&height),
                        };
                        let aspect =
                            width / height / (template.width() as f32 / template.height() as f32);
                        let aspect_range = if *kind == Articulation::Staccatissimo {
                            0.55..=1.65
                        } else {
                            0.8..=1.25
                        };
                        if !size_ok || !aspect_range.contains(&aspect) {
                            return None;
                        }
                        let crop = imageops::crop_imm(
                            &image,
                            min_x,
                            min_y,
                            max_x - min_x + 1,
                            max_y - min_y + 1,
                        )
                        .to_image();
                        let mut quality = ink_similarity(&crop, template);
                        // At a staff line the outermost ink row can be inseparable
                        // from the line. Compare the correspondingly clipped outline.
                        let cut = (template.height() as f32 * 0.15).round() as u32;
                        if cut > 0 && cut < template.height() {
                            for (edge, from_top) in
                                [(bounds.top, true), (bounds.bottom - 1.0, false)]
                            {
                                if staff
                                    .lines
                                    .iter()
                                    .any(|line| (*line - edge).abs() <= gap * 0.18)
                                {
                                    let clipped = imageops::crop_imm(
                                        template,
                                        0,
                                        if from_top { cut } else { 0 },
                                        template.width(),
                                        template.height() - cut,
                                    )
                                    .to_image();
                                    quality = quality.max(ink_similarity(&crop, &clipped));
                                }
                            }
                        }
                        (quality >= 0.57).then_some((
                            *kind,
                            quality
                                + if matches!(
                                    kind,
                                    Articulation::Portato
                                        | Articulation::AccentStaccato
                                        | Articulation::TenutoAccent
                                        | Articulation::MarcatoStaccato
                                        | Articulation::Fermata
                                ) {
                                    0.3
                                } else {
                                    0.0
                                },
                        ))
                    })
                    .max_by(|first, second| first.1.total_cmp(&second.1))
            };
            let Some((kind, quality)) = detected else {
                continue;
            };
            let nearest = score
                .notes
                .iter()
                .enumerate()
                .filter_map(|(index, note)| {
                    if note.staff != staff_index {
                        return None;
                    }
                    let horizontal = (note.x - center_x).abs() / gap;
                    let vertical = if center_y < note.y {
                        note.y - bounds.bottom
                    } else {
                        bounds.top - note.y
                    } / gap;
                    (horizontal <= 0.6
                        && (if matches!(kind, Articulation::Staccato | Articulation::Tenuto) {
                            0.7
                        } else {
                            0.45
                        }..=5.0)
                            .contains(&vertical))
                    .then_some((index, vertical + horizontal * 3.0))
                })
                .min_by(|first, second| first.1.total_cmp(&second.1));
            if let Some((index, distance)) = nearest
                && score.notes[index].staff == staff_index
            {
                let rank = quality - distance * 0.03;
                if proposals[index].is_none_or(|(_, previous)| rank > previous) {
                    proposals[index] = Some((kind, rank));
                }
            }
        }
    }
    for (note, proposal) in score.notes.iter_mut().zip(proposals) {
        if let Some((kind, _)) = proposal {
            note.articulation = kind;
        }
    }
}

pub(crate) fn recognize_accidental(
    binary: &GrayImage,
    staff: &Staff,
    x: f32,
    y: f32,
) -> Option<(i8, SymbolBounds)> {
    let gap = staff.spacing();
    let mut local = staff.clone();
    local.left = (x - gap * 3.5).max(staff.left);
    local.right = (x - gap * 0.15).min(staff.right);
    if local.right <= local.left {
        return None;
    }
    let image = HeaderImage::region_with_margin(binary, &local, local.right, 5.0);
    let center_y = staff.y_at_pitch(staff.pitch_at(y));
    let normalized_y = (center_y - image.top) * image.scale;
    let mut best: Option<(i8, Match)> = None;
    for (value, character, anchor) in [
        (-1, '\u{e260}', 0.78),
        (0, '\u{e261}', 0.5),
        (1, '\u{e262}', 0.5),
    ] {
        let symbol = resized(&glyph(character), 26.0);
        let mut template = GrayImage::new(symbol.width() + 6, 32);
        imageops::overlay(&mut template, &symbol, 3, 3);
        let anchor = (3.0 + 26.0 * anchor) / 32.0;
        let found = image.find_filtered(
            &template,
            &[2.6, 2.8, 3.0, 3.2, 3.4, 3.6, 3.8],
            [
                0.0,
                normalized_y - 2.7 * UNIT,
                image.image.width() as f32,
                normalized_y + 2.2 * UNIT,
            ],
            |bounds| {
                let inset = (bounds.bottom - bounds.top) * 3.0 / 32.0;
                (bounds.top + (bounds.bottom - bounds.top) * anchor - center_y).abs() < gap * 0.25
                    && bounds.right - inset <= x - gap * 0.55
                    && bounds.right - inset >= x - gap * 2.0
                    && ![&staff.header.clef_bounds, &staff.header.time_bounds]
                        .into_iter()
                        .flatten()
                        .chain(staff.header.key_bounds.iter())
                        .any(|header| {
                            bounds.left < header.right
                                && bounds.right > header.left
                                && bounds.top < header.bottom
                                && bounds.bottom > header.top
                        })
            },
        );
        if let Some(found) = found.filter(|found| found.quality >= 0.82)
            && best
                .as_ref()
                .is_none_or(|(_, previous)| found.quality > previous.quality)
        {
            best = Some((value, found));
        }
    }
    best.map(|(value, found)| (value, found.bounds))
}

pub(crate) fn has_triplet_marker(
    binary: &GrayImage,
    staff: &Staff,
    center_x: f32,
    center_y: f32,
) -> bool {
    let gap = staff.spacing();
    let mut local = staff.clone();
    local.left = (center_x - 2.5 * gap).max(staff.left);
    local.right = (center_x + 2.5 * gap).min(staff.right);
    if local.right <= local.left {
        return false;
    }
    let image = HeaderImage::region_with_margin(binary, &local, local.right, 5.0);
    let x = (center_x - image.left) * image.scale;
    let y = (center_y - image.top) * image.scale;
    ['\u{e883}', '\u{e083}'].into_iter().any(|character| {
        image
            .find_filtered(
                &glyph(character),
                &[0.5, 0.65, 0.8, 1.0],
                [x - 1.5 * UNIT, y - UNIT, x + 1.5 * UNIT, y + UNIT],
                |bounds| {
                    ((bounds.left + bounds.right) / 2.0 - center_x).abs() <= gap
                        && ((bounds.top + bounds.bottom) / 2.0 - center_y).abs() <= gap * 0.6
                },
            )
            .is_some_and(|found| found.quality >= 0.78)
    })
}

fn hollow_noteheads(binary: &GrayImage, staff: &Staff, prepared: &HeaderImage) -> Vec<Notehead> {
    let top = prepared.top as u32;
    let left = prepared.left as u32;
    let bottom = (staff.lines[4] + staff.spacing() * 5.0).min(binary.height() as f32) as u32;
    let right = staff.right.min(binary.width() as f32) as u32;
    let raw = imageops::crop_imm(binary, left, top, right - left, bottom - top).to_image();
    let raw = imageops::resize(
        &raw,
        prepared.image.width(),
        prepared.image.height(),
        FilterType::Triangle,
    );
    let raw = imageproc::contrast::threshold(&raw, 127, imageproc::contrast::ThresholdType::Binary);
    let labels = imageproc::region_labelling::connected_components(
        &raw,
        imageproc::region_labelling::Connectivity::Four,
        Luma([0]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut components = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32, 0_u64, 0_u64); maximum + 1];
    for (x, y, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let bound = &mut components[label[0] as usize];
        bound.0 = bound.0.min(x);
        bound.1 = bound.1.min(y);
        bound.2 = bound.2.max(x);
        bound.3 = bound.3.max(y);
        bound.4 += 1;
        bound.5 += u64::from(x);
        bound.6 += u64::from(y);
    }
    let mut holes: Vec<(f32, f32, f32)> = Vec::new();
    for (left, top, right, bottom, count, sum_x, sum_y) in components.into_iter().skip(1) {
        let width = (right - left + 1) as f32;
        let height = (bottom - top + 1) as f32;
        if left == 0
            || top == 0
            || right + 1 == raw.width()
            || bottom + 1 == raw.height()
            || !(2.5..=13.0).contains(&width)
            || !(1.0..=8.0).contains(&height)
            || !(4..=65).contains(&count)
            || width < height * 0.8
        {
            continue;
        }
        let x = prepared.left + (sum_x as f32 / count as f32) / prepared.scale;
        let y = prepared.top + (sum_y as f32 / count as f32) / prepared.scale;
        if let Some(hole) = holes.iter_mut().find(|hole| {
            (hole.0 - x).abs() < staff.spacing() * 0.65
                && (hole.1 - y).abs() < staff.spacing() * 0.45
        }) {
            let weight = hole.2 + count as f32;
            hole.0 = (hole.0 * hole.2 + x * count as f32) / weight;
            hole.1 = (hole.1 * hole.2 + y * count as f32) / weight;
            hole.2 = weight;
        } else {
            holes.push((x, y, count as f32));
        }
    }
    holes
        .into_iter()
        .filter(|(x, y, _)| {
            let center_x = (*x - prepared.left) * prepared.scale;
            let center_y = (*y - prepared.top) * prepared.scale;
            prepared
                .find_filtered(
                    &glyph('\u{e0a3}'),
                    &[0.65, 0.75, 0.85, 0.95, 1.05],
                    [
                        center_x - 1.2 * UNIT,
                        center_y - UNIT,
                        center_x + 1.2 * UNIT,
                        center_y + UNIT,
                    ],
                    |bounds| {
                        ((bounds.left + bounds.right) / 2.0 - x).abs() < staff.spacing() * 0.3
                            && ((bounds.top + bounds.bottom) / 2.0 - y).abs()
                                < staff.spacing() * 0.3
                    },
                )
                .is_some_and(|found| found.quality >= 0.66)
        })
        .map(|(x, y, _)| Notehead {
            x,
            y,
            kind: HeadKind::Open,
        })
        .collect()
}

pub(crate) fn recognize_flags(
    binary: &GrayImage,
    staff: &Staff,
    stem_x: i32,
    tip: i32,
    direction: i32,
) -> usize {
    let gap = staff.spacing();
    let mut local = staff.clone();
    local.left = (stem_x as f32 - gap * 2.0).max(staff.left);
    local.right = (stem_x as f32 + gap * 2.0).min(staff.right);
    let image = HeaderImage::region_with_margin(binary, &local, local.right, 5.0);
    let x = (stem_x as f32 - image.left) * image.scale;
    let y = (tip as f32 - image.top) * image.scale;
    // Down-stem flags can lie to the right of the stem as well. Keep both
    // placements, but require the matched glyph to touch this stem's tip.
    let search = if direction < 0 {
        [
            x - 0.3 * UNIT,
            y - 0.3 * UNIT,
            x + 2.0 * UNIT,
            y + 4.8 * UNIT,
        ]
    } else {
        [
            x - 2.0 * UNIT,
            y - 4.8 * UNIT,
            x + 2.0 * UNIT,
            y + 0.3 * UNIT,
        ]
    };
    let mut found = Vec::new();
    for (count, character, heights) in if direction < 0 {
        [
            (1, '\u{e240}', [2.4, 2.8, 3.2]),
            (2, '\u{e242}', [2.8, 3.2, 3.6]),
            (3, '\u{e244}', [3.2, 3.6, 4.0]),
            (4, '\u{e246}', [3.6, 4.0, 4.4]),
        ]
    } else {
        [
            (1, '\u{e241}', [2.4, 2.8, 3.2]),
            (2, '\u{e243}', [2.8, 3.2, 3.6]),
            (3, '\u{e245}', [3.2, 3.6, 4.0]),
            (4, '\u{e247}', [3.6, 4.0, 4.4]),
        ]
    } {
        if let Some(found_match) = image
            .find_filtered(&glyph(character), &heights, search, |bounds| {
                let edge = if direction < 0 {
                    bounds.top
                } else {
                    bounds.bottom
                };
                (edge - tip as f32).abs() <= gap * 0.35
                    && (bounds.left - stem_x as f32)
                        .abs()
                        .min((bounds.right - stem_x as f32).abs())
                        <= gap * 0.4
            })
            .filter(|found| found.quality >= if count == 1 { 0.75 } else { 0.81 })
        {
            found.push((count, found_match.quality));
        }
    }
    found.sort_by(|first, second| second.1.total_cmp(&first.1));
    found
        .first()
        .filter(|first| {
            found
                .get(1)
                .is_none_or(|second| first.1 - second.1 >= 0.025)
        })
        .map(|found| found.0)
        .unwrap_or(0)
}

pub(crate) fn recognize_noteheads(binary: &GrayImage, staff: &Staff) -> Vec<Notehead> {
    let image = HeaderImage::region_with_margin(binary, staff, staff.right, 5.0);
    let holes = hollow_noteheads(binary, staff, &image);
    let mut matches = Vec::new();
    for (kind, character) in [
        (HeadKind::Filled, '\u{e0a4}'),
        (HeadKind::Whole, '\u{e0a2}'),
    ] {
        let source = glyph(character);
        for height in [0.65, 0.75, 0.85, 0.95, 1.05] {
            let head = resized(&source, height * UNIT);
            let mut template = GrayImage::new(head.width() + 4, head.height() + 4);
            imageops::overlay(&mut template, &head, 2, 2);
            if template.width() > image.image.width() || template.height() > image.image.height() {
                continue;
            }
            let scores = match_template(
                &image.image,
                &template,
                MatchTemplateMethod::CrossCorrelationNormalized,
            );
            for (x, y, quality) in scores.enumerate_pixels() {
                let quality = quality[0];
                let minimum = if kind == HeadKind::Filled { 0.82 } else { 0.74 };
                if !quality.is_finite() || quality < minimum {
                    continue;
                }
                let center_x =
                    image.left + (x as f32 + template.width() as f32 / 2.0) / image.scale;
                let center_y =
                    image.top + (y as f32 + template.height() as f32 / 2.0) / image.scale;
                let snapped_y = staff.y_at_pitch(staff.pitch_at(center_y));
                if (center_y - snapped_y).abs() > staff.spacing() * 0.22 {
                    continue;
                }
                if kind == HeadKind::Filled
                    && holes.iter().any(|hole| {
                        (hole.x - center_x).abs() < staff.spacing() * 0.9
                            && (hole.y - center_y).abs() < staff.spacing() * 0.7
                    })
                {
                    continue;
                }
                matches.push((
                    quality,
                    Notehead {
                        x: center_x,
                        y: center_y,
                        kind,
                    },
                ));
            }
        }
    }
    matches.sort_by(|first, second| second.0.total_cmp(&first.0));
    let mut heads: Vec<Notehead> = Vec::new();
    for (_, head) in matches {
        if heads.iter().any(|other| {
            (other.x - head.x).abs() < staff.spacing() * 0.85
                && (other.y - head.y).abs() < staff.spacing() * 0.5
        }) {
            continue;
        }
        heads.push(head);
    }
    for hole in holes {
        if !heads.iter().any(|head| {
            (head.x - hole.x).abs() < staff.spacing() * 0.85
                && (head.y - hole.y).abs() < staff.spacing() * 0.65
        }) {
            heads.push(hole);
        }
    }
    heads
}

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(include_bytes!("../assets/Bravura.otf"))
            .expect("embedded Bravura font")
    })
}

fn glyph(character: char) -> GrayImage {
    let font = font();
    let outline = font
        .outline_glyph(font.glyph_id(character).with_scale(256.0))
        .expect("SMuFL glyph outline");
    let bounds = outline.px_bounds();
    let mut image = GrayImage::new(bounds.width().ceil() as u32, bounds.height().ceil() as u32);
    outline.draw(|x, y, coverage| {
        image.put_pixel(x, y, Luma([if coverage >= 0.5 { 255 } else { 0 }]))
    });
    image
}

fn resized(image: &GrayImage, height: f32) -> GrayImage {
    let height = height.round().max(1.0) as u32;
    let width = ((image.width() as f32 / image.height() as f32) * height as f32)
        .round()
        .max(1.0) as u32;
    imageops::resize(image, width, height, FilterType::Triangle)
}

fn number(value: u8) -> GrayImage {
    let digits: Vec<_> = value
        .to_string()
        .chars()
        .map(|digit| {
            let character = char::from_u32(0xe080 + digit.to_digit(10).unwrap()).unwrap();
            resized(&glyph(character), 30.0)
        })
        .collect();
    let width = digits.iter().map(GrayImage::width).sum::<u32>() + digits.len() as u32 - 1;
    let mut image = GrayImage::new(width, 30);
    let mut left = 0;
    for digit in digits {
        imageops::overlay(&mut image, &digit, left, 0);
        left += i64::from(digit.width()) + 1;
    }
    image
}

fn meter_template(meter: TimeSignature) -> GrayImage {
    match meter {
        TimeSignature::Common => glyph('\u{e08a}'),
        TimeSignature::Cut => glyph('\u{e08b}'),
        TimeSignature::Meter { beats, beat_unit } => {
            let top = number(beats);
            let bottom = number(beat_unit);
            let width = top.width().max(bottom.width());
            let mut image = GrayImage::new(width, 62);
            imageops::overlay(&mut image, &top, i64::from((width - top.width()) / 2), 0);
            imageops::overlay(
                &mut image,
                &bottom,
                i64::from((width - bottom.width()) / 2),
                32,
            );
            image
        }
    }
}

struct Match {
    quality: f32,
    bounds: SymbolBounds,
}

struct HeaderImage {
    image: GrayImage,
    left: f32,
    top: f32,
    scale: f32,
    staff_top: f32,
}

impl HeaderImage {
    fn region(binary: &GrayImage, staff: &Staff, right: f32) -> Self {
        Self::region_with_margin(binary, staff, right, 2.0)
    }

    fn region_with_margin(binary: &GrayImage, staff: &Staff, right: f32, margin: f32) -> Self {
        let gap = staff.spacing();
        let left = staff.left.max(0.0) as u32;
        let top = (staff.lines[0] - margin * gap).max(0.0) as u32;
        let right = right.min(staff.right).min(binary.width() as f32) as u32;
        let bottom = (staff.lines[4] + margin * gap).min(binary.height() as f32) as u32;
        let source = imageops::crop_imm(binary, left, top, right - left, bottom - top).to_image();
        let mut ink = source.clone();
        let offset = (gap * 0.2).ceil() as u32;
        for (x, y, pixel) in ink.enumerate_pixels_mut() {
            let line = staff
                .lines
                .iter()
                .any(|line| (*line - (top + y) as f32).abs() <= (gap * 0.12).max(1.0))
                || (margin > 2.0 && {
                    let relative = ((top + y) as f32 - staff.lines[0]) / gap;
                    (relative - relative.round()).abs() * gap <= (gap * 0.12).max(1.0)
                });
            let crossing = y >= offset
                && y + offset < source.height()
                && if margin > 2.0 {
                    source.get_pixel(x, y - offset)[0] == 0
                        || source.get_pixel(x, y + offset)[0] == 0
                } else {
                    source.get_pixel(x, y - offset)[0] == 0
                        && source.get_pixel(x, y + offset)[0] == 0
                };
            pixel[0] = if source.get_pixel(x, y)[0] == 0 && (!line || crossing) {
                255
            } else {
                0
            };
        }
        let scale = UNIT / gap;
        let image = imageops::resize(
            &ink,
            (ink.width() as f32 * scale).round().max(1.0) as u32,
            (ink.height() as f32 * scale).round().max(1.0) as u32,
            FilterType::Triangle,
        );
        Self {
            image,
            left: left as f32,
            top: top as f32,
            scale,
            staff_top: (staff.lines[0] - top as f32) * scale,
        }
    }

    fn find(&self, template: &GrayImage, heights: &[f32], search: [f32; 4]) -> Option<Match> {
        self.find_filtered(template, heights, search, |_| true)
    }

    fn find_filtered(
        &self,
        template: &GrayImage,
        heights: &[f32],
        search: [f32; 4],
        accept: impl Fn(&SymbolBounds) -> bool,
    ) -> Option<Match> {
        let left = search[0].max(0.0) as u32;
        let top = search[1].max(0.0) as u32;
        let right = search[2].min(self.image.width() as f32).max(0.0) as u32;
        let bottom = search[3].min(self.image.height() as f32).max(0.0) as u32;
        if right <= left || bottom <= top {
            return None;
        }
        let source =
            imageops::crop_imm(&self.image, left, top, right - left, bottom - top).to_image();
        if source.pixels().all(|pixel| pixel[0] == 0) {
            return None;
        }
        let mut best: Option<Match> = None;
        for &height in heights {
            let template = resized(template, height * UNIT);
            if template.width() > source.width() || template.height() > source.height() {
                continue;
            }
            let scores = match_template(
                &source,
                &template,
                MatchTemplateMethod::CrossCorrelationNormalized,
            );
            for (x, y, quality) in scores.enumerate_pixels() {
                let quality = quality[0];
                if !quality.is_finite() || best.as_ref().is_some_and(|best| best.quality >= quality)
                {
                    continue;
                }
                let bounds = SymbolBounds {
                    left: self.left + (left + x) as f32 / self.scale,
                    top: self.top + (top + y) as f32 / self.scale,
                    right: self.left + (left + x + template.width()) as f32 / self.scale,
                    bottom: self.top + (top + y + template.height()) as f32 / self.scale,
                };
                if accept(&bounds) {
                    best = Some(Match { quality, bounds });
                }
            }
        }
        best
    }
}

fn enclosed_spaces(image: &GrayImage) -> usize {
    let image =
        imageproc::contrast::threshold(image, 127, imageproc::contrast::ThresholdType::Binary);
    let labels = imageproc::region_labelling::connected_components(
        &image,
        imageproc::region_labelling::Connectivity::Four,
        Luma([255]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut regions = vec![(0, false); maximum + 1];
    for (column, row, label) in labels.enumerate_pixels() {
        if label[0] != 0 {
            let region = &mut regions[label[0] as usize];
            region.0 += 1;
            region.1 |=
                column == 0 || row == 0 || column + 1 == image.width() || row + 1 == image.height();
        }
    }
    regions
        .iter()
        .filter(|(area, edge)| *area >= 2 && !edge)
        .count()
}

fn recognize_multimeasure_rests(
    binary: &GrayImage,
    staff: &Staff,
    staff_index: usize,
) -> Vec<Rest> {
    let gap = staff.spacing();
    let middle = staff.lines[2];
    let ink = |column: i32, row: f32| {
        column >= 0
            && column < binary.width() as i32
            && row >= 0.0
            && row.round() < binary.height() as f32
            && binary.get_pixel(column as u32, row.round() as u32)[0] == 0
    };
    let mut runs: Vec<(i32, i32)> = Vec::new();
    for column in staff.left.ceil() as i32..=staff.right.floor() as i32 {
        if [-0.13, 0.0, 0.13]
            .iter()
            .all(|offset| ink(column, middle + gap * offset))
        {
            if let Some(last) = runs.last_mut()
                && column - last.1 <= 1 + (gap * 0.08).floor() as i32
            {
                last.1 = column;
            } else {
                runs.push((column, column));
            }
        }
    }
    let mut rests = Vec::new();
    for (left, right) in runs {
        if !(3.0..=24.0).contains(&((right - left) as f32 / gap)) {
            continue;
        }
        let cap = |edge: i32| {
            let radius = (gap * 0.2).ceil() as i32;
            (edge - radius..=edge + radius).any(|column| {
                (0..=20)
                    .filter(|sample| ink(column, staff.lines[1] + gap * *sample as f32 / 10.0))
                    .count()
                    >= 19
                    && !ink(column, staff.lines[0] + gap * 0.45)
                    && !ink(column, staff.lines[0] + gap * 3.55)
            })
        };
        if !cap(left) || !cap(right) {
            continue;
        }
        let center = (left + right) as f32 / 2.0;
        let mut local = staff.clone();
        local.left = (center - gap * 2.0).max(staff.left);
        local.right = (center + gap * 2.0).min(staff.right);
        let mut image = HeaderImage::region_with_margin(binary, &local, local.right, 3.0);
        let mut source = imageops::crop_imm(
            binary,
            image.left as u32,
            image.top as u32,
            (local.right as u32 - image.left as u32).max(1),
            ((staff.lines[0] - image.top).floor() as u32).max(1),
        )
        .to_image();
        imageops::invert(&mut source);
        image.image = imageops::resize(
            &source,
            (source.width() as f32 * image.scale).round().max(1.0) as u32,
            (source.height() as f32 * image.scale).round().max(1.0) as u32,
            FilterType::Triangle,
        );
        let mut best: Option<(u8, Match)> = None;
        for count in 2..=99 {
            let digits = number(count);
            let expected_holes = enclosed_spaces(&digits);
            let mut template = GrayImage::new(digits.width() + 8, digits.height() + 8);
            imageops::overlay(&mut template, &digits, 4, 4);
            let found = image.find_filtered(
                &template,
                &[
                    1.8, 1.9, 2.0, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 3.0,
                ],
                [
                    0.0,
                    image.staff_top - 3.0 * UNIT,
                    image.image.width() as f32,
                    image.staff_top,
                ],
                |bounds| {
                    if ((bounds.left + bounds.right) / 2.0 - center).abs() >= gap * 0.4 {
                        return false;
                    }
                    let left = ((bounds.left - image.left) * image.scale).round() as u32;
                    let top = ((bounds.top - image.top) * image.scale).round() as u32;
                    let right = ((bounds.right - image.left) * image.scale).round() as u32;
                    let bottom = ((bounds.bottom - image.top) * image.scale).round() as u32;
                    let region = imageops::crop_imm(
                        &image.image,
                        left,
                        top,
                        right.min(image.image.width()) - left,
                        bottom.min(image.image.height()) - top,
                    )
                    .to_image();
                    let touches_edge = (0..region.width()).any(|column| {
                        region.get_pixel(column, 0)[0] > 127
                            || region.get_pixel(column, region.height() - 1)[0] > 127
                    }) || (0..region.height()).any(|row| {
                        region.get_pixel(0, row)[0] > 127
                            || region.get_pixel(region.width() - 1, row)[0] > 127
                    });
                    !touches_edge && {
                        let actual_holes = enclosed_spaces(&region);
                        actual_holes == expected_holes || (actual_holes < 2 && expected_holes < 2)
                    }
                },
            );
            if let Some(found) = found.filter(|found| found.quality >= 0.72)
                && best
                    .as_ref()
                    .is_none_or(|(_, previous)| found.quality > previous.quality)
            {
                best = Some((count, found));
            }
        }
        if let Some((count, found)) = best {
            rests.push(Rest {
                x: center,
                y: middle,
                staff: staff_index,
                duration: Duration::Whole,
                dots: 0,
                measures: Some(count),
                confirmed: false,
                bounds: Some(SymbolBounds {
                    left: left as f32 - gap * 0.3,
                    right: right as f32 + gap * 0.3,
                    top: found.bounds.top - gap * 0.3,
                    bottom: staff.lines[3] + gap * 0.3,
                }),
            });
        }
    }
    rests
}

pub fn recognize_rests(binary: &GrayImage, staff: &Staff, staff_index: usize) -> Vec<Rest> {
    let image = HeaderImage::region(binary, staff, staff.right);
    let extended = HeaderImage::region_with_margin(binary, staff, staff.right, 3.0);
    let mut matches = Vec::new();
    for (duration, character, heights, position) in [
        (Duration::Whole, '\u{e4e3}', 3..=5, 1.2),
        (Duration::Half, '\u{e4e4}', 3..=5, 1.5),
        (Duration::Quarter, '\u{e4e5}', 26..=32, 0.5),
        (Duration::Eighth, '\u{e4e6}', 15..=23, 1.0),
        (Duration::Sixteenth, '\u{e4e7}', 23..=35, 0.5),
        (Duration::ThirtySecond, '\u{e4e8}', 31..=43, 0.0),
        (Duration::SixtyFourth, '\u{e4e9}', 40..=55, -0.5),
    ] {
        // Tall, displaced rests need room below the staff. Keep the original
        // staff-line cleanup for the rectangular whole- and half-rest shapes.
        let image = if duration.flag_count() > 0 {
            &extended
        } else {
            &image
        };
        let template = glyph(character);
        for height in heights {
            let height = height as f32 / 10.0;
            let templates = if matches!(duration, Duration::Whole | Duration::Half) {
                [10, 12, 14, 16]
                    .into_iter()
                    .map(|width| {
                        GrayImage::from_pixel(width, (height * UNIT).round() as u32, Luma([255]))
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![resized(&template, height * UNIT)]
            };
            for template in templates {
                if template.width() > image.image.width()
                    || template.height() > image.image.height()
                {
                    continue;
                }
                let top = (image.staff_top + (position - 0.35) * UNIT).max(0.0) as u32;
                let displacement = if duration.flag_count() > 0 { 2.0 } else { 0.0 };
                let bottom = ((image.staff_top + (position + displacement + height + 0.35) * UNIT)
                    as u32)
                    .min(image.image.height());
                if bottom - top < template.height() {
                    continue;
                }
                let source =
                    imageops::crop_imm(&image.image, 0, top, image.image.width(), bottom - top)
                        .to_image();
                let scores = match_template(
                    &source,
                    &template,
                    MatchTemplateMethod::CrossCorrelationNormalized,
                );
                for (x, y, quality) in scores.enumerate_pixels() {
                    let quality = quality[0];
                    let minimum = match duration.flag_count() {
                        1 => 0.78,
                        2..=4 => 0.82,
                        _ => 0.86,
                    };
                    if !quality.is_finite() || quality < minimum {
                        continue;
                    }
                    let bounds = SymbolBounds {
                        left: image.left + x as f32 / image.scale,
                        top: image.top + (top + y) as f32 / image.scale,
                        right: image.left + (x + template.width()) as f32 / image.scale,
                        bottom: image.top + (top + y + template.height()) as f32 / image.scale,
                    };
                    let center = (
                        (bounds.left + bounds.right) / 2.0,
                        (bounds.top + bounds.bottom) / 2.0,
                    );
                    let relative_top = (bounds.top - staff.lines[0]) / staff.spacing();
                    if (duration == Duration::Whole && !(0.95..=1.3).contains(&relative_top))
                        || (duration == Duration::Half && !(1.4..=1.7).contains(&relative_top))
                    {
                        continue;
                    }
                    if [&staff.header.clef_bounds, &staff.header.time_bounds]
                        .into_iter()
                        .flatten()
                        .chain(staff.header.key_bounds.iter())
                        .any(|symbol| symbol.contains(center.0, center.1))
                    {
                        continue;
                    }
                    let mut outside = 0;
                    let margin = if matches!(duration, Duration::Whole | Duration::Half) {
                        4
                    } else {
                        2
                    };
                    for border_y in (top + y).saturating_sub(margin)
                        ..(top + y + template.height() + margin).min(image.image.height())
                    {
                        for border_x in x.saturating_sub(margin)
                            ..(x + template.width() + margin).min(image.image.width())
                        {
                            if (border_x < x
                                || border_x >= x + template.width()
                                || border_y < top + y
                                || border_y >= top + y + template.height())
                                && image.image.get_pixel(border_x, border_y)[0] > 127
                            {
                                outside += 1;
                            }
                        }
                    }
                    if outside > if duration == Duration::Eighth { 7 } else { 3 } {
                        continue;
                    }
                    matches.push((quality, duration, bounds));
                }
            }
        }
    }
    // A short template can match only the lower hooks of a longer rest.
    // Require comparable quality so a weak, oversized match cannot replace
    // a clearly recognized shorter rest.
    let complete_rests: Vec<_> = matches
        .iter()
        .filter(|(_, duration, _)| duration.flag_count() >= 2)
        .map(|(quality, duration, bounds)| (*quality, *duration, bounds.clone()))
        .collect();
    matches.retain(|(quality, duration, bounds)| {
        duration.flag_count() == 0
            || !complete_rests
                .iter()
                .any(|(full_quality, full_duration, full)| {
                    full_duration.flag_count() > duration.flag_count()
                        && *full_quality >= *quality - 0.03
                        && ((full.left + full.right - bounds.left - bounds.right) / 2.0).abs()
                            < staff.spacing() * 0.8
                        && bounds.top >= full.top - staff.spacing() * 0.25
                        && bounds.bottom < full.bottom + staff.spacing() * 0.25
                })
    });
    matches.sort_by(|first, second| second.0.total_cmp(&first.0));
    let mut rests = recognize_multimeasure_rests(binary, staff, staff_index);
    for (_, duration, bounds) in matches {
        let x = (bounds.left + bounds.right) / 2.0;
        let y = (bounds.top + bounds.bottom) / 2.0;
        if rests
            .iter()
            .any(|rest| (rest.x - x).abs() < staff.spacing() * 1.3)
        {
            continue;
        }
        rests.push(Rest {
            x,
            y,
            staff: staff_index,
            duration,
            dots: 0,
            measures: None,
            confirmed: false,
            bounds: Some(bounds),
        });
    }
    rests.sort_by(|first, second| first.x.total_cmp(&second.x));
    rests
}

fn align_local_staff(binary: &GrayImage, staff: &mut Staff) {
    // Page-wide row peaks can miss the lines beside a clef or meter when a
    // scan is slightly curved. Use the nearby horizontal ink as the guide.
    let gap = staff.spacing();
    let left = staff.left.max(0.0) as u32;
    let right = staff
        .right
        .min(staff.left + gap * 6.0)
        .min(binary.width() as f32) as u32;
    if right <= left {
        return;
    }
    for line in &mut staff.lines {
        let top = (*line - gap * 0.4).max(0.0) as u32;
        let bottom = ((*line + gap * 0.4).ceil() as u32).min(binary.height());
        let profile: Vec<_> = (top..bottom)
            .map(|y| {
                (left..right)
                    .filter(|&x| binary.get_pixel(x, y)[0] == 0)
                    .count()
            })
            .collect();
        let Some((peak, &maximum)) = profile.iter().enumerate().max_by_key(|(_, ink)| *ink) else {
            continue;
        };
        if (maximum as f32) < (right - left) as f32 * 0.7 {
            continue;
        }
        let mut first = peak;
        let mut last = peak;
        while first > 0 && profile[first - 1] as f32 >= maximum as f32 * 0.85 {
            first -= 1;
        }
        while last + 1 < profile.len() && profile[last + 1] as f32 >= maximum as f32 * 0.85 {
            last += 1;
        }
        *line = top as f32 + (first + last) as f32 * 0.5;
    }
}

pub fn recognize_header(binary: &GrayImage, staff: &Staff) -> StaffHeader {
    let header = recognize_header_at(binary, staff);
    if header.clef != Clef::Unknown || staff.spacing() < 4.0 {
        return header;
    }
    // A page-wide line peak can miss the slightly curved opening lines and
    // place staff.left inside the clef. Recover the nearby edge from at least
    // four lines only after the normal header searches have failed. Keep the
    // original staff geometry for note recognition and its sampling grid.
    let gap = staff.spacing();
    let left = (staff.left - gap * 3.0).max(0.0) as u32;
    let right = staff.left.min(binary.width() as f32) as u32;
    let Some(left) = (left..right).find(|&x| {
        staff
            .lines
            .iter()
            .filter(|&&line| {
                ((line - gap * 0.35).max(0.0) as u32
                    ..=((line + gap * 0.35) as u32).min(binary.height() - 1))
                    .any(|y| binary.get_pixel(x, y)[0] == 0)
            })
            .count()
            >= 4
    }) else {
        return header;
    };
    let mut local = staff.clone();
    local.left = left as f32;
    recognize_header_at(binary, &local)
}

fn recognize_header_at(binary: &GrayImage, staff: &Staff) -> StaffHeader {
    let header = StaffHeader {
        clef: Clef::Unknown,
        ..Default::default()
    };
    if staff.spacing() < 4.0 || staff.right - staff.left < staff.spacing() * 3.0 {
        return header;
    }
    // A taller crop also uses the stroke-preserving cleanup needed for thin
    // curves crossing a staff line. Retry only when the normal crop failed.
    for margin in [2.0, 2.5] {
        for align in [false, true] {
            for padding in [0.0, 0.75] {
                let mut local = staff.clone();
                local.left = (staff.left - padding * staff.spacing()).max(0.0);
                local.right = (staff.left + 20.0 * staff.spacing()).min(staff.right);
                if align {
                    align_local_staff(binary, &mut local);
                }
                let image = HeaderImage::region_with_margin(binary, &local, local.right, margin);
                let header = recognize_header_image(&image, &local);
                if header.clef != Clef::Unknown {
                    return header;
                }
            }
        }
    }
    header
}

fn recognize_header_image(image: &HeaderImage, staff: &Staff) -> StaffHeader {
    let mut header = StaffHeader {
        clef: Clef::Unknown,
        ..Default::default()
    };
    let top = image.staff_top;
    let bass = image.find(
        &glyph('\u{e062}'),
        &[
            3.0, 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 4.0, 4.1, 4.2,
        ],
        [0.0, top - 0.5 * UNIT, 6.0 * UNIT, top + 4.5 * UNIT],
    );
    // Engraving fonts can have a narrower treble clef than Bravura. Compare
    // these proportions at the same confidence threshold instead of accepting
    // a weaker match to the wrong shape.
    let treble_template = glyph('\u{e050}');
    let treble = [1.0, 0.95, 0.9, 0.85, 0.8]
        .into_iter()
        .filter_map(|width_scale| {
            let template = imageops::resize(
                &treble_template,
                (treble_template.width() as f32 * width_scale) as u32,
                treble_template.height(),
                FilterType::Triangle,
            );
            image.find(
                &template,
                &[
                    6.0, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6, 6.7, 6.8, 6.9, 7.0, 7.1, 7.2, 7.3, 7.4, 7.5,
                    7.6, 7.7, 7.8,
                ],
                [0.0, top - 2.0 * UNIT, 6.0 * UNIT, top + 6.0 * UNIT],
            )
        })
        .max_by(|first, second| first.quality.total_cmp(&second.quality));
    let mut clefs: Vec<_> = [(Clef::Bass, bass), (Clef::Treble, treble)]
        .into_iter()
        .filter_map(|(clef, found)| found.map(|found| (clef, found)))
        .collect();
    clefs.sort_by(|first, second| second.1.quality.total_cmp(&first.1.quality));
    let Some((clef, found)) = clefs.first() else {
        return header;
    };
    if found.quality < 0.78
        || clefs
            .get(1)
            .is_some_and(|second| found.quality - second.1.quality < 0.04)
    {
        return header;
    }
    header.clef = *clef;
    header.clef_bounds = Some(found.bounds.clone());
    let left = (found.bounds.right - image.left) * image.scale + 0.15 * UNIT;
    // Bound the header at a narrow, connected barline. A vertical stroke
    // inside a stacked number must not truncate the opening time signature.
    let ink = imageproc::contrast::threshold(
        &image.image,
        64,
        imageproc::contrast::ThresholdType::Binary,
    );
    let labels = imageproc::region_labelling::connected_components(
        &ink,
        imageproc::region_labelling::Connectivity::Eight,
        Luma([0]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut components = vec![(u32::MAX, u32::MAX, 0, 0); maximum + 1];
    for (x, y, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let bounds = &mut components[label[0] as usize];
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    }
    let meter_right = components
        .into_iter()
        .skip(1)
        .filter(|&(x0, y0, x1, y1)| {
            x0 as f32 > left + UNIT * 0.3
                && (x1 - x0 + 1) as f32 <= UNIT * 0.5
                && y0 as f32 <= image.staff_top + UNIT * 0.3
                && y1 as f32 >= image.staff_top + UNIT * 3.7
        })
        .map(|(x0, _, _, _)| x0 as f32)
        .fold(20.0 * UNIT, f32::min);
    let mut meters = Vec::new();
    for meter in [
        TimeSignature::Common,
        TimeSignature::Cut,
        TimeSignature::Meter {
            beats: 2,
            beat_unit: 2,
        },
        TimeSignature::Meter {
            beats: 2,
            beat_unit: 4,
        },
        TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        },
        TimeSignature::Meter {
            beats: 4,
            beat_unit: 4,
        },
        TimeSignature::Meter {
            beats: 3,
            beat_unit: 8,
        },
        TimeSignature::Meter {
            beats: 6,
            beat_unit: 8,
        },
        TimeSignature::Meter {
            beats: 9,
            beat_unit: 8,
        },
        TimeSignature::Meter {
            beats: 12,
            beat_unit: 8,
        },
    ] {
        let (heights, search): (&[f32], _) = match meter {
            TimeSignature::Common => (
                &[1.7, 1.9, 2.1, 2.3],
                [left, top + 0.5 * UNIT, meter_right, top + 3.5 * UNIT],
            ),
            TimeSignature::Cut => (
                &[2.8, 3.1, 3.4],
                [left, top + 0.1 * UNIT, meter_right, top + 3.9 * UNIT],
            ),
            _ => (
                &[3.6, 3.9, 4.2],
                [left, top - 0.4 * UNIT, meter_right, top + 4.4 * UNIT],
            ),
        };
        if let Some(found) = image.find(&meter_template(meter), heights, search) {
            meters.push((meter, found));
        }
    }
    meters.sort_by(|first, second| second.1.quality.total_cmp(&first.1.quality));

    if let Some((meter, found)) = meters.first()
        && found.quality >= 0.82
        && meters
            .get(1)
            .is_none_or(|second| found.quality - second.1.quality >= 0.035)
    {
        header.time_signature = Some(*meter);
        header.time_bounds = Some(found.bounds.clone());
    }
    let (key_signature, key_bounds) = recognize_key(image, staff, &header);
    header.key_signature = key_signature;
    header.key_bounds = key_bounds;
    header
}

fn recognize_key(
    image: &HeaderImage,
    staff: &Staff,
    header: &StaffHeader,
) -> (Option<KeySignature>, Vec<SymbolBounds>) {
    let Some(clef) = &header.clef_bounds else {
        return (None, vec![]);
    };
    let gap = staff.spacing();
    let end = header
        .time_bounds
        .as_ref()
        .map(|bounds| (bounds.left - image.left) * image.scale)
        .unwrap_or(20.0 * UNIT);
    let mut candidates = Vec::new();
    for (sign, character, positions, anchor, heights) in [
        (
            1,
            '\u{e262}',
            [0.0, 1.5, -0.5, 1.0, 2.5, 0.5, 2.0],
            0.5,
            [2.4, 2.7, 3.0],
        ),
        (
            -1,
            '\u{e260}',
            [2.0, 0.5, 2.5, 1.0, 3.0, 1.5, 3.5],
            0.78,
            [2.3, 2.6, 2.9],
        ),
    ] {
        let template = glyph(character);
        let mut left = (clef.right - image.left) * image.scale + 0.05 * UNIT;
        let mut bounds = Vec::new();
        let mut quality = 0.0;
        for mut position in positions {
            if header.clef == Clef::Bass {
                position += 1.0;
            }
            let pitch_y = staff.lines[0] + position * gap;
            let expected = (pitch_y - image.top) * image.scale;
            let search = [
                left,
                expected - (anchor * 3.0 + 0.35) * UNIT,
                (left + 3.0 * UNIT).min(end),
                expected + ((1.0 - anchor) * 3.0 + 0.35) * UNIT,
            ];
            let found = image.find_filtered(&template, &heights, search, |candidate| {
                (candidate.top + (candidate.bottom - candidate.top) * anchor - pitch_y).abs()
                    <= gap * 0.35
                    && bounds.last().is_none_or(|previous: &SymbolBounds| {
                        candidate.left - previous.right <= gap * 0.9
                    })
            });
            // Following signs have a prescribed pitch and must be adjacent to
            // a confidently identified first sign. Staff-line removal can
            // damage their strokes differently at each vertical position.
            let minimum = if bounds.is_empty() { 0.83 } else { 0.68 };
            let Some(found) = found.filter(|found| found.quality >= minimum) else {
                break;
            };
            left = (found.bounds.right - image.left) * image.scale + 0.05 * UNIT;
            quality += found.quality;
            bounds.push(found.bounds);
        }
        if !bounds.is_empty() {
            candidates.push((
                quality / bounds.len() as f32,
                KeySignature {
                    fifths: sign * bounds.len() as i8,
                },
                bounds,
            ));
        }
    }
    candidates.sort_by(|first, second| second.0.total_cmp(&first.0));
    if candidates.len() > 1 && candidates[0].0 - candidates[1].0 < 0.04 {
        return (None, vec![]);
    }
    if candidates.is_empty() && header.time_bounds.is_some() {
        let left = ((clef.right - image.left) * image.scale + UNIT * 0.15)
            .ceil()
            .max(0.0) as u32;
        let right = (end - UNIT * 0.15)
            .floor()
            .min(image.image.width() as f32)
            .max(0.0) as u32;
        // Only an observed empty interval between two recognized symbols
        // establishes zero fifths. Missing/unreadable signs remain unknown.
        if right > left + (UNIT * 0.4) as u32 {
            let ink = (left..right)
                .flat_map(|x| (0..image.image.height()).map(move |y| (x, y)))
                .filter(|&(x, y)| image.image.get_pixel(x, y)[0] > 127)
                .count();
            if ink <= (UNIT * 0.5) as usize {
                return (Some(KeySignature { fifths: 0 }), vec![]);
            }
        }
    }
    candidates
        .into_iter()
        .next()
        .map(|(_, key, bounds)| (Some(key), bounds))
        .unwrap_or((None, vec![]))
}

fn key_sequence(
    image: &HeaderImage,
    staff: &Staff,
    mut left: f32,
    sequence: &[(char, f32)],
) -> Option<(f32, Vec<SymbolBounds>)> {
    let mut bounds: Vec<SymbolBounds> = Vec::new();
    let mut quality = 0.0;
    for &(character, position) in sequence {
        let anchor = if character == '\u{e260}' { 0.78 } else { 0.5 };
        let position = position
            + if staff.header.clef == Clef::Bass {
                1.0
            } else {
                0.0
            };
        let pitch_y = staff.lines[0] + position * staff.spacing();
        let expected = (pitch_y - image.top) * image.scale;
        let found = image.find_filtered(
            &glyph(character),
            &[2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 3.0],
            [
                left,
                expected - (anchor * 3.0 + 0.35) * UNIT,
                left + 3.0 * UNIT,
                expected + ((1.0 - anchor) * 3.0 + 0.35) * UNIT,
            ],
            |candidate| {
                (candidate.top + (candidate.bottom - candidate.top) * anchor - pitch_y).abs()
                    <= staff.spacing() * 0.35
                    && bounds.last().is_none_or(|previous| {
                        candidate.left - previous.right < staff.spacing() * 0.9
                    })
            },
        )?;
        if found.quality < 0.83 {
            return None;
        }
        left = (found.bounds.right - image.left) * image.scale + 0.05 * UNIT;
        quality += found.quality;
        bounds.push(found.bounds);
    }
    Some((quality / sequence.len() as f32, bounds))
}

pub(crate) fn recognize_time_changes(
    binary: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    bars: &[Barline],
) -> Vec<TimeChange> {
    static TEMPLATES: OnceLock<Vec<(TimeSignature, GrayImage)>> = OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| {
        [TimeSignature::Common, TimeSignature::Cut]
            .into_iter()
            .chain((1..=32).flat_map(|beats| {
                [1, 2, 4, 8, 16, 32]
                    .into_iter()
                    .map(move |beat_unit| TimeSignature::Meter { beats, beat_unit })
            }))
            .map(|meter| (meter, meter_template(meter)))
            .collect()
    });
    let gap = staff.spacing();
    let header_end = [&staff.header.clef_bounds, &staff.header.time_bounds]
        .into_iter()
        .flatten()
        .chain(staff.header.key_bounds.iter())
        .map(|bounds| bounds.right)
        .fold(staff.left, f32::max);
    let mut starts = vec![(header_end + gap * 0.2, false)];
    starts.extend(
        bars.iter()
            .filter(|bar| bar.staff == staff_index)
            .map(|bar| (bar.x + gap * 0.2, true)),
    );
    starts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut changes: Vec<TimeChange> = Vec::new();
    for (left, after_bar) in starts {
        if staff.right - left < gap * 1.5 {
            continue;
        }
        let mut local = staff.clone();
        local.left = left;
        local.right = (left + gap * 5.0).min(staff.right);
        align_local_staff(binary, &mut local);
        let image = HeaderImage::region(binary, &local, local.right);
        let top = image.staff_top.round() as u32;
        let bottom = (image.staff_top + 4.0 * UNIT).round() as u32;
        let bar_columns: Vec<_> = (0..image.image.width())
            .map(|x| {
                let ink = (top..=bottom.min(image.image.height() - 1))
                    .filter(|&y| {
                        (x.saturating_sub(1)..=(x + 1).min(image.image.width() - 1))
                            .any(|near| image.image.get_pixel(near, y)[0] >= 64)
                    })
                    .count();
                // Only tolerate remnants of the known boundary at the left
                // edge. Ink from another digit still rejects partial numbers.
                after_bar && x as f32 <= UNIT * 0.3 && ink as f32 >= (bottom - top + 1) as f32 * 0.9
            })
            .collect();
        let mut candidates = Vec::new();
        for (meter, template) in templates {
            let heights: &[f32] = match meter {
                TimeSignature::Common => &[1.7, 1.9, 2.1, 2.3],
                TimeSignature::Cut => &[2.8, 3.1, 3.4],
                _ => &[3.6, 3.7, 3.8, 3.9, 4.0, 4.1, 4.2],
            };
            if let Some(found) = image
                .find_filtered(
                    template,
                    heights,
                    [
                        0.0,
                        image.staff_top - 0.4 * UNIT,
                        image.image.width() as f32,
                        image.staff_top + 4.4 * UNIT,
                    ],
                    |bounds| {
                        if ((bounds.top + bounds.bottom) * 0.5 - local.lines[2]).abs() > gap * 0.3 {
                            return false;
                        }
                        // Reject a single digit matched inside a wider number.
                        let x0 = ((bounds.left - image.left) * image.scale).round() as u32;
                        let x1 = ((bounds.right - image.left) * image.scale).round() as u32;
                        let y0 = ((bounds.top - image.top) * image.scale).round() as u32;
                        let y1 = ((bounds.bottom - image.top) * image.scale).round() as u32;
                        let mut outside = 0;
                        for y in y0.saturating_sub(2)..(y1 + 2).min(image.image.height()) {
                            for x in x0.saturating_sub(2)..(x1 + 2).min(image.image.width()) {
                                if (x < x0 || x >= x1 || y < y0 || y >= y1)
                                    && image.image.get_pixel(x, y)[0] > 127
                                    && !((x < x0 || x >= x1) && bar_columns[x as usize])
                                {
                                    outside += 1;
                                }
                            }
                        }
                        outside <= 4
                    },
                )
                .filter(|found| found.quality >= 0.86)
            {
                candidates.push((*meter, found));
            }
        }
        candidates.sort_by(|a, b| b.1.quality.total_cmp(&a.1.quality));
        if let Some((meter, found)) = candidates.first()
            && candidates
                .get(1)
                .is_none_or(|second| found.quality - second.1.quality >= 0.025)
            && !staff
                .header
                .time_bounds
                .as_ref()
                .is_some_and(|bounds| bounds.contains(found.bounds.left, found.bounds.top))
            && !changes
                .iter()
                .any(|change| (change.x - found.bounds.left).abs() < gap)
        {
            changes.push(TimeChange {
                staff: staff_index,
                x: found.bounds.left,
                time_signature: *meter,
                bounds: vec![found.bounds.clone()],
                confirmed: false,
            });
        }
    }
    changes
}

pub(crate) fn recognize_key_changes(
    binary: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    bars: &[Barline],
) -> Vec<KeyChange> {
    if staff.header.clef == Clef::Unknown {
        return vec![];
    }
    let mut previous = staff.header.key_signature.unwrap_or_default();
    let positions = |fifths: i8| {
        if fifths < 0 {
            [2.0, 0.5, 2.5, 1.0, 3.0, 1.5, 3.5]
        } else {
            [0.0, 1.5, -0.5, 1.0, 2.5, 0.5, 2.0]
        }
    };
    let mut bars: Vec<_> = bars.iter().filter(|bar| bar.staff == staff_index).collect();
    bars.sort_by(|first, second| first.x.total_cmp(&second.x));
    let mut changes = Vec::new();
    for bar in bars {
        if staff.right - bar.x < staff.spacing() * 3.0 {
            continue;
        }
        let mut local = staff.clone();
        local.left = bar.x + staff.spacing() * 0.15;
        local.right = (bar.x + staff.spacing() * 15.0).min(staff.right);
        let image = HeaderImage::region(binary, &local, local.right);
        let mut candidates = Vec::new();
        for fifths in -7_i8..=7 {
            if fifths == previous.fifths {
                continue;
            }
            let old = positions(previous.fifths);
            let next = positions(fifths);
            let removed = if fifths.signum() == previous.fifths.signum() {
                fifths.unsigned_abs() as usize
            } else {
                0
            };
            let old_count = previous.fifths.unsigned_abs() as usize;
            let mut cancellations = vec![vec![]];
            if old_count > removed {
                cancellations.push(old[removed..old_count].to_vec());
                if removed > 0 {
                    cancellations.push(old[..old_count].to_vec());
                }
            }
            for cancellation in cancellations {
                let mut sequence: Vec<_> = cancellation
                    .into_iter()
                    .map(|position| ('\u{e261}', position))
                    .collect();
                sequence.extend(
                    next[..fifths.unsigned_abs() as usize]
                        .iter()
                        .map(|position| {
                            (if fifths < 0 { '\u{e260}' } else { '\u{e262}' }, *position)
                        }),
                );
                if sequence.len() < 2 {
                    continue;
                }
                if let Some((quality, bounds)) = key_sequence(&image, staff, 0.0, &sequence) {
                    candidates.push((bounds.len(), quality, fifths, bounds));
                }
            }
        }
        candidates.sort_by(|first, second| {
            second
                .0
                .cmp(&first.0)
                .then_with(|| second.1.total_cmp(&first.1))
        });
        if let Some((_, _, fifths, bounds)) = candidates.into_iter().next() {
            previous = KeySignature { fifths };
            changes.push(KeyChange {
                staff: staff_index,
                x: bounds[0].left - staff.spacing() * 0.1,
                key_signature: previous,
                bounds,
                confirmed: false,
            });
        }
    }
    changes
}

pub(crate) fn draw_demo_header(image: &mut GrayImage, top: u32, clef: Clef) {
    let (character, height, clef_top) = match clef {
        Clef::Bass => ('\u{e062}', 3.6 * 16.0, top - 2),
        _ => ('\u{e050}', 6.4 * 16.0, top - 24),
    };
    let mut symbols = vec![(resized(&glyph(character), height), 75, clef_top)];
    if top == 100 {
        symbols.push((
            resized(&meter_template(TimeSignature::Common), 32.0),
            160,
            top + 16,
        ));
    }
    for (symbol, left, top) in symbols {
        for (x, y, pixel) in symbol.enumerate_pixels() {
            if pixel[0] > 127 {
                image.put_pixel(left + x, top + y, Luma([0]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ending_labels_keep_multiple_and_multidigit_passes_and_uncertain_text() {
        use crate::model::RepeatMarkKind;
        for label in ["12", "1,2", "X"] {
            let mut image = GrayImage::from_pixel(600, 300, Luma([255]));
            for x in 100..=500 {
                image.put_pixel(x, 80, Luma([0]));
            }
            for y in 80..=100 {
                image.put_pixel(100, y, Luma([0]));
            }
            let mut x = 110;
            for character in label.chars() {
                if character == ',' {
                    imageproc::drawing::draw_filled_circle_mut(
                        &mut image,
                        (x + 3, 105),
                        1,
                        Luma([0]),
                    );
                    x += 12;
                } else if character == 'X' {
                    imageproc::drawing::draw_line_segment_mut(
                        &mut image,
                        (x as f32, 88.0),
                        (x as f32 + 10.0, 108.0),
                        Luma([0]),
                    );
                    imageproc::drawing::draw_line_segment_mut(
                        &mut image,
                        (x as f32 + 10.0, 88.0),
                        (x as f32, 108.0),
                        Luma([0]),
                    );
                } else {
                    let mut digit = resized(
                        &trimmed_glyph(
                            char::from_u32(0xe080 + character.to_digit(10).unwrap()).unwrap(),
                        ),
                        20.0,
                    );
                    imageops::invert(&mut digit);
                    imageops::overlay(&mut image, &digit, i64::from(x), 88);
                    x += digit.width() as i32 + 2;
                }
            }
            let score = Score {
                staves: vec![Staff {
                    lines: [180.0, 200.0, 220.0, 240.0, 260.0],
                    left: 20.0,
                    right: 580.0,
                    header: Default::default(),
                }],
                ..Default::default()
            };
            let binary = imageproc::contrast::threshold(
                &image,
                170,
                imageproc::contrast::ThresholdType::Binary,
            );
            let marks = recognize_repeat_marks(&binary, &score);
            assert_eq!(marks.len(), 1, "{label}: {marks:?}");
            assert_eq!(marks[0].kind, RepeatMarkKind::Ending);
            assert_eq!(marks[0].numbers, if label == "X" { "" } else { label });
        }
    }

    #[test]
    fn repeat_signs_and_endings_survive_scaling_without_false_notes() {
        use crate::model::RepeatMarkKind;
        for scale in [0.75, 1.0, 1.4] {
            let mut image = GrayImage::from_pixel(900, 360, Luma([255]));
            for line in [180, 200, 220, 240, 260] {
                for x in 20..880 {
                    image.put_pixel(x, line, Luma([0]));
                }
            }
            for (x, kind) in [(100, RepeatMarkKind::Segno), (240, RepeatMarkKind::Coda)] {
                let mut sign = resized(&trimmed_glyph(kind.glyph().unwrap()), 48.0);
                imageops::invert(&mut sign);
                imageops::overlay(&mut image, &sign, x, 90);
            }
            for (left, right, closed, digit) in [(360, 580, true, 1), (620, 850, false, 2)] {
                for x in left..=right {
                    image.put_pixel(x, 100, Luma([0]));
                    image.put_pixel(x, 101, Luma([0]));
                }
                for y in 100..=124 {
                    image.put_pixel(left, y, Luma([0]));
                    if closed {
                        image.put_pixel(right, y, Luma([0]));
                    }
                }
                let mut label = resized(
                    &trimmed_glyph(char::from_u32(0xe080 + digit).unwrap()),
                    20.0,
                );
                imageops::invert(&mut label);
                imageops::overlay(&mut image, &label, i64::from(left + 10), 108);
            }
            // Plain roof and unlabelled beam with hooks are negative cases.
            for x in 50..300 {
                image.put_pixel(x, 50, Luma([0]));
            }
            for x in 400..800 {
                image.put_pixel(x, 40, Luma([0]));
            }
            for y in 40..65 {
                image.put_pixel(400, y, Luma([0]));
                image.put_pixel(799, y, Luma([0]));
            }
            let image = imageops::resize(
                &image,
                (900.0 * scale) as u32,
                (360.0 * scale) as u32,
                FilterType::Triangle,
            );
            let score = crate::recognition::recognize(&image);
            assert_eq!(
                score
                    .repeat_marks
                    .iter()
                    .map(|mark| mark.kind)
                    .collect::<Vec<_>>(),
                vec![
                    RepeatMarkKind::Segno,
                    RepeatMarkKind::Coda,
                    RepeatMarkKind::Ending,
                    RepeatMarkKind::Ending
                ],
                "scale {scale}: {:?}",
                score.repeat_marks
            );
            assert_eq!(score.repeat_marks[2].numbers, "1");
            assert_eq!(score.repeat_marks[3].numbers, "2");
            assert!(score.repeat_marks[2].closed);
            assert!(!score.repeat_marks[3].closed);
            assert!(score.repeat_marks.iter().all(|mark| !mark.confirmed));
            assert!(score.notes.is_empty(), "scale {scale}: {:?}", score.notes);
            assert!(score.validate().is_ok());
        }
    }

    #[test]
    fn recognizes_dynamic_words_above_and_below_at_multiple_scales() {
        for scale in [0.75, 1.0, 1.4] {
            for below in [false, true] {
                for kind in DynamicKind::ALL {
                    let mut image = GrayImage::from_pixel(700, 400, Luma([255]));
                    for row in [140, 160, 180, 200, 220] {
                        for column in 20..680 {
                            image.put_pixel(column, row, Luma([0]));
                        }
                    }
                    let symbol = resized(&glyph(kind.glyph()), 30.0);
                    let y = if below { 280 } else { 80 };
                    for (column, row, pixel) in symbol.enumerate_pixels() {
                        if pixel[0] > 127 {
                            image.put_pixel(
                                420 - symbol.width() / 2 + column,
                                y - symbol.height() / 2 + row,
                                Luma([0]),
                            );
                        }
                    }
                    let image = imageops::resize(
                        &image,
                        (700.0 * scale) as u32,
                        (400.0 * scale) as u32,
                        FilterType::Triangle,
                    );
                    let binary = imageproc::contrast::threshold(
                        &image,
                        170,
                        imageproc::contrast::ThresholdType::Binary,
                    );
                    let score = Score {
                        staves: vec![Staff {
                            lines: [140.0, 160.0, 180.0, 200.0, 220.0].map(|v| v * scale),
                            left: 20.0 * scale,
                            right: 680.0 * scale,
                            header: StaffHeader::default(),
                        }],
                        ..Score::default()
                    };
                    let dynamics = recognize_dynamics(&binary, &score);
                    assert_eq!(
                        dynamics.len(),
                        1,
                        "{kind:?} below {below} scale {scale}: {dynamics:?}"
                    );
                    assert_eq!(dynamics[0].kind, kind, "below {below} scale {scale}");
                    assert_eq!(dynamics[0].staff, 0);
                    assert!((dynamics[0].x - 420.0 * scale).abs() < 3.0);
                    assert!(!dynamics[0].confirmed);
                    // Full pipeline must not create notes from loops in dynamic letters.
                    if scale == 1.0
                        && below
                        && matches!(
                            kind,
                            DynamicKind::P | DynamicKind::Mf | DynamicKind::Ff | DynamicKind::Sfz
                        )
                    {
                        let recognized = crate::recognition::recognize(&image);
                        assert_eq!(recognized.dynamics.len(), 1, "{kind:?} scale {scale}");
                        assert_eq!(recognized.dynamics[0].kind, kind);
                        assert!(
                            recognized.notes.is_empty(),
                            "{kind:?} scale {scale}: {:?}",
                            recognized.notes
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn dynamics_do_not_merge_an_adjacent_accent_from_the_following_staff() {
        let mut score = Score {
            staves: vec![Staff {
                lines: [100.0, 120.0, 140.0, 160.0, 180.0],
                left: 20.0,
                right: 680.0,
                header: StaffHeader::default(),
            }],
            ..Score::default()
        };
        let mut second = score.staves[0].clone();
        second.lines = second.lines.map(|y| y + 200.0);
        score.staves.push(second);
        let mut image = GrayImage::from_pixel(700, 500, Luma([255]));
        let dynamic = resized(&glyph(DynamicKind::Ff.glyph()), 40.0);
        let accent = resized(&glyph('\u{e4a0}'), 12.0);
        let accent_x = 400 + dynamic.width() + 3;
        for (symbol, x, y) in [(dynamic, 400, 235), (accent, accent_x, 265)] {
            for (column, row, pixel) in symbol.enumerate_pixels() {
                if pixel[0] > 127 {
                    image.put_pixel(x + column, y + row, Luma([0]));
                }
            }
        }
        let found = recognize_dynamics(&image, &score);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, DynamicKind::Ff);
        assert_eq!(found[0].staff, 0);
        assert!(found[0].bounds.as_ref().unwrap().right < accent_x as f32);
    }

    #[test]
    fn dynamics_reject_digits_articulations_and_assign_the_nearest_staff() {
        let mut score = Score {
            staves: vec![Staff {
                lines: [100.0, 120.0, 140.0, 160.0, 180.0],
                left: 20.0,
                right: 680.0,
                header: StaffHeader::default(),
            }],
            ..Score::default()
        };
        let mut second = score.staves[0].clone();
        second.lines = second.lines.map(|y| y + 230.0);
        score.staves.push(second);
        let mut image = GrayImage::from_pixel(700, 500, Luma([255]));
        for (character, height, x, y) in [
            ('\u{e083}', 30.0, 100, 220),
            ('\u{e4ac}', 20.0, 200, 220),
            ('\u{e4c0}', 20.0, 300, 220),
            (DynamicKind::Mf.glyph(), 30.0, 420, 225),
        ] {
            let symbol = resized(&glyph(character), height);
            for (column, row, pixel) in symbol.enumerate_pixels() {
                if pixel[0] > 127 {
                    image.put_pixel(x + column, y + row, Luma([0]));
                }
            }
        }
        let found = recognize_dynamics(&image, &score);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, DynamicKind::Mf);
        assert_eq!(found[0].staff, 0);
        let mut known = score.clone();
        known.staves[0].header.time_bounds = found[0].bounds.clone();
        assert!(recognize_dynamics(&image, &known).is_empty());
    }

    #[test]
    fn recognizes_articulations_above_and_below_notes_at_multiple_scales() {
        for scale in [0.75, 1.0, 1.4] {
            for below in [false, true] {
                for close in [false, true] {
                    for (kind, character, height) in [
                        (Articulation::Staccato, '\u{e4a2}', 6.0),
                        (Articulation::Tenuto, '\u{e4a4}', 3.0),
                        (
                            Articulation::Marcato,
                            Articulation::Marcato.glyph(below).unwrap(),
                            16.0,
                        ),
                        (
                            Articulation::Staccatissimo,
                            Articulation::Staccatissimo.glyph(below).unwrap(),
                            16.0,
                        ),
                        (
                            Articulation::Portato,
                            Articulation::Portato.glyph(below).unwrap(),
                            12.0,
                        ),
                        (
                            Articulation::AccentStaccato,
                            Articulation::AccentStaccato.glyph(below).unwrap(),
                            18.0,
                        ),
                        (
                            Articulation::TenutoAccent,
                            Articulation::TenutoAccent.glyph(below).unwrap(),
                            18.0,
                        ),
                        (
                            Articulation::MarcatoStaccato,
                            Articulation::MarcatoStaccato.glyph(below).unwrap(),
                            24.0,
                        ),
                        (
                            Articulation::Fermata,
                            Articulation::Fermata.glyph(below).unwrap(),
                            20.0,
                        ),
                        (
                            Articulation::Accent,
                            if below { '\u{e4a1}' } else { '\u{e4a0}' },
                            12.0,
                        ),
                    ] {
                        let mut image = GrayImage::from_pixel(700, 360, Luma([255]));
                        for line in [140, 160, 180, 200, 220] {
                            for column in 20..680 {
                                image.put_pixel(column, line, Luma([0]));
                            }
                        }
                        let head_y = if below { 170 } else { 190 };
                        let down = below != close;
                        let stem_x = if down { 408 } else { 432 };
                        let tip = if down { head_y + 65 } else { head_y - 65 };
                        imageproc::drawing::draw_filled_ellipse_mut(
                            &mut image,
                            (420, head_y),
                            13,
                            8,
                            Luma([0]),
                        );
                        for row in tip.min(head_y)..=tip.max(head_y) {
                            for column in stem_x..=stem_x + 1 {
                                image.put_pixel(column as u32, row as u32, Luma([0]));
                            }
                        }
                        let baseline = crate::recognition::recognize(&imageops::resize(
                            &image,
                            (700.0 * scale) as u32,
                            (360.0 * scale) as u32,
                            FilterType::Triangle,
                        ));
                        assert_eq!(baseline.notes.len(), 1);
                        let symbol = resized(&glyph(character), height);
                        let clearance = 22.max((height * 0.5 + 14.0) as i32);
                        let center_y = if close {
                            if below {
                                head_y + clearance
                            } else {
                                head_y - clearance
                            }
                        } else if below {
                            270
                        } else {
                            90
                        };
                        for (column, row, pixel) in symbol.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    420 - symbol.width() / 2 + column,
                                    center_y as u32 - symbol.height() / 2 + row,
                                    Luma([0]),
                                );
                            }
                        }
                        let image = imageops::resize(
                            &image,
                            (700.0 * scale) as u32,
                            (360.0 * scale) as u32,
                            FilterType::Triangle,
                        );
                        let score = crate::recognition::recognize(&image);
                        assert_eq!(
                            score.notes.len(),
                            1,
                            "{kind:?}, below {below}, close {close}, scale {scale}: {:?}",
                            score.notes
                        );
                        assert_eq!(
                            score.notes[0].articulation, kind,
                            "below {below}, close {close}, scale {scale}"
                        );
                        assert_eq!(
                            score.notes[0].duration, baseline.notes[0].duration,
                            "{kind:?}, below {below}, close {close}, scale {scale}: {:?}",
                            score.notes
                        );
                        assert_eq!(score.notes[0].pitch, baseline.notes[0].pitch);
                        assert_eq!(score.notes[0].dots, 0);
                        assert!(!score.notes[0].confirmed);
                        let restored: Score =
                            serde_json::from_str(&serde_json::to_string(&score).unwrap()).unwrap();
                        assert_eq!(restored.notes[0].articulation, kind);
                    }
                }
            }
        }
    }

    #[test]
    fn articulation_candidates_reject_foreign_shapes_and_wrong_positions() {
        for scale in [0.75, 1.0, 1.4] {
            for case in 0..9 {
                let mut image = GrayImage::from_pixel(700, 360, Luma([255]));
                let staff = Staff {
                    lines: [140.0, 160.0, 180.0, 200.0, 220.0].map(|row| row * scale),
                    left: 20.0 * scale,
                    right: 680.0 * scale,
                    header: StaffHeader::default(),
                };
                match case {
                    0..=2 => {
                        let (character, height) = match case {
                            0 => ('\u{e883}', 16.0),
                            1 => ('\u{e522}', 32.0),
                            _ => ('\u{e262}', 52.0),
                        };
                        let symbol = resized(&glyph(character), height);
                        for (column, row, pixel) in symbol.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    420 - symbol.width() / 2 + column,
                                    90 - symbol.height() / 2 + row,
                                    Luma([0]),
                                );
                            }
                        }
                    }
                    3..=6 => {
                        let (left, top, width, height) = match case {
                            3 => (410, 85, 20, 10),
                            4 => (380, 90, 80, 2),
                            5 => (410, 80, 2, 50),
                            _ => (410, 190, 20, 3),
                        };
                        imageproc::drawing::draw_filled_rect_mut(
                            &mut image,
                            imageproc::rect::Rect::at(left, top).of_size(width, height),
                            Luma([0]),
                        );
                    }
                    7 => {
                        let symbol = resized(&glyph('\u{e4a0}'), 12.0);
                        for (column, row, pixel) in symbol.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    450 - symbol.width() / 2 + column,
                                    90 + row,
                                    Luma([0]),
                                );
                            }
                        }
                    }
                    _ => imageproc::drawing::draw_filled_circle_mut(
                        &mut image,
                        (446, 190),
                        3,
                        Luma([0]),
                    ),
                }
                let image = imageops::resize(
                    &image,
                    (700.0 * scale) as u32,
                    (360.0 * scale) as u32,
                    FilterType::Triangle,
                );
                let mut score = Score {
                    staves: vec![staff],
                    ..Score::default()
                };
                score.add_note(420.0 * scale, 190.0 * scale);
                recognize_articulations(&image, &mut score);
                assert_eq!(
                    score.notes[0].articulation,
                    Articulation::None,
                    "case {case}, scale {scale}"
                );
            }
        }
    }

    #[test]
    fn articulation_is_assigned_to_one_nearest_note_and_excludes_known_symbols() {
        let mut image = GrayImage::from_pixel(700, 360, Luma([255]));
        imageproc::drawing::draw_filled_rect_mut(
            &mut image,
            imageproc::rect::Rect::at(410, 90).of_size(20, 3),
            Luma([0]),
        );
        let mut score = Score {
            staves: vec![Staff {
                lines: [140.0, 160.0, 180.0, 200.0, 220.0],
                left: 20.0,
                right: 680.0,
                header: StaffHeader::default(),
            }],
            ..Score::default()
        };
        for (x, y) in [(420.0, 190.0), (420.0, 210.0), (455.0, 190.0)] {
            score.add_note(x, y);
        }
        let original = score.clone();
        recognize_articulations(&image, &mut score);
        assert_eq!(
            score
                .notes
                .iter()
                .map(|note| note.articulation)
                .collect::<Vec<_>>(),
            vec![Articulation::Tenuto, Articulation::None, Articulation::None]
        );
        score = original;
        score.add_rest(420.0, 90.0, Duration::Half);
        score.rests[0].bounds = Some(SymbolBounds {
            left: 410.0,
            top: 90.0,
            right: 430.0,
            bottom: 93.0,
        });
        recognize_articulations(&image, &mut score);
        assert!(
            score
                .notes
                .iter()
                .all(|note| note.articulation == Articulation::None)
        );
    }

    #[test]
    fn requires_a_triplet_digit_and_shared_beam() {
        for down in [false, true] {
            for (digit, beam, expected) in [(true, true, 3), (false, true, 0), (true, false, 0)] {
                for scale in [0.75, 1.0, 1.4] {
                    let mut image = GrayImage::from_pixel(600, 340, Luma([255]));
                    for line in [120, 140, 160, 180, 200] {
                        for x in 20..580 {
                            image.put_pixel(x, line, Luma([0]));
                        }
                    }
                    let head_y = if down { 140 } else { 180 };
                    let tip = if down { 220 } else { 100 };
                    let stem_offset = if down { -12 } else { 12 };
                    for center in [180, 260, 340] {
                        imageproc::drawing::draw_filled_ellipse_mut(
                            &mut image,
                            (center, head_y),
                            13,
                            8,
                            Luma([0]),
                        );
                        for y in tip.min(head_y)..=tip.max(head_y) {
                            for x in center + stem_offset..=center + stem_offset + 1 {
                                image.put_pixel(x as u32, y as u32, Luma([0]));
                            }
                        }
                    }
                    if beam {
                        for y in tip..tip + 7 {
                            for x in 180 + stem_offset..=340 + stem_offset + 1 {
                                image.put_pixel(x as u32, y as u32, Luma([0]));
                            }
                        }
                    }
                    if digit {
                        let symbol = resized(&glyph('\u{e883}'), 16.0);
                        let top = if down { tip + 12 } else { tip - 28 };
                        for (x, y, pixel) in symbol.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    260 - symbol.width() / 2 + x,
                                    top as u32 + y,
                                    Luma([0]),
                                );
                            }
                        }
                    }
                    let image = imageops::resize(
                        &image,
                        (image.width() as f32 * scale) as u32,
                        (image.height() as f32 * scale) as u32,
                        FilterType::Triangle,
                    );
                    let score = crate::recognition::recognize(&image);
                    assert_eq!(
                        score.notes.len(),
                        3,
                        "down {down}, scale {scale}, digit {digit}, beam {beam}: {:?}",
                        score.notes
                    );
                    assert_eq!(
                        score.notes.iter().filter(|note| note.triplet).count(),
                        expected,
                        "down {down}, scale {scale}, digit {digit}, beam {beam}: {:?}",
                        score.notes
                    );
                }
            }
        }
    }

    #[test]
    fn recognizes_note_accidentals_without_inventing_them() {
        for down in [false, true] {
            for scale in [0.75, 1.0, 1.4] {
                for head_y in [100, 110] {
                    for (character, expected, anchor) in [
                        (None, None, 0.5),
                        (Some('\u{e260}'), Some(-1), 0.78),
                        (Some('\u{e261}'), Some(0), 0.5),
                        (Some('\u{e262}'), Some(1), 0.5),
                    ] {
                        let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
                        imageproc::drawing::draw_filled_ellipse_mut(
                            &mut image,
                            (420, head_y),
                            13,
                            8,
                            Luma([0]),
                        );
                        let stem_x = if down { 408 } else { 432 };
                        let tip = if down { head_y + 65 } else { head_y - 65 };
                        for row in tip.min(head_y)..=tip.max(head_y) {
                            for column in stem_x..=stem_x + 1 {
                                image.put_pixel(column as u32, row as u32, Luma([0]));
                            }
                        }
                        if let Some(character) = character {
                            let symbol = resized(&glyph(character), 52.0);
                            let top = (head_y as f32 - 52.0 * anchor).round() as u32;
                            for (column, row, pixel) in symbol.enumerate_pixels() {
                                if pixel[0] > 127 {
                                    image.put_pixel(
                                        380 - symbol.width() / 2 + column,
                                        top + row,
                                        Luma([0]),
                                    );
                                }
                            }
                        }
                        let image = imageops::resize(
                            &image,
                            (700.0 * scale) as u32,
                            (220.0 * scale) as u32,
                            FilterType::Triangle,
                        );
                        let score = crate::recognition::recognize(&image);
                        assert_eq!(
                            score.notes.len(),
                            1,
                            "{character:?}, down {down}, scale {scale}, y {head_y}: {:?}",
                            score.notes
                        );
                        let note = &score.notes[0];
                        assert_eq!(
                            note.accidental_explicit,
                            expected.is_some(),
                            "{character:?}, down {down}, scale {scale}, y {head_y}: {note:?}"
                        );
                        assert_eq!(
                            note.accidental,
                            expected.unwrap_or(0),
                            "{character:?}, scale {scale}: {note:?}"
                        );
                        assert_eq!(note.duration, Duration::Quarter, "{note:?}");
                        assert!(score.validate().is_ok());
                    }
                }
            }
        }
    }

    #[test]
    fn recognizes_open_heads_and_flags_in_both_directions() {
        for (down, right_flag) in [(false, true), (true, false), (true, true)] {
            for scale in [0.75, 1.0, 1.4] {
                let mut image = GrayImage::from_pixel(1400, 360, Luma([255]));
                for line in [90, 114, 138, 162, 186] {
                    for x in 20..1380 {
                        image.put_pixel(x, line, Luma([0]));
                    }
                }
                for (index, duration) in Duration::ALL.into_iter().enumerate() {
                    let center = 150 + index as i32 * 160;
                    let character = match duration {
                        Duration::Whole => '\u{e0a2}',
                        Duration::Half => '\u{e0a3}',
                        _ => '\u{e0a4}',
                    };
                    let head = resized(&glyph(character), 19.0);
                    let left = center - head.width() as i32 / 2;
                    let top = 174 - head.height() as i32 / 2;
                    for (x, y, pixel) in head.enumerate_pixels() {
                        if pixel[0] > 127 {
                            image.put_pixel(
                                (left + x as i32) as u32,
                                (top + y as i32) as u32,
                                Luma([0]),
                            );
                        }
                    }
                    if duration == Duration::Whole {
                        continue;
                    }
                    let stem_x = if down {
                        left + 1
                    } else {
                        left + head.width() as i32 - 2
                    };
                    let length = if duration.flag_count() >= 3 { 112 } else { 84 };
                    let tip = if down { 174 + length } else { 174 - length };
                    for y in tip.min(174)..=tip.max(174) {
                        for x in stem_x..=stem_x + 1 {
                            image.put_pixel(x as u32, y as u32, Luma([0]));
                        }
                    }
                    if duration.flag_count() > 0 {
                        let character = char::from_u32(
                            0xe240 + (duration.flag_count() as u32 - 1) * 2 + u32::from(down),
                        )
                        .unwrap();
                        let flag = resized(
                            &glyph(character),
                            match duration.flag_count() {
                                1 => 67.0,
                                2 => 77.0,
                                3 => 86.0,
                                _ => 96.0,
                            },
                        );
                        let left = if down && !right_flag {
                            stem_x - flag.width() as i32 + 2
                        } else {
                            stem_x
                        };
                        let top = if down {
                            tip - flag.height() as i32
                        } else {
                            tip
                        };
                        for (x, y, pixel) in flag.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    (left + x as i32) as u32,
                                    (top + y as i32) as u32,
                                    Luma([0]),
                                );
                            }
                        }
                    }
                }
                let image = imageops::resize(
                    &image,
                    (image.width() as f32 * scale) as u32,
                    (image.height() as f32 * scale) as u32,
                    FilterType::Triangle,
                );
                let score = crate::recognition::recognize(&image);
                assert_eq!(
                    score
                        .notes
                        .iter()
                        .map(|note| note.duration)
                        .collect::<Vec<_>>(),
                    Duration::ALL,
                    "down {down}, scale {scale}: {:?}",
                    score.notes
                );
                assert!(
                    score.notes.iter().all(|note| !note.accidental_explicit),
                    "{:?}",
                    score.notes
                );
            }
        }
    }

    #[test]
    fn recognizes_multimeasure_rests_with_counts_but_not_incomplete_symbols() {
        for scale in [0.75, 1.0, 1.4] {
            for count in [2, 3, 5, 7, 8, 9, 12, 16] {
                for (complete, gap_width) in [(true, 0), (false, 0), (true, 1), (true, 8)] {
                    let mut image = GrayImage::from_pixel(600, 260, Luma([255]));
                    for row in [100, 120, 140, 160, 180] {
                        for column in 20..580 {
                            image.put_pixel(column, row, Luma([0]));
                        }
                    }
                    for row in 137..=143 {
                        for column in 220..=420 {
                            image.put_pixel(column, row, Luma([0]));
                        }
                    }
                    for column in [220, 420] {
                        if complete || column == 220 {
                            for offset in 0..2 {
                                for row in 120..=160 {
                                    image.put_pixel(column + offset, row, Luma([0]));
                                }
                            }
                        }
                    }
                    for column in 350..350 + gap_width {
                        for row in 135..=145 {
                            image.put_pixel(column, row, Luma([255]));
                        }
                    }
                    let digits = resized(&number(count), 42.0);
                    for (column, row, pixel) in digits.enumerate_pixels() {
                        if pixel[0] > 127 {
                            image.put_pixel(320 - digits.width() / 2 + column, 45 + row, Luma([0]));
                        }
                    }
                    let image = imageops::resize(
                        &image,
                        (600.0 * scale) as u32,
                        (260.0 * scale) as u32,
                        FilterType::Triangle,
                    );
                    let score = crate::recognition::recognize(&image);
                    let rests: Vec<_> = score
                        .rests
                        .iter()
                        .filter(|rest| rest.measures.is_some())
                        .collect();
                    assert_eq!(
                        rests.len(),
                        usize::from(complete && gap_width < 8),
                        "count {count}, scale {scale}, complete {complete}, gap {gap_width}: {:?}",
                        score.rests
                    );
                    if complete && gap_width < 8 {
                        assert_eq!(
                            rests[0].measures,
                            Some(count),
                            "scale {scale}: {:?}",
                            rests[0]
                        );
                        assert!(
                            score
                                .notes
                                .iter()
                                .all(|note| note.x < 210.0 * scale || note.x > 430.0 * scale),
                            "{:?}",
                            score.notes
                        );
                    }
                    assert!(score.validate().is_ok());
                }
            }
        }
        let mut image = GrayImage::from_pixel(400, 240, Luma([255]));
        let staff = Staff {
            lines: [80.0, 100.0, 120.0, 140.0, 160.0],
            left: 20.0,
            right: 380.0,
            header: StaffHeader::default(),
        };
        for row in 117..=123 {
            for column in 100..=300 {
                image.put_pixel(column, row, Luma([0]));
            }
        }
        for column in [100, 300] {
            for row in 100..=140 {
                image.put_pixel(column, row, Luma([0]));
            }
        }
        assert!(recognize_multimeasure_rests(&image, &staff, 0).is_empty());
    }

    #[test]
    fn recognizes_displaced_eighth_rests_without_inventing_notes() {
        for scale in [0.75, 1.0, 1.4] {
            let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
            let symbol = resized(&glyph('\u{e4e6}'), 38.0);
            for (left, top) in [(240, 90), (400, 130)] {
                for (column, row, pixel) in symbol.enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(left + column, top + row, Luma([0]));
                    }
                }
            }
            let image = imageops::resize(
                &image,
                (image.width() as f32 * scale) as u32,
                (image.height() as f32 * scale) as u32,
                FilterType::Triangle,
            );
            let score = crate::recognition::recognize(&image);
            assert_eq!(score.rests.len(), 2, "scale {scale}: {:?}", score.rests);
            assert!(
                score
                    .rests
                    .iter()
                    .all(|rest| rest.duration == Duration::Eighth)
            );
            assert!(score.notes.is_empty(), "{:?}", score.notes);
        }
    }

    #[test]
    fn short_rests_keep_all_hooks_at_different_sizes_and_vertical_positions() {
        for scale in [0.75, 1.0, 1.4] {
            for size in [0.85, 1.0, 1.1] {
                for displacement in [0, 40] {
                    let (mut image, mut staff) =
                        printed_header('\u{e062}', 3.6, TimeSignature::Common);
                    for (index, (character, height, top)) in [
                        ('\u{e4e7}', 58.0, 80),
                        ('\u{e4e8}', 76.0, 70),
                        ('\u{e4e9}', 96.0, 60),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let symbol = resized(&glyph(character), height * size);
                        for (x, y, pixel) in symbol.enumerate_pixels() {
                            if pixel[0] > 127 {
                                image.put_pixel(
                                    240 + index as u32 * 140 + x,
                                    top + displacement + y,
                                    Luma([0]),
                                );
                            }
                        }
                    }
                    let image = imageops::resize(
                        &image,
                        (700.0 * scale) as u32,
                        (220.0 * scale) as u32,
                        FilterType::Triangle,
                    );
                    let binary = imageproc::contrast::threshold(
                        &image,
                        127,
                        imageproc::contrast::ThresholdType::Binary,
                    );
                    staff.lines = staff.lines.map(|line| line * scale);
                    staff.left *= scale;
                    staff.right *= scale;
                    staff.header = recognize_header(&binary, &staff);
                    let rests = recognize_rests(&binary, &staff, 0);
                    assert_eq!(
                        rests.iter().map(|rest| rest.duration).collect::<Vec<_>>(),
                        [
                            Duration::Sixteenth,
                            Duration::ThirtySecond,
                            Duration::SixtyFourth
                        ],
                        "scale {scale}, size {size}, displacement {displacement}: {rests:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn recognizes_rest_values_without_inventing_notes() {
        for scale in [0.75, 1.0, 1.4] {
            let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
            for (index, (character, height, top)) in [
                ('\u{e4e3}', 10.0, 90),
                ('\u{e4e4}', 10.0, 100),
                ('\u{e4e5}', 58.0, 80),
                ('\u{e4e6}', 38.0, 90),
                ('\u{e4e7}', 58.0, 80),
                ('\u{e4e8}', 76.0, 70),
                ('\u{e4e9}', 96.0, 60),
            ]
            .into_iter()
            .enumerate()
            {
                let symbol = resized(&glyph(character), height);
                for (x, y, pixel) in symbol.enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(240 + index as u32 * 60 + x, top + y, Luma([0]));
                    }
                }
            }
            let image = imageops::resize(
                &image,
                (image.width() as f32 * scale) as u32,
                (image.height() as f32 * scale) as u32,
                FilterType::Triangle,
            );
            let score = crate::recognition::recognize(&image);
            assert_eq!(
                score
                    .rests
                    .iter()
                    .map(|rest| rest.duration)
                    .collect::<Vec<_>>(),
                Duration::ALL,
                "scale {scale}: {:?}",
                score.rests
            );
            assert!(
                score.notes.is_empty(),
                "rest glyphs must not become notes: {:?}",
                score.notes
            );
            assert!(score.validate().is_ok());
        }
        assert!(
            crate::recognition::recognize(&crate::recognition::demo_image())
                .rests
                .is_empty()
        );
    }

    #[test]
    fn recognizes_headers_when_local_staff_lines_differ_from_page_peaks() {
        for scale in [0.75, 1.0, 1.4] {
            for (character, height, clef) in [
                ('\u{e062}', 3.6, Clef::Bass),
                ('\u{e050}', 6.4, Clef::Treble),
            ] {
                let (image, staff) = printed_header(character, height, TimeSignature::Common);
                let image = imageops::resize(
                    &image,
                    (700.0 * scale) as u32,
                    (220.0 * scale) as u32,
                    FilterType::Triangle,
                );
                let binary = imageproc::contrast::threshold(
                    &image,
                    170,
                    imageproc::contrast::ThresholdType::Binary,
                );
                for offset in [-5.0, 5.0] {
                    let mut local = staff.clone();
                    local.lines = local.lines.map(|line| (line + offset) * scale);
                    local.left *= scale;
                    local.right *= scale;
                    let header = recognize_header(&binary, &local);
                    assert_eq!(header.clef, clef, "scale {scale}, offset {offset}");
                    assert_eq!(header.time_signature, Some(TimeSignature::Common));
                }
            }
        }
    }

    #[test]
    fn detects_tight_meters_at_double_bars_and_curved_staff_ends() {
        for scale in [0.75, 1.0, 1.4] {
            let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
            for (left, beats, strokes) in [
                (316, 2, [306, 307, 308, 312, 313, 314]),
                (634, 4, [623, 624, 625, 630, 631, 632]),
            ] {
                for x in strokes {
                    for y in 70..=150 {
                        image.put_pixel(x, y, Luma([0]));
                    }
                }
                let symbol = resized(
                    &meter_template(TimeSignature::Meter {
                        beats,
                        beat_unit: 4,
                    }),
                    78.0,
                );
                for (x, y, pixel) in symbol.enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(left + x, 71 + y, Luma([0]));
                    }
                }
            }
            let mut curved = GrayImage::from_pixel(700, 230, Luma([255]));
            for (x, y, pixel) in image.enumerate_pixels() {
                curved.put_pixel(x, y + if x >= 600 { 3 } else { 0 }, *pixel);
            }
            let image = imageops::resize(
                &curved,
                (700.0 * scale) as u32,
                (230.0 * scale) as u32,
                FilterType::Triangle,
            );
            let score = crate::recognition::recognize(&image);
            assert!(
                score.staves[0].right / scale >= 678.0,
                "scale {scale}: {:?}",
                score.staves
            );
            assert_eq!(
                score
                    .time_changes
                    .iter()
                    .map(|change| change.time_signature)
                    .collect::<Vec<_>>(),
                [
                    TimeSignature::Meter {
                        beats: 2,
                        beat_unit: 4
                    },
                    TimeSignature::Meter {
                        beats: 4,
                        beat_unit: 4
                    }
                ],
                "scale {scale}: {:?}",
                score.time_changes
            );
            assert!(score.notes.is_empty(), "scale {scale}: {:?}", score.notes);
        }
    }

    #[test]
    fn detects_inline_time_changes_without_turning_digits_into_notes() {
        for scale in [0.75, 1.0, 1.4] {
            for meter in [
                TimeSignature::Common,
                TimeSignature::Cut,
                TimeSignature::Meter {
                    beats: 3,
                    beat_unit: 4,
                },
                TimeSignature::Meter {
                    beats: 6,
                    beat_unit: 8,
                },
                TimeSignature::Meter {
                    beats: 5,
                    beat_unit: 4,
                },
                TimeSignature::Meter {
                    beats: 7,
                    beat_unit: 8,
                },
                TimeSignature::Meter {
                    beats: 12,
                    beat_unit: 8,
                },
            ] {
                let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
                for y in 70..=150 {
                    image.put_pixel(300, y, Luma([0]));
                }
                let (height, top) = match meter {
                    TimeSignature::Common => (40.0, 90),
                    TimeSignature::Cut => (64.0, 78),
                    _ => (78.0, 71),
                };
                let symbol = resized(&meter_template(meter), height);
                for (x, y, pixel) in symbol.enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(320 + x, top + y, Luma([0]));
                    }
                }
                imageproc::drawing::draw_filled_ellipse_mut(
                    &mut image,
                    (520, 130),
                    13,
                    8,
                    Luma([0]),
                );
                for y in 70..130 {
                    image.put_pixel(531, y, Luma([0]));
                }
                let image = imageops::resize(
                    &image,
                    (700.0 * scale) as u32,
                    (220.0 * scale) as u32,
                    FilterType::Triangle,
                );
                let score = crate::recognition::recognize(&image);
                assert_eq!(
                    score.time_changes.len(),
                    1,
                    "{meter:?}, scale {scale}: {:?}",
                    score.time_changes
                );
                assert_eq!(score.time_changes[0].time_signature, meter, "scale {scale}");
                assert_eq!(
                    score.staves[0].header.time_signature,
                    Some(TimeSignature::Common),
                    "meter {meter:?}, scale {scale}"
                );
                assert!(!score.time_changes[0].confirmed);
                assert_eq!(
                    score.notes.len(),
                    1,
                    "{meter:?}, scale {scale}: {:?}",
                    score.notes
                );
                assert!((score.notes[0].x / scale - 520.0).abs() < 5.0);
                assert!(score.validate().is_ok());
            }
        }
    }

    #[test]
    fn detects_a_meter_at_a_later_staff_start_without_a_repeated_clef() {
        let (header, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
        let mut image = GrayImage::from_pixel(700, 400, Luma([255]));
        imageops::overlay(&mut image, &header, 0, 0);
        for y in [230, 250, 270, 290, 310] {
            for x in 20..680 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        let meter = TimeSignature::Meter {
            beats: 6,
            beat_unit: 8,
        };
        let symbol = resized(&meter_template(meter), 78.0);
        for (x, y, pixel) in symbol.enumerate_pixels() {
            if pixel[0] > 127 {
                image.put_pixel(40 + x, 231 + y, Luma([0]));
            }
        }
        let score = crate::recognition::recognize(&image);
        assert_eq!(score.staves.len(), 2);
        assert_eq!(score.time_changes.len(), 1, "{:?}", score.time_changes);
        assert_eq!(score.time_changes[0].staff, 1);
        assert_eq!(score.time_changes[0].time_signature, meter);
        assert_eq!(score.time_signature_before(1, 500.0), Some(meter));
        assert!(score.notes.is_empty(), "{:?}", score.notes);
    }

    #[test]
    fn inline_meters_require_complete_aligned_symbols() {
        for case in 0..4 {
            let (mut image, mut staff) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
            let symbol = resized(
                &meter_template(TimeSignature::Meter {
                    beats: 6,
                    beat_unit: 8,
                }),
                78.0,
            );
            for (x, y, pixel) in symbol.enumerate_pixels() {
                if pixel[0] > 127
                    && (case == 0 || case == 3 || (case == 1 && y < 37) || (case == 2 && y > 41))
                {
                    image.put_pixel(320 + x, (if case == 3 { 101 } else { 71 }) + y, Luma([0]));
                }
            }
            staff.header = recognize_header(&image, &staff);
            let changes = recognize_time_changes(
                &image,
                &staff,
                0,
                &[Barline {
                    staff: 0,
                    x: 300.0,
                    kind: crate::model::BarlineKind::Single,
                    confirmed: false,
                }],
            );
            assert_eq!(
                changes.len(),
                usize::from(case == 0),
                "case {case}: {changes:?}"
            );
        }
    }

    #[test]
    fn recognizes_ordered_sharp_and_flat_signatures() {
        for clef in [Clef::Bass, Clef::Treble] {
            for fifths in [-7_i8, -3, -1, 1, 3, 7] {
                let (mut image, mut staff) = printed_header(
                    if clef == Clef::Bass {
                        '\u{e062}'
                    } else {
                        '\u{e050}'
                    },
                    if clef == Clef::Bass { 3.6 } else { 6.4 },
                    TimeSignature::Common,
                );
                for y in 0..image.height() {
                    for x in 100..image.width() {
                        image.put_pixel(x, y, Luma([255]));
                    }
                }
                let (positions, character, anchor, height) = if fifths > 0 {
                    ([0.0, 1.5, -0.5, 1.0, 2.5, 0.5, 2.0], '\u{e262}', 0.5, 54.0)
                } else {
                    ([2.0, 0.5, 2.5, 1.0, 3.0, 1.5, 3.5], '\u{e260}', 0.78, 52.0)
                };
                for (index, mut position) in positions
                    .into_iter()
                    .take(fifths.unsigned_abs() as usize)
                    .enumerate()
                {
                    if clef == Clef::Bass {
                        position += 1.0;
                    }
                    let top = (staff.lines[0] + position * 20.0 - anchor * height).round() as u32;
                    let symbol = resized(&glyph(character), height);
                    for (x, y, pixel) in symbol.enumerate_pixels() {
                        if pixel[0] > 127 {
                            image.put_pixel(115 + index as u32 * 28 + x, top + y, Luma([0]));
                        }
                    }
                }
                let symbol = resized(&meter_template(TimeSignature::Common), 40.0);
                for (x, y, pixel) in symbol.enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(
                            160 + u32::from(fifths.unsigned_abs()) * 28 + x,
                            90 + y,
                            Luma([0]),
                        );
                    }
                }
                for &line in &staff.lines {
                    for x in 20..680 {
                        image.put_pixel(x, line as u32, Luma([0]));
                    }
                }
                staff.header = recognize_header(&image, &staff);
                assert_eq!(staff.header.clef, clef);
                assert_eq!(
                    staff.header.key_signature,
                    Some(KeySignature { fifths }),
                    "{clef:?}, {fifths}: {:?}",
                    staff.header
                );
                assert_eq!(
                    staff.header.key_bounds.len(),
                    fifths.unsigned_abs() as usize
                );
            }
        }
    }

    #[test]
    fn zero_fifths_requires_an_observed_empty_header_interval() {
        let (image, _) = printed_header('\u{e050}', 6.4, TimeSignature::Common);
        for scale in [0.75, 1.0, 1.5] {
            let clean = imageops::resize(
                &image,
                (700.0 * scale) as u32,
                (220.0 * scale) as u32,
                FilterType::Nearest,
            );
            let staff = Staff {
                lines: [70.0, 90.0, 110.0, 130.0, 150.0].map(|y| y * scale),
                left: 20.0 * scale,
                right: 680.0 * scale,
                header: Default::default(),
            };
            assert_eq!(
                recognize_header(&clean, &staff).key_signature,
                Some(KeySignature { fifths: 0 })
            );
            let mut dirty = clean.clone();
            imageproc::drawing::draw_filled_rect_mut(
                &mut dirty,
                imageproc::rect::Rect::at((120.0 * scale) as i32, (80.0 * scale) as i32)
                    .of_size((4.0 * scale) as u32, (30.0 * scale) as u32),
                Luma([0]),
            );
            assert!(
                recognize_header(&dirty, &staff).key_signature.is_none(),
                "unreadable ink must not mean zero fifths"
            );
            let mut no_meter = clean.clone();
            for x in (155.0 * scale) as u32..(200.0 * scale) as u32 {
                for y in 0..no_meter.height() {
                    if !staff
                        .lines
                        .iter()
                        .any(|line| (*line - y as f32).abs() < scale)
                    {
                        no_meter.put_pixel(x, y, Luma([255]));
                    }
                }
            }
            let header = recognize_header(&no_meter, &staff);
            assert!(header.time_signature.is_none());
            assert!(header.key_signature.is_none());
        }
    }

    #[test]
    fn damaged_key_continuations_need_a_readable_first_sign() {
        for occluded_first in [false, true] {
            let (mut image, staff) = printed_header('\u{e050}', 6.4, TimeSignature::Common);
            for y in 0..image.height() {
                for x in 100..image.width() {
                    image.put_pixel(x, y, Luma([255]));
                }
            }
            let flat = resized(&glyph('\u{e260}'), 52.0);
            for (index, position) in [2.0, 0.5, 2.5].into_iter().enumerate() {
                let top =
                    (staff.lines[0] + position * staff.spacing() - 0.78 * 52.0).round() as u32;
                for (x, y, pixel) in flat.enumerate_pixels() {
                    // Interrupted strokes model the differing damage left by
                    // staff-line removal at successive key-sign positions.
                    if pixel[0] > 127
                        && (!(occluded_first || index > 0) || y % 3 != 1)
                        && !(occluded_first && index == 0)
                    {
                        image.put_pixel(115 + index as u32 * 28 + x, top + y, Luma([0]));
                    }
                }
            }
            let meter = resized(&meter_template(TimeSignature::Common), 40.0);
            for (x, y, pixel) in meter.enumerate_pixels() {
                if pixel[0] > 127 {
                    image.put_pixel(250 + x, 90 + y, Luma([0]));
                }
            }
            for &line in &staff.lines {
                for x in 20..680 {
                    image.put_pixel(x, line as u32, Luma([0]));
                }
            }
            let header = recognize_header(&image, &staff);
            assert_eq!(
                header.key_signature,
                if occluded_first {
                    None
                } else {
                    Some(KeySignature { fifths: -3 })
                },
                "{header:?}"
            );
        }
    }

    #[test]
    fn inline_key_changes_require_compact_ordered_groups_at_barlines() {
        let cases = [
            (
                0,
                -4,
                vec![
                    ('\u{e260}', 2.0),
                    ('\u{e260}', 0.5),
                    ('\u{e260}', 2.5),
                    ('\u{e260}', 1.0),
                ],
            ),
            (
                -4,
                -1,
                vec![
                    ('\u{e261}', 0.5),
                    ('\u{e261}', 2.5),
                    ('\u{e261}', 1.0),
                    ('\u{e260}', 2.0),
                ],
            ),
            (
                -4,
                -2,
                vec![
                    ('\u{e261}', 2.0),
                    ('\u{e261}', 0.5),
                    ('\u{e261}', 2.5),
                    ('\u{e261}', 1.0),
                    ('\u{e260}', 2.0),
                    ('\u{e260}', 0.5),
                ],
            ),
            (-2, 0, vec![('\u{e261}', 2.0), ('\u{e261}', 0.5)]),
            (
                2,
                -2,
                vec![
                    ('\u{e261}', 0.0),
                    ('\u{e261}', 1.5),
                    ('\u{e260}', 2.0),
                    ('\u{e260}', 0.5),
                ],
            ),
            (
                0,
                3,
                vec![('\u{e262}', 0.0), ('\u{e262}', 1.5), ('\u{e262}', -0.5)],
            ),
        ];
        let fixture = |clef, previous, sequence: &[(char, f32)], spacing: u32, scale: f32| {
            let mut image = GrayImage::from_pixel(700, 220, Luma([255]));
            let mut staff = Staff {
                lines: [70.0, 90.0, 110.0, 130.0, 150.0],
                left: 20.0,
                right: 680.0,
                header: StaffHeader {
                    clef,
                    key_signature: (previous != 0).then_some(KeySignature { fifths: previous }),
                    ..Default::default()
                },
            };
            for (index, &(character, position)) in sequence.iter().enumerate() {
                let height = if character == '\u{e260}' { 52.0 } else { 54.0 };
                let anchor = if character == '\u{e260}' { 0.78 } else { 0.5 };
                let position = position + if clef == Clef::Bass { 1.0 } else { 0.0 };
                let top = (70.0 + position * 20.0 - anchor * height).round() as u32;
                for (x, y, pixel) in resized(&glyph(character), height).enumerate_pixels() {
                    if pixel[0] > 127 {
                        image.put_pixel(120 + index as u32 * spacing + x, top + y, Luma([0]));
                    }
                }
            }
            for &line in &staff.lines {
                for x in 20..680 {
                    image.put_pixel(x, line as u32, Luma([0]));
                }
            }
            for y in 70..=150 {
                image.put_pixel(100, y, Luma([0]));
            }
            let image = imageops::resize(
                &image,
                (700.0 * scale) as u32,
                (220.0 * scale) as u32,
                FilterType::Triangle,
            );
            let image = imageproc::contrast::threshold(
                &image,
                170,
                imageproc::contrast::ThresholdType::Binary,
            );
            staff.lines = staff.lines.map(|line| line * scale);
            staff.left *= scale;
            staff.right *= scale;
            (
                image,
                staff,
                vec![crate::model::Barline {
                    staff: 0,
                    x: 100.0 * scale,
                    kind: crate::model::BarlineKind::Double,
                    confirmed: false,
                }],
            )
        };
        for clef in [Clef::Treble, Clef::Bass] {
            for scale in [0.65, 1.0, 1.3] {
                for (previous, next, sequence) in &cases {
                    let (image, staff, bars) = fixture(clef, *previous, sequence, 28, scale);
                    let changes = recognize_key_changes(&image, &staff, 0, &bars);
                    assert_eq!(
                        changes.len(),
                        1,
                        "{clef:?}, {previous}->{next}, scale {scale}: {changes:?}"
                    );
                    assert_eq!(
                        changes[0].key_signature.fifths, *next,
                        "{clef:?}, {previous}->{next}, scale {scale}"
                    );
                    assert_eq!(changes[0].bounds.len(), sequence.len());
                    assert!(recognize_key_changes(&image, &staff, 0, &[]).is_empty());
                }
                for (previous, sequence, spacing) in [
                    (0, vec![('\u{e260}', 2.0)], 28),
                    (0, vec![('\u{e260}', 0.5), ('\u{e260}', 2.0)], 28),
                    (0, vec![('\u{e260}', 2.0), ('\u{e260}', 0.5)], 56),
                    (-2, vec![('\u{e260}', 2.0), ('\u{e260}', 0.5)], 28),
                    (0, vec![('\u{e261}', 2.0), ('\u{e261}', 0.5)], 28),
                ] {
                    let (image, staff, bars) = fixture(clef, previous, &sequence, spacing, scale);
                    assert!(
                        recognize_key_changes(&image, &staff, 0, &bars).is_empty(),
                        "{clef:?}, scale {scale}, {sequence:?}"
                    );
                }
            }
        }
    }

    fn printed_header(clef: char, clef_height: f32, meter: TimeSignature) -> (GrayImage, Staff) {
        let mut image = GrayImage::from_pixel(700, 220, Luma([255]));
        let staff = Staff {
            lines: [70.0, 90.0, 110.0, 130.0, 150.0],
            left: 20.0,
            right: 680.0,
            header: Default::default(),
        };
        let clef_image = resized(&glyph(clef), clef_height * 20.0);
        let clef_top = if clef == '\u{e062}' { 68 } else { 40 };
        let (meter_height, meter_top) = match meter {
            TimeSignature::Common => (40.0, 90),
            TimeSignature::Cut => (64.0, 78),
            _ => (78.0, 71),
        };
        for (template, left, top) in [
            (clef_image, 30, clef_top),
            (
                resized(&meter_template(meter), meter_height),
                160,
                meter_top,
            ),
        ] {
            for (x, y, pixel) in template.enumerate_pixels() {
                if pixel[0] > 127 {
                    image.put_pixel(left + x, top + y, Luma([0]));
                }
            }
        }
        for &line in &staff.lines {
            for x in 20..680 {
                image.put_pixel(x, line as u32, Luma([0]));
            }
        }
        (image, staff)
    }

    #[test]
    fn recognizes_supported_meters_and_rescaled_print() {
        for meter in [
            TimeSignature::Common,
            TimeSignature::Cut,
            TimeSignature::Meter {
                beats: 2,
                beat_unit: 2,
            },
            TimeSignature::Meter {
                beats: 2,
                beat_unit: 4,
            },
            TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
            TimeSignature::Meter {
                beats: 4,
                beat_unit: 4,
            },
            TimeSignature::Meter {
                beats: 3,
                beat_unit: 8,
            },
            TimeSignature::Meter {
                beats: 6,
                beat_unit: 8,
            },
            TimeSignature::Meter {
                beats: 9,
                beat_unit: 8,
            },
            TimeSignature::Meter {
                beats: 12,
                beat_unit: 8,
            },
        ] {
            let (image, _) = printed_header('\u{e062}', 3.6, meter);
            for scale in [0.65_f32, 1.3] {
                let scaled = imageops::resize(
                    &image,
                    (image.width() as f32 * scale) as u32,
                    (image.height() as f32 * scale) as u32,
                    FilterType::Triangle,
                );
                let score = crate::recognition::recognize(&scaled);
                let header = &score.staves.first().expect("staff detected").header;
                assert_eq!(header.clef, Clef::Bass, "scale {scale}: {header:?}");
                assert_eq!(
                    header.time_signature,
                    Some(meter),
                    "scale {scale}: {header:?}"
                );
            }
        }
    }

    #[test]
    fn keeps_real_notes_after_recognized_header() {
        let (mut image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
        imageproc::drawing::draw_filled_ellipse_mut(&mut image, (300, 140), 12, 7, Luma([0]));
        for y in 100..140 {
            image.put_pixel(311, y, Luma([0]));
        }
        let score = crate::recognition::recognize(&image);
        assert_eq!(score.notes.len(), 1, "{:?}", score.notes);
        assert_eq!(score.notes[0].pitch_label(), "A2");
        assert!((score.notes[0].x - 300.0).abs() < 3.0);
    }

    #[test]
    fn does_not_classify_a_real_leading_note_as_a_clef() {
        let mut image = GrayImage::from_pixel(700, 220, Luma([255]));
        for line in [70, 90, 110, 130, 150] {
            for x in 20..680 {
                image.put_pixel(x, line, Luma([0]));
            }
        }
        imageproc::drawing::draw_filled_ellipse_mut(&mut image, (65, 140), 12, 7, Luma([0]));
        for y in 100..140 {
            image.put_pixel(76, y, Luma([0]));
        }
        let score = crate::recognition::recognize(&image);
        assert_eq!(score.staves[0].header.clef, Clef::Unknown);
        assert!(score.staves[0].header.time_signature.is_none());
        assert_eq!(score.notes.len(), 1);
    }

    #[test]
    fn detects_bass_and_common_time_as_symbols_not_notes() {
        let (image, _) = printed_header('\u{e062}', 3.6, TimeSignature::Common);
        let score = crate::recognition::recognize(&image);
        assert_eq!(score.staves.len(), 1);
        let header = &score.staves[0].header;
        assert_eq!(header.clef, Clef::Bass, "{header:?}");
        assert_eq!(
            header.time_signature,
            Some(TimeSignature::Common),
            "{header:?}"
        );
        assert!(header.clef_bounds.is_some());
        assert!(header.time_bounds.is_some());
        assert!(score.notes.is_empty(), "{:?}", score.notes);
    }

    #[test]
    fn recovers_headers_when_curved_lines_place_the_edge_inside_the_clef() {
        for (character, height, clef) in [
            ('\u{e062}', 3.6, Clef::Bass),
            ('\u{e050}', 6.4, Clef::Treble),
        ] {
            let (source, staff) = printed_header(character, height, TimeSignature::Common);
            for scale in [0.75, 1.0, 1.4] {
                let image = imageops::resize(
                    &source,
                    (source.width() as f32 * scale) as u32,
                    (source.height() as f32 * scale) as u32,
                    FilterType::Triangle,
                );
                let binary = imageproc::contrast::threshold(
                    &image,
                    170,
                    imageproc::contrast::ThresholdType::Binary,
                );
                let mut local = staff.clone();
                local.lines = local.lines.map(|y| y * scale);
                local.left *= scale;
                local.right *= scale;
                let baseline = recognize_header(&binary, &local);
                assert_eq!(baseline.clef, clef);
                local.left = baseline.clef_bounds.as_ref().unwrap().left + local.spacing() * 1.5;
                assert_eq!(recognize_header_at(&binary, &local).clef, Clef::Unknown);
                let header = recognize_header(&binary, &local);
                assert_eq!(header.clef, clef, "scale {scale}: {header:?}");
                assert_eq!(header.time_signature, Some(TimeSignature::Common));
                assert_eq!(header.key_signature, baseline.key_signature);
            }
        }
    }

    #[test]
    fn detects_left_overhanging_treble_and_twelve_eight() {
        let meter = TimeSignature::Meter {
            beats: 12,
            beat_unit: 8,
        };
        for height in [6.3, 6.7, 7.1] {
            let (image, staff) = printed_header('\u{e050}', height, meter);
            for scale in [0.75, 1.0, 1.4] {
                let image = imageops::resize(
                    &image,
                    (image.width() as f32 * scale) as u32,
                    (image.height() as f32 * scale) as u32,
                    FilterType::Triangle,
                );
                let mut local = staff.clone();
                local.lines = local.lines.map(|line| line * scale);
                local.left *= scale;
                local.right *= scale;
                let baseline = recognize_header(&image, &local);
                assert_eq!(
                    baseline.clef,
                    Clef::Treble,
                    "height {height}, scale {scale}: {baseline:?}"
                );
                local.left = baseline.clef_bounds.as_ref().unwrap().left + local.spacing() * 0.35;
                let header = recognize_header(&image, &local);
                assert_eq!(
                    header.clef,
                    Clef::Treble,
                    "height {height}, scale {scale}: {header:?}"
                );
                assert_eq!(
                    header.time_signature,
                    Some(meter),
                    "height {height}, scale {scale}: {header:?}"
                );
            }
        }
    }

    #[test]
    fn detects_treble_and_numeric_time() {
        let (image, staff) = printed_header(
            '\u{e050}',
            6.4,
            TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
        );
        let header = recognize_header(&image, &staff);
        assert_eq!(header.clef, Clef::Treble, "{header:?}");
        assert_eq!(
            header.time_signature,
            Some(TimeSignature::Meter {
                beats: 3,
                beat_unit: 4
            }),
            "{header:?}"
        );
    }
}
