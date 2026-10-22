use image::{GrayImage, Luma, imageops};
use imageproc::region_labelling::{Connectivity, connected_components};

use crate::model::{Score, Span, SpanKind, Staff};

fn ink(image: &GrayImage, column: f32, row: f32) -> bool {
    let column = column.round() as i32;
    let row = row.round() as i32;
    column >= 0
        && row >= 0
        && column < image.width() as i32
        && row < image.height() as i32
        && image.get_pixel(column as u32, row as u32)[0] == 0
}

fn support(image: &GrayImage, span: &Span, staff: &Staff, branch: f32) -> f32 {
    let gap = staff.spacing();
    let mut hits = 0;
    let mut samples = 0;
    for sample in 2..39 {
        let fraction = sample as f32 / 40.0;
        let point = if branch == 0.0 {
            span.curve_point(fraction)
        } else {
            let opening = if span.kind == SpanKind::Crescendo {
                fraction
            } else {
                1.0 - fraction
            };
            [
                span.start[0] + (span.end[0] - span.start[0]) * fraction,
                span.start[1]
                    + (span.end[1] - span.start[1]) * fraction
                    + branch * span.height * opening,
            ]
        };
        if staff.lines.iter().any(|line| {
            (point[1] - line).abs() < gap * 0.16
                && ink(image, point[0] - gap * 2.0, *line)
                && ink(image, point[0] + gap * 2.0, *line)
        }) {
            continue;
        }
        samples += 1;
        let offset = (-(gap * 0.14).ceil() as i32..=(gap * 0.14).ceil() as i32)
            .filter(|offset| ink(image, point[0], point[1] + *offset as f32))
            .min_by_key(|offset| offset.abs());
        if let Some(offset) = offset {
            let row = point[1] + offset as f32;
            let thickness: usize = [-1.0, 1.0]
                .iter()
                .map(|side| {
                    (1..=(gap * 0.5).ceil() as i32)
                        .take_while(|distance| ink(image, point[0], row + side * *distance as f32))
                        .count()
                })
                .sum();
            if thickness as f32 <= gap * if branch == 0.0 { 0.4 } else { 0.24 } {
                hits += 1;
            }
        }
    }
    if samples < 22 {
        0.0
    } else {
        hits as f32 / samples as f32
    }
}

fn fit_bow(image: &GrayImage, staff: &Staff, mut span: Span) -> Option<Span> {
    let gap = staff.spacing();
    for _iteration in 0..3 {
        let mut equations = [[0.0_f32; 4]; 3];
        let mut count = 0;
        for sample in 2..39 {
            let fraction = sample as f32 / 40.0;
            let point = span.curve_point(fraction);
            let radius = (gap * 0.45).ceil() as i32;
            let row = (-radius..=radius)
                .filter_map(|offset| {
                    let row = point[1].round() + offset as f32;
                    if !ink(image, point[0], row)
                        || staff.lines.iter().any(|line| {
                            (row - line).abs() < gap * 0.16
                                && ink(image, point[0] - gap * 2.0, *line)
                                && ink(image, point[0] + gap * 2.0, *line)
                        })
                    {
                        return None;
                    }
                    let thickness: usize = [-1.0, 1.0]
                        .iter()
                        .map(|side| {
                            (1..=(gap * 0.5).ceil() as i32)
                                .take_while(|distance| {
                                    ink(image, point[0], row + side * *distance as f32)
                                })
                                .count()
                        })
                        .sum();
                    (thickness as f32 <= gap * 0.4).then_some(row)
                })
                .min_by(|first, second| {
                    (first - point[1])
                        .abs()
                        .total_cmp(&(second - point[1]).abs())
                });
            let Some(row) = row else {
                continue;
            };
            let basis = [1.0, fraction, 4.0 * fraction * (1.0 - fraction)];
            for axis in 0..3 {
                for column in 0..3 {
                    equations[axis][column] += basis[axis] * basis[column];
                }
                equations[axis][3] += basis[axis] * row;
            }
            count += 1;
        }
        if count < 22 {
            return None;
        }
        for axis in 0..3 {
            let pivot = (axis..3).max_by(|first, second| {
                equations[*first][axis]
                    .abs()
                    .total_cmp(&equations[*second][axis].abs())
            })?;
            equations.swap(axis, pivot);
            let divisor = equations[axis][axis];
            if divisor.abs() < 0.0001 {
                return None;
            }
            for value in &mut equations[axis][axis..] {
                *value /= divisor;
            }
            let pivot_row = equations[axis];
            for (row_index, row) in equations.iter_mut().enumerate() {
                if row_index == axis {
                    continue;
                }
                let factor = row[axis];
                for (value, pivot_value) in row[axis..].iter_mut().zip(&pivot_row[axis..]) {
                    *value -= factor * pivot_value;
                }
            }
        }
        let height = equations[2][3];
        if height.signum() != span.height.signum()
            || !(0.055..=0.32).contains(&(height.abs() / (span.end[0] - span.start[0])))
        {
            return None;
        }
        span.start[1] = equations[0][3];
        span.end[1] = equations[0][3] + equations[1][3];
        span.height = height;
    }
    Some(span)
}

