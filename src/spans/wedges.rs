use super::*;

/// Open-ended continuations have two disconnected strokes. Recover straight,
/// thin components and pair them only when both printed branches agree.
fn open_fragments(image: &GrayImage, staff: &Staff, staff_index: usize) -> Vec<Span> {
    let gap = staff.spacing();
    let scale = 12.0 / gap;
    let left = staff.left.max(0.0) as u32;
    let right = staff.right.min(image.width() as f32) as u32;
    let top = (staff.lines[0] - gap * 6.0).max(0.0) as u32;
    let bottom = (staff.lines[4] + gap * 6.0).min(image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return vec![];
    }
    let mut local = imageops::resize(
        &imageops::crop_imm(image, left, top, right - left, bottom - top).to_image(),
        ((right - left) as f32 * scale).round().max(1.0) as u32,
        ((bottom - top) as f32 * scale).round().max(1.0) as u32,
        imageops::FilterType::Triangle,
    );
    for (_, y, pixel) in local.enumerate_pixels_mut() {
        let row = top as f32 + y as f32 / scale;
        pixel[0] = if pixel[0] < 220
            && staff
                .lines
                .iter()
                .all(|line| (row - line).abs() > gap * 0.16)
        {
            0
        } else {
            255
        };
    }
    let labels = connected_components(&local, Connectivity::Eight, Luma([255]));
    let maximum = labels.pixels().map(|p| p[0]).max().unwrap_or(0) as usize;
    let mut components = vec![Vec::new(); maximum + 1];
    for (x, y, p) in labels.enumerate_pixels() {
        if p[0] != 0 {
            components[p[0] as usize].push((x, y));
        }
    }
    // Each line stores x bounds, intercept and slope in normalized pixels.
    let mut lines = vec![];
    for pixels in components.iter().skip(1).filter(|c| !c.is_empty()) {
        let x0 = pixels.iter().map(|p| p.0).min().unwrap();
        let x1 = pixels.iter().map(|p| p.0).max().unwrap();
        let width = (x1 - x0) as f32;
        if !(30.0..=480.0).contains(&width) {
            continue;
        }
        let mut columns = vec![(0.0, 0_u32, u32::MAX, 0); (x1 - x0 + 1) as usize];
        for &(x, y) in pixels {
            let c = &mut columns[(x - x0) as usize];
            c.0 += y as f32;
            c.1 += 1;
            c.2 = c.2.min(y);
            c.3 = c.3.max(y);
        }
        if columns.iter().any(|c| c.1 == 0 || c.3 - c.2 > 3) {
            continue;
        }
        let count = columns.len() as f32;
        let mean_x = width * 0.5;
        let mean_y = columns.iter().map(|c| c.0 / c.1 as f32).sum::<f32>() / count;
        let slope = columns
            .iter()
            .enumerate()
            .map(|(x, c)| (x as f32 - mean_x) * (c.0 / c.1 as f32 - mean_y))
            .sum::<f32>()
            / (0..columns.len())
                .map(|x| (x as f32 - mean_x).powi(2))
                .sum::<f32>();
        let intercept = mean_y - slope * mean_x;
        if slope.abs() > 0.25
            || columns
                .iter()
                .enumerate()
                .any(|(x, c)| (c.0 / c.1 as f32 - intercept - slope * x as f32).abs() > 1.8)
        {
            continue;
        }
        lines.push((x0 as f32, x1 as f32, intercept, slope));
    }
    let mut fragments = vec![];
    for (i, a) in lines.iter().enumerate() {
        for b in lines.iter().skip(i + 1) {
            if (a.0 - b.0).abs() > 6.0 || (a.1 - b.1).abs() > 6.0 {
                continue;
            }
            let x0 = a.0.max(b.0);
            let x1 = a.1.min(b.1);
            let at = |line: &(f32, f32, f32, f32), x: f32| line.2 + line.3 * (x - line.0);
            let center = |x| (at(a, x) + at(b, x)) * 0.5;
            let opening = |x| (at(a, x) - at(b, x)).abs() * 0.5;
            let first = opening(x0);
            let last = opening(x1);
            if first.min(last) < 1.5
                || first.max(last) > 16.8
                || (first - last).abs() < 1.8
                || (at(a, x0) - at(b, x0)).signum() != (at(a, x1) - at(b, x1)).signum()
            {
                continue;
            }
            let start = [left as f32 + x0 / scale, top as f32 + center(x0) / scale];
            let end = [left as f32 + x1 / scale, top as f32 + center(x1) / scale];
            if [start[1], end[1]]
                .iter()
                .any(|&y| y > staff.lines[0] - gap * 0.4 && y < staff.lines[4] + gap * 0.4)
            {
                continue;
            }
            let maximum = first.max(last);
            let span = Span {
                staff: staff_index,
                kind: if last > first {
                    SpanKind::Crescendo
                } else {
                    SpanKind::Decrescendo
                },
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: Some([first / maximum, last / maximum]),
                start,
                end,
                height: maximum / scale,
                confirmed: false,
            };
            if support(image, &span, staff, -1.0) >= 0.87
                && support(image, &span, staff, 1.0) >= 0.87
            {
                fragments.push(span);
            }
        }
    }
    fragments
}

