use super::*;

/// Recover isolated printed curves from their actual endpoints, rather than
/// forcing them to begin at the horizontal centers of the noteheads.
pub(super) fn curves(
    image: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    bars: &[crate::model::Barline],
) -> Vec<Span> {
    let gap = staff.spacing();
    let scale = 16.0 / gap;
    let left = staff.left.max(0.0) as u32;
    let top = (staff.lines[0] - gap * 5.0).max(0.0) as u32;
    let right = staff.right.min(image.width() as f32) as u32;
    let bottom = (staff.lines[4] + gap * 5.0).min(image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return vec![];
    }
    let mut local = imageops::resize(
        &imageops::crop_imm(image, left, top, right - left, bottom - top).to_image(),
        ((right - left) as f32 * scale).round().max(1.0) as u32,
        ((bottom - top) as f32 * scale).round().max(1.0) as u32,
        imageops::FilterType::Triangle,
    );
    for (x, y, pixel) in local.enumerate_pixels_mut() {
        let row = top as f32 + y as f32 / scale;
        let column = left as f32 + x as f32 / scale;
        let line = staff
            .lines
            .iter()
            .any(|line| (row - line).abs() < gap * 0.12);
        let crossing = ink(image, column, row - gap * 0.22) && ink(image, column, row + gap * 0.22);
        pixel[0] = if pixel[0] < 180 && (!line || crossing) {
            0
        } else {
            255
        };
    }
    let labels = connected_components(&local, Connectivity::Eight, Luma([255]));
    let maximum = labels.pixels().map(|p| p[0]).max().unwrap_or(0) as usize;
    let mut components = vec![(u32::MAX, u32::MAX, 0, 0); maximum + 1];
    for (x, y, p) in labels.enumerate_pixels() {
        if p[0] == 0 {
            continue;
        }
        let bounds = &mut components[p[0] as usize];
        bounds.0 = bounds.0.min(x);
        bounds.1 = bounds.1.min(y);
        bounds.2 = bounds.2.max(x);
        bounds.3 = bounds.3.max(y);
    }
    let mut curves = vec![];
    for (label, &(x0, y0, x1, y1)) in components.iter().enumerate().skip(1) {
        if x0 == u32::MAX {
            continue;
        }
        let width = (x1 - x0) as f32;
        if !(0.7 * 16.0..=80.0 * 16.0).contains(&width)
            || y1 - y0 < 2
            || (y1 - y0) as f32 > 16.0 * 6.0
        {
            continue;
        }
        let mut points = vec![];
        let mut thick = 0;
        for x in x0..=x1 {
            let rows: Vec<_> = (y0..=y1)
                .filter(|&y| labels.get_pixel(x, y)[0] as usize == label)
                .collect();
            if rows.is_empty() {
                continue;
            }
            // A bar crossing a curve joins its connected component. Exclude
            // that vertical stroke from the fit, preserving the curve on both sides.
            if rows.last().unwrap() - rows[0] > 40
                && bars.iter().any(|bar| {
                    bar.staff == staff_index
                        && (left as f32 + x as f32 / scale - bar.x).abs() < gap * 0.8
                })
            {
                continue;
            }
            if rows.last().unwrap() - rows[0] > 6 {
                thick += 1;
            }
            points.push((
                (x - x0) as f32 / width,
                rows.iter().sum::<u32>() as f32 / rows.len() as f32,
            ));
        }
        if (points.len() as f32) < width * 0.88 || thick as f32 > width * 0.12 {
            continue;
        }
        let Some(coefficients) = quadratic(&points) else {
            continue;
        };
        let height = coefficients[2];
        if height.abs() < 16.0 * 0.14 || height.abs() > width * 0.6 {
            continue;
        }
        let residuals: Vec<_> = points
            .iter()
            .map(|&(t, y)| {
                (y - coefficients[0] - t * coefficients[1] - 4.0 * height * t * (1.0 - t)).abs()
            })
            .collect();
        if residuals.iter().map(|r| r * r).sum::<f32>() / residuals.len() as f32
            > (16.0_f32 * 0.12).powi(2)
            || residuals.iter().any(|&r| r > 16.0 * 0.3)
        {
            continue;
        }
        let span = Span {
            staff: staff_index,
            kind: SpanKind::Slur,
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            start: [
                left as f32 + x0 as f32 / scale,
                top as f32 + coefficients[0] / scale,
            ],
            end: [
                left as f32 + x1 as f32 / scale,
                top as f32 + (coefficients[0] + coefficients[1]) / scale,
            ],
            height: height / scale,
            confirmed: false,
        };
        if continuous_bow(image, staff, &span) {
            curves.push(span);
        }
    }
    curves
}

fn quadratic(points: &[(f32, f32)]) -> Option<[f32; 3]> {
    let origin = points.first()?.1;
    let mut equations = [[0.0; 4]; 3];
    for &(t, y) in points {
        let basis = [1.0, t, 4.0 * t * (1.0 - t)];
        for axis in 0..3 {
            for column in 0..3 {
                equations[axis][column] += basis[axis] * basis[column];
            }
            equations[axis][3] += basis[axis] * (y - origin);
        }
    }
    for axis in 0..3 {
        let pivot = (axis..3).max_by(|&a, &b| {
            equations[a][axis]
                .abs()
                .total_cmp(&equations[b][axis].abs())
        })?;
        equations.swap(axis, pivot);
        let divisor = equations[axis][axis];
        if divisor.abs() < 0.0001 {
            return None;
        }
        for value in &mut equations[axis][axis..] {
            *value /= divisor;
        }
        let pivot = equations[axis];
        for (index, row) in equations.iter_mut().enumerate() {
            if index == axis {
                continue;
            }
            let factor = row[axis];
            for (value, pivot) in row[axis..].iter_mut().zip(&pivot[axis..]) {
                *value -= factor * pivot;
            }
        }
    }
    Some([equations[0][3] + origin, equations[1][3], equations[2][3]])
}

pub(super) fn endpoint_note(score: &Score, span: &Span, right: bool) -> Option<usize> {
    let point = if right { span.end } else { span.start };
    let gap = score.staves[span.staff].spacing();
    score
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.staff == span.staff)
        .filter_map(|(index, note)| {
            let dx = (point[0] - note.x) / gap * if right { -1.0 } else { 1.0 };
            let dy = (point[1] - note.y) / gap * span.height.signum();
            if !(-0.3..=1.6).contains(&dx) || !(0.1..=1.9).contains(&dy) {
                return None;
            }
            Some((index, dx.abs() + (dy - 0.5).abs() * 0.6))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}