pub(crate) fn recognize(image: &GrayImage, score: &Score) -> Vec<Span> {
    let mut spans = Vec::new();
    for (staff_index, staff) in score.staves.iter().enumerate() {
        spans.extend(hairpins(image, staff, staff_index));
        let gap = staff.spacing();
        let mut notes: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.staff == staff_index)
            .collect();
        notes.sort_by(|first, second| first.x.total_cmp(&second.x));
        let mut candidates = Vec::new();
        for (first_index, first) in notes.iter().enumerate() {
            for (last_index, last) in notes.iter().enumerate().skip(first_index + 1) {
                let width = (last.x - first.x) / gap;
                if width > 28.0 {
                    break;
                }
                if width < 1.1 {
                    continue;
                }
                for side in [-1.0, 1.0] {
                    for offset in [0.1, 0.25, 0.4, 0.55, 0.8, 1.1, 1.4] {
                        for height in [0.45, 0.65, 0.9, 1.2, 1.6, 2.1, 2.7] {
                            if !(0.045..=0.28).contains(&(height / (width - 0.2))) {
                                continue;
                            }
                            let kind = if last_index == first_index + 1
                                && first.pitch == last.pitch
                                && first.accidental == last.accidental
                                && !score.rests.iter().any(|rest| {
                                    rest.staff == staff_index && rest.x > first.x && rest.x < last.x
                                }) {
                                SpanKind::Tie
                            } else {
                                SpanKind::Slur
                            };
                            let span = Span {
                                staff: staff_index,
                                kind,
                                start: [first.x + gap * 0.1, first.y + side * gap * offset],
                                end: [last.x - gap * 0.1, last.y + side * gap * offset],
                                height: side * gap * height,
                                confirmed: false,
                            };
                            if support(image, &span, staff, 0.0) < 0.45 {
                                continue;
                            }
                            let Some(span) = fit_bow(image, staff, span) else {
                                continue;
                            };
                            let quality = support(image, &span, staff, 0.0);
                            if quality >= 0.8 {
                                candidates.push((quality, span));
                            }
                        }
                    }
                }
            }
        }
        candidates.sort_by(|first, second| {
            (second.0 * (second.1.end[0] - second.1.start[0]).sqrt())
                .total_cmp(&(first.0 * (first.1.end[0] - first.1.start[0]).sqrt()))
        });
        let strong: Vec<_> = candidates
            .iter()
            .filter(|(quality, _)| *quality >= 0.87)
            .map(|(_, span)| span.clone())
            .collect();
        candidates.retain(|(quality, span)| {
            *quality >= 0.87
                || strong.iter().any(|other| {
                    (span.start[0] - other.start[0]).abs() < gap * 0.3
                        && other.end[0] - span.end[0] > gap * 0.6
                        && span.height.signum() == other.height.signum()
                        && (span.end[1]
                            - other.curve_point(
                                (span.end[0] - other.start[0]) / (other.end[0] - other.start[0]),
                            )[1])
                            .abs()
                            > gap * 0.35
                })
        });
        for (_, span) in candidates {
            if spans.iter().any(|other| {
                if other.staff != span.staff
                    || !matches!(other.kind, SpanKind::Slur | SpanKind::Tie)
                {
                    return false;
                }
                let overlap = span.end[0].min(other.end[0]) - span.start[0].max(other.start[0]);
                let width = (span.end[0] - span.start[0]).min(other.end[0] - other.start[0]);
                if overlap < width * 0.8 {
                    return false;
                }
                let left = span.start[0].max(other.start[0]);
                let distances: Vec<_> = (1..=10)
                    .map(|sample| {
                        let column = left + overlap * sample as f32 / 10.0;
                        let first = span
                            .curve_point((column - span.start[0]) / (span.end[0] - span.start[0]));
                        let second = other.curve_point(
                            (column - other.start[0]) / (other.end[0] - other.start[0]),
                        );
                        (first[1] - second[1]).abs()
                    })
                    .collect();
                distances.iter().sum::<f32>() / 10.0 < gap * 0.26
                    && distances.iter().all(|distance| *distance < gap * 0.35)
            }) {
                continue;
            }
            spans.push(span);
        }
    }
    spans.sort_by(|first, second| {
        first
            .staff
            .cmp(&second.staff)
            .then_with(|| first.start[0].total_cmp(&second.start[0]))
    });
    spans
}