fn nearest_note(score: &Score, staff: usize, x: f32) -> Option<usize> {
    score
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.staff == staff && (n.x - x).abs() <= score.staves[staff].spacing() * 2.0)
        .min_by(|a, b| (a.1.x - x).abs().total_cmp(&(b.1.x - x).abs()))
        .map(|(i, _)| i)
}

fn same_fragment(a: &Span, b: &Span, gap: f32) -> bool {
    a.staff == b.staff
        && a.kind == b.kind
        && (a.start[0] - b.start[0]).abs() < gap * 0.5
        && (a.end[0] - b.end[0]).abs() < gap * 0.5
        && (a.start[1] - b.start[1]).abs() < gap * 0.5
}

pub(super) fn attach_and_connect(image: &GrayImage, score: &Score, spans: &mut Vec<Span>) {
    let fragments: Vec<Vec<Span>> = score
        .staves
        .iter()
        .enumerate()
        .map(|(i, staff)| {
            let mut candidates: Vec<_> = spans
                .iter()
                .filter(|s| s.staff == i && !s.is_bow())
                .cloned()
                .collect();
            candidates.extend(open_fragments(image, staff, i));
            candidates
        })
        .collect();
    for staff in 0..score.staves.len() {
        let rows = score.bow_staves(staff);
        let Some(next) = rows
            .iter()
            .position(|&s| s == staff)
            .and_then(|i| rows.get(i + 1))
            .copied()
        else {
            continue;
        };
        let upper = &score.staves[staff];
        let lower = &score.staves[next];
        let g = upper.spacing();
        let h = lower.spacing();
        let mut pairs = vec![];
        for (i, exit) in fragments[staff].iter().enumerate() {
            if exit.end[0] < upper.right - g * 2.0 || exit.wedge_opening(1.0) < g * 0.1 {
                continue;
            }
            for (j, entry) in fragments[next].iter().enumerate() {
                if entry.kind != exit.kind
                    || entry.start[0] < score.span_entry_x(next) - h * 0.75
                    || entry.start[0] > score.span_entry_x(next) + h * 2.5
                    || entry.wedge_opening(0.0) < h * 0.1
                {
                    continue;
                }
                let level = ((exit.end[1] - upper.lines[0]) / g
                    - (entry.start[1] - lower.lines[0]) / h)
                    .abs();
                let opening = (exit.wedge_opening(1.0) / g - entry.wedge_opening(0.0) / h).abs();
                if level > 0.6 || opening > 0.45 {
                    continue;
                }
                if let Some((a, b)) = nearest_note(score, staff, exit.start[0]).zip(nearest_note(
                    score,
                    next,
                    entry.end[0],
                )) && score.bow_note_order(a, b)
                {
                    pairs.push((level + opening, i, j, a, b));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut used_exit = std::collections::HashSet::new();
        let mut used_entry = std::collections::HashSet::new();
        for (_, i, j, a, b) in pairs {
            if used_exit.contains(&i) || used_entry.contains(&j) {
                continue;
            }
            used_exit.insert(i);
            used_entry.insert(j);
            let exit = &fragments[staff][i];
            let entry = &fragments[next][j];
            let mut joined = exit.clone();
            joined.end = entry.end;
            joined.height = exit.height.max(entry.height / h * g);
            joined.wedge_openings = Some([
                exit.wedge_opening(0.0) / joined.height,
                entry.wedge_opening(1.0) / h * g / joined.height,
            ]);
            bind_bow(score, &mut joined, a, b);
            joined.continuation = Some(BowContinuation {
                exit_x: (exit.end[0] - upper.right) / g,
                entry_x: (entry.start[0] - lower.left) / h,
                exit_y: (exit.end[1] - upper.lines[0]) / g,
                entry_y: (entry.start[1] - lower.lines[0]) / h,
                exit_height: exit.wedge_opening(1.0) / g,
                entry_height: entry.wedge_opening(0.0) / h,
            });
            spans.retain(|s| !same_fragment(s, exit, g) && !same_fragment(s, entry, h));
            spans.push(joined);
        }
    }
    for span in spans
        .iter_mut()
        .filter(|s| !s.is_bow() && s.start_anchor.is_none() && s.end_anchor.is_none())
    {
        if let Some((a, b)) = nearest_note(score, span.staff, span.start[0]).zip(nearest_note(
            score,
            span.staff,
            span.end[0],
        )) && score.bow_note_order(a, b)
        {
            bind_bow(score, span, a, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{draw, fixture};
    use super::*;

    #[test]
    fn recognizes_both_wedge_directions_across_systems_with_open_continuations() {
        for scale in [0.75, 1.0, 1.5] {
            for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
                for above in [false, true] {
                    let (_, mut score) = fixture(scale);
                    let mut image = GrayImage::from_pixel(
                        (640.0 * scale) as u32,
                        (700.0 * scale) as u32,
                        Luma([255]),
                    );
                    score.notes.clear();
                    let mut next = score.staves[0].clone();
                    next.lines = next.lines.map(|y| y + 300.0 * scale);
                    next.header.key_bounds.push(crate::model::SymbolBounds {
                        left: 45.0 * scale,
                        right: 70.0 * scale,
                        top: 400.0 * scale,
                        bottom: 480.0 * scale,
                    });
                    score.staves.push(next);
                    score.add_note(500.0 * scale, 150.0 * scale).unwrap();
                    score.add_note(180.0 * scale, 450.0 * scale).unwrap();
                    let y = if above { 40.0 } else { 240.0 };
                    let (out, inside) = if kind == SpanKind::Crescendo {
                        ([0.0, 0.5], [0.5, 1.0])
                    } else {
                        ([1.0, 0.5], [0.5, 0.0])
                    };
                    let exit = Span {
                        staff: 0,
                        kind,
                        start_anchor: None,
                        end_anchor: None,
                        continuation: None,
                        wedge_openings: Some(out),
                        start: [500.0 * scale, y * scale],
                        end: [615.0 * scale, y * scale],
                        height: 12.0 * scale,
                        confirmed: false,
                    };
                    let entry = Span {
                        staff: 1,
                        start: [80.0 * scale, (y + 300.0) * scale],
                        end: [180.0 * scale, (y + 300.0) * scale],
                        wedge_openings: Some(inside),
                        ..exit.clone()
                    };
                    draw(&mut image, &exit);
                    assert!(
                        recognize(&image, &score)
                            .iter()
                            .all(|s| s.continuation.is_none()),
                        "one half is insufficient"
                    );
                    draw(&mut image, &entry);
                    let spans = recognize(&image, &score);
                    let joined: Vec<_> = spans
                        .iter()
                        .filter(|s| s.kind == kind && s.continuation.is_some())
                        .collect();
                    assert_eq!(
                        joined.len(),
                        1,
                        "scale {scale}, {kind:?}, above {above}: {spans:?}"
                    );
                    assert_eq!(joined[0].start_anchor.as_ref().unwrap().note, 0);
                    assert_eq!(joined[0].end_anchor.as_ref().unwrap().note, 1);
                    for piece in score.span_segments(joined[0]) {
                        let staff = &score.staves[piece.staff];
                        assert!(support(&image, &piece, staff, -1.0) >= 0.85, "{piece:?}");
                        assert!(support(&image, &piece, staff, 1.0) >= 0.85, "{piece:?}");
                    }
                    score.spans = spans;
                    score.validate().unwrap();
                    score.staff_assignments = vec![
                        crate::model::StaffAssignment {
                            staff: 0,
                            system: 0,
                            part: 0,
                            name: "A".into(),
                        },
                        crate::model::StaffAssignment {
                            staff: 1,
                            system: 0,
                            part: 1,
                            name: "B".into(),
                        },
                    ];
                    assert!(
                        recognize(&image, &score)
                            .iter()
                            .all(|s| s.continuation.is_none()),
                        "different parts cannot be joined"
                    );
                }
            }
        }
    }

    #[test]
    fn separate_complete_wedges_and_parallel_strokes_are_not_joined() {
        let (mut image, mut score) = fixture(1.0);
        image = GrayImage::from_pixel(image.width(), 700, Luma([255]));
        score.notes.clear();
        let mut next = score.staves[0].clone();
        next.lines = next.lines.map(|y| y + 300.0);
        score.staves.push(next);
        for (x, y) in [(500.0, 150.0), (180.0, 450.0)] {
            score.add_note(x, y).unwrap();
        }
        let exit = Span {
            staff: 0,
            kind: SpanKind::Crescendo,
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            start: [500.0, 240.0],
            end: [615.0, 240.0],
            height: 10.0,
            confirmed: false,
        };
        let entry = Span {
            staff: 1,
            start: [25.0, 540.0],
            end: [180.0, 540.0],
            ..exit.clone()
        };
        draw(&mut image, &exit);
        draw(&mut image, &entry);
        assert!(
            recognize(&image, &score)
                .iter()
                .all(|s| s.continuation.is_none())
        );
        let (mut image, score) = fixture(1.0);
        let parallel = Span {
            wedge_openings: Some([1.0, 1.0]),
            start: [150.0, 230.0],
            end: [300.0, 230.0],
            ..exit
        };
        draw(&mut image, &parallel);
        assert!(open_fragments(&image, &score.staves[0], 0).is_empty());
    }
}
