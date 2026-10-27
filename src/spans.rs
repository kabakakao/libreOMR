use image::{GrayImage, Luma, imageops};
use imageproc::region_labelling::{Connectivity, connected_components};

use crate::model::{BowContinuation, NoteAnchor, Score, Span, SpanKind, Staff};

#[cfg(test)]
mod boundary_tests;
mod contours;
mod wedges;

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
            span.wedge_point(fraction, branch)
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
    let endpoints = [span.start[1], span.end[1]];
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
            || height.abs() < gap * 0.35
            || !(0.018..=0.32).contains(&(height.abs() / (span.end[0] - span.start[0])))
        {
            return None;
        }
        span.start[1] = equations[0][3];
        span.end[1] = equations[0][3] + equations[1][3];
        span.height = height;
    }
    // A fit to unrelated ink can drift away from the very notes it proposes to join.
    if (span.start[1] - endpoints[0]).abs() > gap * 0.5
        || (span.end[1] - endpoints[1]).abs() > gap * 0.5
    {
        return None;
    }
    Some(span)
}

fn continuous_bow(image: &GrayImage, staff: &Staff, span: &Span) -> bool {
    let gap = staff.spacing();
    let mut hits = [0; 4];
    let mut missing = 0;
    for sample in 0..64 {
        let point = span.curve_point((sample as f32 + 0.5) / 64.0);
        let found = (-(gap * 0.14).ceil() as i32..=(gap * 0.14).ceil() as i32)
            .any(|dy| ink(image, point[0], point[1] + dy as f32));
        if found {
            hits[sample / 16] += 1;
            missing = 0;
        } else {
            missing += 1;
        }
        if missing > 8 {
            return false;
        }
    }
    hits.iter().all(|&hit| hit >= 11)
}

fn bind_bow(score: &Score, span: &mut Span, first: usize, last: usize) {
    let anchor = |note: usize, point: [f32; 2]| NoteAnchor {
        note,
        offset: [
            point[0] - score.notes[note].x,
            point[1] - score.notes[note].y,
        ],
    };
    span.start_anchor = Some(anchor(first, span.start));
    span.end_anchor = Some(anchor(last, span.end));
}