fn hairpins(image: &GrayImage, staff: &Staff, staff_index: usize) -> Vec<Span> {
    let gap = staff.spacing();
    let left = staff.left.max(0.0) as u32;
    let top = (staff.lines[0] - gap * 6.0).max(0.0) as u32;
    let right = staff.right.min(image.width() as f32 - 1.0) as u32;
    let bottom = (staff.lines[4] + gap * 6.0).min(image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return Vec::new();
    }
    let scale = 12.0 / gap;
    let mut local = imageops::resize(
        &imageops::crop_imm(image, left, top, right - left, bottom - top).to_image(),
        ((right - left) as f32 * scale).round().max(1.0) as u32,
        ((bottom - top) as f32 * scale).round().max(1.0) as u32,
        imageops::FilterType::Triangle,
    );
    for (column, row, pixel) in local.enumerate_pixels_mut() {
        let original_y = top as f32 + row as f32 / scale;
        let line = staff
            .lines
            .iter()
            .any(|line| (original_y - line).abs() < gap * 0.12);
        let crossing = ink(
            image,
            left as f32 + column as f32 / scale,
            original_y - gap * 0.22,
        ) && ink(
            image,
            left as f32 + column as f32 / scale,
            original_y + gap * 0.22,
        );
        pixel[0] = if pixel[0] < 220 && (!line || crossing) {
            0
        } else {
            255
        };
    }
    let labels = connected_components(&local, Connectivity::Eight, Luma([255]));
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut bounds = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32); maximum + 1];
    for (column, row, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let entry = &mut bounds[label[0] as usize];
        entry.0 = entry.0.min(column);
        entry.1 = entry.1.min(row);
        entry.2 = entry.2.max(column);
        entry.3 = entry.3.max(row);
        entry.4 += 1;
    }
    let mut spans = Vec::new();
    for (label, &(min_x, min_y, max_x, max_y, area)) in bounds.iter().enumerate().skip(1) {
        if area == 0 {
            continue;
        }
        let width = (max_x - min_x + 1) as f32;
        let height = (max_y - min_y + 1) as f32;
        if !(2.5..=30.0).contains(&(width / 12.0))
            || !(0.35..=2.0).contains(&(height / 12.0))
            || area as f32 > width * height * 0.45
        {
            continue;
        }
        let end_range = (width * 0.1).ceil() as u32;
        let extent = |first: bool| {
            let mut minimum = u32::MAX;
            let mut maximum = 0;
            for column in min_x..=max_x {
                for row in min_y..=max_y {
                    if (if first {
                        column <= min_x + end_range
                    } else {
                        column + end_range >= max_x
                    }) && labels.get_pixel(column, row)[0] as usize == label
                    {
                        minimum = minimum.min(row);
                        maximum = maximum.max(row);
                    }
                }
            }
            (minimum, maximum)
        };
        let first = extent(true);
        let last = extent(false);
        let first_height = (first.1 - first.0) as f32;
        let last_height = (last.1 - last.0) as f32;
        let kind = if first_height < last_height * 0.35 {
            SpanKind::Crescendo
        } else if last_height < first_height * 0.35 {
            SpanKind::Decrescendo
        } else {
            continue;
        };
        let span = Span {
            staff: staff_index,
            kind,
            start: [
                left as f32 + min_x as f32 / scale,
                top as f32 + (first.0 + first.1) as f32 / (2.0 * scale),
            ],
            end: [
                left as f32 + max_x as f32 / scale,
                top as f32 + (last.0 + last.1) as f32 / (2.0 * scale),
            ],
            height: (height - 1.0) / (2.0 * scale),
            confirmed: false,
        };
        if support(image, &span, staff, -1.0) >= 0.87 && support(image, &span, staff, 1.0) >= 0.87 {
            spans.push(span);
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clef, StaffHeader};

    fn fixture(scale: f32) -> (GrayImage, Score) {
        let image =
            GrayImage::from_pixel((640.0 * scale) as u32, (300.0 * scale) as u32, Luma([255]));
        let mut score = Score {
            staves: vec![Staff {
                lines: [100.0, 120.0, 140.0, 160.0, 180.0].map(|value| value * scale),
                left: 20.0 * scale,
                right: 620.0 * scale,
                header: StaffHeader {
                    clef: Clef::Bass,
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        score.add_note(150.0 * scale, 150.0 * scale);
        score.add_note(300.0 * scale, 150.0 * scale);
        (image, score)
    }

    fn draw(image: &mut GrayImage, span: &Span) {
        for sample in 0..120 {
            let fraction = sample as f32 / 120.0;
            let next = (sample + 1) as f32 / 120.0;
            if matches!(span.kind, SpanKind::Slur | SpanKind::Tie) {
                let first = span.curve_point(fraction);
                let last = span.curve_point(next);
                imageproc::drawing::draw_line_segment_mut(
                    image,
                    (first[0], first[1]),
                    (last[0], last[1]),
                    Luma([0]),
                );
            } else {
                for side in [-1.0, 1.0] {
                    let point = |part: f32| {
                        let opening = if span.kind == SpanKind::Crescendo {
                            part
                        } else {
                            1.0 - part
                        };
                        (
                            span.start[0] + (span.end[0] - span.start[0]) * part,
                            span.start[1] + side * span.height * opening,
                        )
                    };
                    imageproc::drawing::draw_line_segment_mut(
                        image,
                        point(fraction),
                        point(next),
                        Luma([0]),
                    );
                }
            }
        }
    }

    #[test]
    fn detects_hairpins_and_distinguishes_ties_from_slurs() {
        for scale in [0.75, 1.0, 1.5] {
            for kind in SpanKind::ALL {
                for above in [false, true] {
                    let (mut image, mut score) = fixture(scale);
                    let side = if above { -1.0 } else { 1.0 };
                    if kind == SpanKind::Slur {
                        score.notes[1].pitch += 2;
                        score.notes[1].y -= 20.0 * scale;
                    }
                    let span = Span {
                        staff: 0,
                        kind,
                        start: [
                            152.0 * scale,
                            if matches!(kind, SpanKind::Crescendo | SpanKind::Decrescendo) {
                                (140.0 + side * 100.0) * scale
                            } else {
                                (150.0 + side * 16.0) * scale
                            },
                        ],
                        end: [
                            298.0 * scale,
                            if matches!(kind, SpanKind::Crescendo | SpanKind::Decrescendo) {
                                (140.0 + side * 100.0) * scale
                            } else {
                                score.notes[1].y + side * 16.0 * scale
                            },
                        ],
                        height: if matches!(kind, SpanKind::Crescendo | SpanKind::Decrescendo) {
                            10.0 * scale
                        } else {
                            side * 24.0 * scale
                        },
                        confirmed: false,
                    };
                    draw(&mut image, &span);
                    let detected = recognize(&image, &score);
                    assert_eq!(
                        detected.len(),
                        1,
                        "{kind:?}, above {above}, scale {scale}: {detected:?}"
                    );
                    assert_eq!(detected[0].kind, kind, "{detected:?}");
                }
            }
        }
    }

    #[test]
    fn intervening_notes_rests_and_changed_accidentals_prevent_ties() {
        for variant in 0..3 {
            let (mut image, mut score) = fixture(1.0);
            match variant {
                0 => {
                    score.notes[1].accidental = -1;
                }
                1 => {
                    score.add_rest(220.0, 140.0, crate::model::Duration::Quarter);
                }
                _ => {
                    score.add_note(220.0, 130.0);
                }
            }
            draw(
                &mut image,
                &Span {
                    staff: 0,
                    kind: SpanKind::Slur,
                    start: [152.0, 166.0],
                    end: [298.0, 166.0],
                    height: 24.0,
                    confirmed: false,
                },
            );
            let spans = recognize(&image, &score);
            assert!(!spans.is_empty(), "variant {variant}");
            assert!(
                spans.iter().all(|span| span.kind == SpanKind::Slur),
                "{spans:?}"
            );
        }
    }

    #[test]
    fn detects_hairpins_five_gaps_outside_the_staff() {
        for scale in [0.75, 1.0, 1.5] {
            let (_, score) = fixture(scale);
            let mut staff = score.staves[0].clone();
            staff.lines = staff.lines.map(|line| line + 100.0 * scale);
            for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
                for side in [-1.0, 1.0] {
                    let mut image = GrayImage::from_pixel(
                        (640.0 * scale) as u32,
                        (480.0 * scale) as u32,
                        Luma([255]),
                    );
                    let row = if side < 0.0 {
                        staff.lines[0] - 5.0 * staff.spacing()
                    } else {
                        staff.lines[4] + 5.0 * staff.spacing()
                    };
                    draw(
                        &mut image,
                        &Span {
                            staff: 0,
                            kind,
                            start: [152.0 * scale, row],
                            end: [298.0 * scale, row],
                            height: 10.0 * scale,
                            confirmed: false,
                        },
                    );
                    let spans = hairpins(&image, &staff, 0);
                    assert_eq!(
                        spans.len(),
                        1,
                        "{kind:?}, side {side}, scale {scale}: {spans:?}"
                    );
                    assert_eq!(spans[0].kind, kind);
                }
            }
        }
    }

    #[test]
    fn lines_beams_and_single_diagonals_are_not_spans() {
        let (mut image, score) = fixture(1.0);
        for row in [100.0, 120.0, 140.0, 160.0, 180.0, 215.0] {
            imageproc::drawing::draw_line_segment_mut(
                &mut image,
                (20.0, row),
                (620.0, row),
                Luma([0]),
            );
        }
        for row in 50..55 {
            imageproc::drawing::draw_line_segment_mut(
                &mut image,
                (150.0, row as f32),
                (300.0, row as f32),
                Luma([0]),
            );
        }
        imageproc::drawing::draw_line_segment_mut(
            &mut image,
            (400.0, 220.0),
            (550.0, 235.0),
            Luma([0]),
        );
        assert!(recognize(&image, &score).is_empty());
    }
}