pub(crate) fn recognize(image: &GrayImage, score: &Score) -> Vec<Span> {
    let mut spans = Vec::new();
    let contours: Vec<_> = score
        .staves
        .iter()
        .enumerate()
        .map(|(i, staff)| contours::curves(image, staff, i, &score.barlines))
        .collect();
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
        for curve in &contours[staff_index] {
            if let Some((first, last)) = contours::endpoint_note(score, curve, false)
                .zip(contours::endpoint_note(score, curve, true))
                && score.bow_note_order(first, last)
            {
                let mut span = curve.clone();
                span.kind = if score.tie_compatible(first, last) {
                    SpanKind::Tie
                } else {
                    SpanKind::Slur
                };
                bind_bow(score, &mut span, first, last);
                candidates.push((1.0, span));
            }
        }
        for (first_index, first) in notes.iter().enumerate() {
            for last in notes.iter().skip(first_index + 1) {
                let width = (last.x - first.x) / gap;
                if width > 80.0 {
                    break;
                }
                if width < 1.1 {
                    continue;
                }
                let first_note = score
                    .notes
                    .iter()
                    .position(|n| std::ptr::eq(n, *first))
                    .unwrap();
                let last_note = score
                    .notes
                    .iter()
                    .position(|n| std::ptr::eq(n, *last))
                    .unwrap();
                let kind = if score.tie_compatible(first_note, last_note) {
                    SpanKind::Tie
                } else {
                    SpanKind::Slur
                };
                for side in [-1.0, 1.0] {
                    // Prefer measured contours to templates for the same notes.
                    if candidates.iter().any(|(q, s)| {
                        *q == 1.0
                            && s.height.signum() == side
                            && s.start_anchor
                                .as_ref()
                                .is_some_and(|a| a.note == first_note)
                            && s.end_anchor.as_ref().is_some_and(|a| a.note == last_note)
                    }) {
                        continue;
                    }
                    for offset in [0.1, 0.25, 0.4, 0.55, 0.8, 1.1, 1.4] {
                        for height in [0.45, 0.65, 0.9, 1.2, 1.6, 2.1, 2.7, 3.6, 5.0] {
                            if !(0.018..=0.28).contains(&(height / (width - 0.2))) {
                                continue;
                            }
                            let span = Span {
                                start_anchor: None,
                                end_anchor: None,
                                continuation: None,
                                wedge_openings: None,
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
                            let Some(mut span) = fit_bow(image, staff, span) else {
                                continue;
                            };
                            let quality = support(image, &span, staff, 0.0);
                            if quality >= 0.8 && continuous_bow(image, staff, &span) {
                                let first = score
                                    .notes
                                    .iter()
                                    .position(|note| std::ptr::eq(note, *first))
                                    .unwrap();
                                let last = score
                                    .notes
                                    .iter()
                                    .position(|note| std::ptr::eq(note, *last))
                                    .unwrap();
                                bind_bow(score, &mut span, first, last);
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
    spans.extend(continuations(image, score, &contours));
    wedges::attach_and_connect(image, score, &mut spans);
    spans.sort_by(|first, second| {
        first
            .staff
            .cmp(&second.staff)
            .then_with(|| first.start[0].total_cmp(&second.start[0]))
    });
    spans
}

/// Match both printed halves. A note near a margin alone is never a continuation.
fn continuations(image: &GrayImage, score: &Score, contours: &[Vec<Span>]) -> Vec<Span> {
    let mut result = Vec::new();
    for staff_index in 0..score.staves.len() {
        let rows = score.bow_staves(staff_index);
        let Some(next_staff) = rows
            .iter()
            .position(|&s| s == staff_index)
            .and_then(|i| rows.get(i + 1))
            .copied()
        else {
            continue;
        };
        let mut outgoing: Vec<_> = score
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.staff == staff_index)
            .collect();
        let mut incoming: Vec<_> = score
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.staff == next_staff)
            .collect();
        outgoing.sort_by(|a, b| b.1.x.total_cmp(&a.1.x));
        incoming.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        for side in [-1.0, 1.0] {
            let ends: Vec<_> = outgoing
                .iter()
                .take(4)
                .filter_map(|&(index, note)| {
                    observed_partial(score, &contours[staff_index], index, side, false)
                        .or_else(|| {
                            partial_bow(
                                image,
                                &score.staves[staff_index],
                                staff_index,
                                [note.x, note.y],
                                side,
                                false,
                            )
                        })
                        .map(|(q, s)| (index, q, s))
                })
                .collect();
            let starts: Vec<_> = incoming
                .iter()
                .take(4)
                .filter_map(|&(index, note)| {
                    observed_partial(score, &contours[next_staff], index, side, true)
                        .or_else(|| {
                            partial_bow(
                                image,
                                &score.staves[next_staff],
                                next_staff,
                                [note.x, note.y],
                                side,
                                true,
                            )
                        })
                        .map(|(q, s)| (index, q, s))
                })
                .collect();
            for voice in 1..=4 {
                let pair = ends
                    .iter()
                    .filter(|(i, _, _)| score.notes[*i].voice == voice)
                    .flat_map(|end| {
                        starts
                            .iter()
                            .filter(|(i, _, _)| score.notes[*i].voice == voice)
                            .map(move |start| (end, start))
                    })
                    .max_by(|(a, b), (c, d)| (a.1 + b.1).total_cmp(&(c.1 + d.1)));
                let Some(((first, _, exit), (last, _, entry))) = pair else {
                    continue;
                };
                let mut span = exit.clone();
                span.end = entry.end;
                span.kind = if score.tie_compatible(*first, *last) {
                    SpanKind::Tie
                } else {
                    SpanKind::Slur
                };
                bind_bow(score, &mut span, *first, *last);
                let upper = &score.staves[staff_index];
                let lower = &score.staves[next_staff];
                span.continuation = Some(BowContinuation {
                    exit_x: (exit.end[0] - upper.right) / upper.spacing(),
                    entry_x: (entry.start[0] - lower.left) / lower.spacing(),
                    exit_y: (exit.end[1] - upper.lines[0]) / upper.spacing(),
                    entry_y: (entry.start[1] - lower.lines[0]) / lower.spacing(),
                    exit_height: exit.height / upper.spacing(),
                    entry_height: entry.height / lower.spacing(),
                });
                result.push(span);
            }
        }
    }
    result
}

fn observed_partial(
    score: &Score,
    curves: &[Span],
    note: usize,
    side: f32,
    incoming: bool,
) -> Option<(f32, Span)> {
    let staff = &score.staves[score.notes[note].staff];
    let gap = staff.spacing();
    let header_end = [&staff.header.clef_bounds, &staff.header.time_bounds]
        .into_iter()
        .flatten()
        .chain(staff.header.key_bounds.iter())
        .map(|b| b.right)
        .fold(staff.left, f32::max);
    curves
        .iter()
        .filter(|curve| {
            curve.height.signum() == side
                && contours::endpoint_note(score, curve, incoming) == Some(note)
                && contours::endpoint_note(score, curve, !incoming).is_none()
                && if incoming {
                    curve.start[0] >= header_end - gap * 0.4
                        && curve.start[0] <= header_end + gap * 2.5
                } else {
                    curve.end[0] >= staff.right - gap * 2.5
                }
        })
        .max_by(|a, b| (a.end[0] - a.start[0]).total_cmp(&(b.end[0] - b.start[0])))
        .map(|s| (1.0, s.clone()))
}

fn partial_bow(
    image: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    note: [f32; 2],
    side: f32,
    incoming: bool,
) -> Option<(f32, Span)> {
    let gap = staff.spacing();
    let mut best: Option<(f32, Span)> = None;
    for margin in if incoming {
        &[0.25, 1.0, 2.0, 3.0, 4.0][..]
    } else {
        &[0.25, 0.75][..]
    } {
        let edge = if incoming {
            staff.left + gap * margin
        } else {
            staff.right - gap * margin
        };
        let width = if incoming {
            note[0] - edge
        } else {
            edge - note[0]
        };
        if !(2.0..=28.0).contains(&(width / gap)) {
            continue;
        }
        for offset in [0.25, 0.55, 0.8, 1.1, 1.4] {
            for rise in [0.6, 1.0, 1.5, 2.0] {
                for height in [0.45, 0.65, 0.9, 1.2] {
                    let tip = [note[0], note[1] + side * gap * offset];
                    let boundary = [edge, tip[1] + side * gap * rise];
                    let span = Span {
                        staff: staff_index,
                        kind: SpanKind::Slur,
                        start_anchor: None,
                        end_anchor: None,
                        continuation: None,
                        wedge_openings: None,
                        start: if incoming { boundary } else { tip },
                        end: if incoming { tip } else { boundary },
                        height: side * gap * height,
                        confirmed: false,
                    };
                    if support(image, &span, staff, 0.0) < 0.55 {
                        continue;
                    }
                    let Some(span) = fit_bow(image, staff, span) else {
                        continue;
                    };
                    let quality = support(image, &span, staff, 0.0);
                    if quality < 0.9 || !continuous_bow(image, staff, &span) {
                        continue;
                    }
                    if best.as_ref().is_none_or(|(q, _)| quality > *q) {
                        best = Some((quality, span));
                    }
                }
            }
        }
    }
    best
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
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
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

    #[test]
    #[ignore = "requires the local Concerto_d_Amore_-_BassCf.pdf reference score"]
    fn concerto_bass_reference_bows() {
        std::fs::create_dir_all("local/bow-analysis").unwrap();
        let (image, mut score) = if std::env::var_os("LIBREOMR_BOW_CACHE").is_some() {
            let image = image::open("local/bow-analysis/reference-page.png")
                .unwrap()
                .to_luma8();
            let score: Score = serde_json::from_slice(
                &std::fs::read("local/bow-analysis/reference-score.json").unwrap(),
            )
            .unwrap();
            (image, score)
        } else {
            let page = crate::document_import::open_document(std::path::Path::new(
                "Concerto_d_Amore_-_BassCf.pdf",
            ))
            .unwrap()
            .remove(0);
            let image = page.render().unwrap();
            let score = crate::recognition::recognize_with_progress(&image, |progress| {
                if progress.staff.is_none() {
                    eprintln!("{}", progress.stage);
                }
            });
            (image, score)
        };
        let binary =
            imageproc::contrast::threshold(&image, 170, imageproc::contrast::ThresholdType::Binary);
        score.spans = recognize(&binary, &score);
        score.recompute_accidentals();
        std::fs::write(
            "local/bow-analysis/improved-score.json",
            serde_json::to_vec_pretty(&score).unwrap(),
        )
        .unwrap();
        score.validate().unwrap();
        for (name, from_staff, from_x, to_staff, to_x) in [
            ("measure 9 to 10", 1, 1057.0, 1, 1146.0),
            ("measure 13 to 14", 1, 2297.0, 2, 366.0),
        ] {
            let note_at = |staff, x: f32| {
                score
                    .notes
                    .iter()
                    .position(|n| n.staff == staff && (n.x - x).abs() < 5.0)
                    .unwrap()
            };
            let a = note_at(from_staff, from_x);
            let b = note_at(to_staff, to_x);
            let found: Vec<_> = score
                .spans
                .iter()
                .filter(|s| {
                    s.kind == SpanKind::Tie
                        && s.start_anchor.as_ref().is_some_and(|n| n.note == a)
                        && s.end_anchor.as_ref().is_some_and(|n| n.note == b)
                })
                .collect();
            assert_eq!(found.len(), 1, "missing or duplicated {name}");
            assert_eq!(
                score.span_segments(found[0]).len(),
                if from_staff == to_staff { 1 } else { 2 }
            );
            eprintln!(
                "{name}: {:?} -> {:?}",
                found[0].start_anchor, found[0].end_anchor
            );
        }
    }

    pub(super) fn fixture(scale: f32) -> (GrayImage, Score) {
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

    pub(super) fn draw(image: &mut GrayImage, span: &Span) {
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
                        let p = span.wedge_point(part, side);
                        (p[0], p[1])
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
                        start_anchor: None,
                        end_anchor: None,
                        continuation: None,
                        wedge_openings: None,
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
                    start_anchor: None,
                    end_anchor: None,
                    continuation: None,
                    wedge_openings: None,
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
                            start_anchor: None,
                            end_anchor: None,
                            continuation: None,
                            wedge_openings: None,
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

    #[test]
    fn connects_both_halves_across_system_breaks_and_keeps_the_scan_geometry() {
        for scale in [0.75, 1.0, 1.5] {
            for side in [-1.0, 1.0] {
                let (mut image, mut score) = fixture(scale);
                image = GrayImage::from_pixel(image.width(), (700.0 * scale) as u32, Luma([255]));
                score.notes.clear();
                let mut next = score.staves[0].clone();
                next.lines = next.lines.map(|y| y + 300.0 * scale);
                score.staves.push(next);
                score.add_note(500.0 * scale, 150.0 * scale);
                score.add_note(150.0 * scale, 450.0 * scale);
                let exit = Span {
                    staff: 0,
                    kind: SpanKind::Tie,
                    start_anchor: None,
                    end_anchor: None,
                    continuation: None,
                    wedge_openings: None,
                    start: [500.0 * scale, (150.0 + side * 16.0) * scale],
                    end: [615.0 * scale, (150.0 + side * 36.0) * scale],
                    height: side * 13.0 * scale,
                    confirmed: false,
                };
                let entry = Span {
                    staff: 1,
                    start: [25.0 * scale, (450.0 + side * 36.0) * scale],
                    end: [150.0 * scale, (450.0 + side * 16.0) * scale],
                    ..exit.clone()
                };
                draw(&mut image, &exit);
                assert!(
                    recognize(&image, &score).is_empty(),
                    "one half is insufficient"
                );
                draw(&mut image, &entry);
                let spans = recognize(&image, &score);
                assert_eq!(spans.len(), 1, "scale {scale}, side {side}: {spans:?}");
                assert_eq!(spans[0].kind, SpanKind::Tie);
                assert_eq!(spans[0].start_anchor.as_ref().unwrap().note, 0);
                assert_eq!(spans[0].end_anchor.as_ref().unwrap().note, 1);
                for segment in score.span_segments(&spans[0]) {
                    assert!(
                        support(&image, &segment, &score.staves[segment.staff], 0.0) >= 0.85,
                        "{segment:?}"
                    );
                }
                score.spans = spans;
                assert!(score.validate().is_ok());
                let restored: Score =
                    serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
                assert!(restored.validate().is_ok());
                assert!(restored.spans[0].continuation.is_some());
                score.staff_assignments = vec![
                    crate::model::StaffAssignment {
                        staff: 0,
                        part: 0,
                        system: 0,
                        name: "A".into(),
                    },
                    crate::model::StaffAssignment {
                        staff: 1,
                        part: 1,
                        system: 0,
                        name: "B".into(),
                    },
                ];
                assert!(
                    recognize(&image, &score).is_empty(),
                    "simultaneous parts cannot form a continuation"
                );
            }
        }
    }

    #[test]
    fn detects_long_shallow_bows_but_rejects_curves_far_from_the_notes() {
        let (mut image, mut score) = fixture(1.0);
        image = GrayImage::from_pixel(1400, image.height(), Luma([255]));
        score.staves[0].right = 1380.0;
        score.notes[1].x = 1250.0;
        let mut span = Span {
            staff: 0,
            kind: SpanKind::Tie,
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            start: [152.0, 166.0],
            end: [1248.0, 166.0],
            height: 24.0,
            confirmed: false,
        };
        draw(&mut image, &span);
        let detected = recognize(&image, &score);
        assert_eq!(detected.len(), 1, "{detected:?}");
        image.fill(255);
        span.start[1] = 40.0;
        span.end[1] = 40.0;
        draw(&mut image, &span);
        assert!(recognize(&image, &score).is_empty());
    }
}
