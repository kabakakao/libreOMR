use image::{GrayImage, Luma};
use imageproc::contrast::{ThresholdType, threshold};
use std::collections::HashMap;

use crate::model::{
    Articulation, Barline, BarlineKind, Clef, Duration, Note, Rest, Score, Staff, StaffHeader,
    SymbolBounds,
};

pub fn straighten(image: GrayImage) -> GrayImage {
    if image.width() < 80 || image.height() < 80 {
        return image;
    }
    let sample = if image.width() > 1200 {
        image::imageops::resize(
            &image,
            1200,
            (image.height() as f32 * 1200.0 / image.width() as f32).round() as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image.clone()
    };
    let binary = threshold(&sample, 170, ThresholdType::Binary);
    let center = (sample.width() - 1) as f32 * 0.5;
    let ink: Vec<_> = binary
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[0] == 0)
        .map(|(column, row, _)| (column as f32 - center, row as f32))
        .collect();
    let projection_score = |slope: f32| {
        let mut rows = vec![0_u32; sample.height() as usize];
        for &(column, row) in &ink {
            let row = (row - slope * column).round() as i32;
            if row >= 0 && (row as usize) < rows.len() {
                rows[row as usize] += 1;
            }
        }
        rows.into_iter()
            .map(|count| u64::from(count).pow(2))
            .sum::<u64>()
    };
    let original_score = projection_score(0.0);
    let mut best_score = original_score;
    let mut best_slope = 0.0_f32;
    for step in -40..=40 {
        let slope = step as f32 * 0.0005;
        let score = projection_score(slope);
        if score > best_score {
            best_score = score;
            best_slope = slope;
        }
    }
    let coarse_slope = best_slope;
    for step in -10..=10 {
        let slope = coarse_slope + step as f32 * 0.00005;
        let score = projection_score(slope);
        if score > best_score {
            best_score = score;
            best_slope = slope;
        }
    }
    if best_slope.abs() < 0.0001 || (best_score as f64) < original_score as f64 * 1.15 {
        return image;
    }
    let baseline = detect_staves(&threshold(&image, 170, ThresholdType::Binary)).len();
    let aligned = imageproc::geometric_transformations::rotate_about_center(
        &image,
        -best_slope.atan(),
        imageproc::geometric_transformations::Interpolation::Bilinear,
        Luma([255]),
    );
    if detect_staves(&threshold(&aligned, 170, ThresholdType::Binary)).len() > baseline {
        aligned
    } else {
        image
    }
}

/// A completed pipeline step count, with the current staff when applicable.
/// Steps differ in cost; this describes work completed, not time remaining.
#[derive(Clone, Copy, Debug)]
pub struct RecognitionProgress {
    pub stage: &'static str,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub staff: Option<(usize, usize)>,
}

impl RecognitionProgress {
    pub fn fraction(self) -> f32 {
        let staff_fraction = self.staff.map_or(0.0, |(current, total)| {
            current.saturating_sub(1) as f32 / total.max(1) as f32
        });
        ((self.completed_steps as f32 + staff_fraction) / self.total_steps as f32).clamp(0.0, 1.0)
    }
}

pub fn recognize(image: &GrayImage) -> Score {
    recognize_with_progress(image, |_| {})
}

/// Recognize a page, reporting progress synchronously before each pipeline stage.
pub fn recognize_with_progress(
    image: &GrayImage,
    mut on_progress: impl FnMut(RecognitionProgress),
) -> Score {
    let mut report = |completed_steps, stage, staff| {
        on_progress(RecognitionProgress {
            stage,
            completed_steps,
            total_steps: 12,
            staff,
        });
    };
    report(0, "Prepare score image", None);
    let binary = threshold(image, 170, ThresholdType::Binary);
    report(1, "Detect staves", None);
    let mut staves = detect_staves(&binary);
    let staff_count = staves.len();
    let mut notes = Vec::new();
    let mut rests = Vec::new();
    report(2, "Detect clefs, notes and rests", None);
    for (staff_index, staff) in staves.iter_mut().enumerate() {
        report(
            2,
            "Detect clefs, notes and rests",
            Some((staff_index + 1, staff_count)),
        );
        staff.header = crate::symbols::recognize_header(&binary, staff);
        let staff_rests = crate::symbols::recognize_rests(&binary, staff, staff_index);
        notes.extend(detect_notes(&binary, staff, staff_index, &staff_rests));
        rests.extend(staff_rests);
    }
    report(3, "Detect barlines", None);
    let mut barlines: Vec<_> = staves
        .iter()
        .enumerate()
        .flat_map(|(index, staff)| {
            report(3, "Detect barlines", Some((index + 1, staff_count)));
            detect_barlines(&binary, staff, index, &notes)
        })
        .collect();
    report(4, "Detect key changes", None);
    let key_changes: Vec<_> = staves
        .iter()
        .enumerate()
        .flat_map(|(index, staff)| {
            report(4, "Detect key changes", Some((index + 1, staff_count)));
            crate::symbols::recognize_key_changes(&binary, staff, index, &barlines)
        })
        .collect();
    report(5, "Detect time changes", None);
    let time_changes: Vec<_> = staves
        .iter()
        .enumerate()
        .flat_map(|(index, staff)| {
            report(5, "Detect time changes", Some((index + 1, staff_count)));
            // Meter digits can initially look like noteheads and hide the
            // adjacent boundary. Find meter anchors before that exclusion.
            let anchors = detect_barlines(&binary, staff, index, &[]);
            crate::symbols::recognize_time_changes(&binary, staff, index, &anchors)
        })
        .collect();
    report(6, "Check notes near key and time changes", None);
    for (index, staff) in staves.iter().enumerate() {
        report(
            6,
            "Check notes near key and time changes",
            Some((index + 1, staff_count)),
        );
        let bounds: Vec<_> = key_changes
            .iter()
            .filter(|change| change.staff == index)
            .flat_map(|change| change.bounds.iter().cloned())
            .chain(
                time_changes
                    .iter()
                    .filter(|change| change.staff == index)
                    .flat_map(|change| change.bounds.iter().cloned()),
            )
            .collect();
        if bounds.is_empty() {
            continue;
        }
        let mut local = staff.clone();
        local.header.key_bounds.extend(bounds);
        let local_rests = crate::symbols::recognize_rests(&binary, &local, index);
        rests.retain(|rest| rest.staff != index);
        rests.extend(local_rests.iter().cloned());
        notes.retain(|note| note.staff != index);
        notes.extend(detect_notes(&binary, &local, index, &local_rests));
        barlines.retain(|bar| bar.staff != index);
        barlines.extend(detect_barlines(&binary, &local, index, &notes));
    }
    let mut score = Score {
        staves,
        notes,
        rests,
        barlines,
        key_changes,
        time_changes,
        ..Score::default()
    };
    report(7, "Detect repeat marks and endings", None);
    score.repeat_marks = crate::symbols::recognize_repeat_marks(&binary, &score);
    report(8, "Detect dynamic markings", None);
    score.dynamics = crate::symbols::recognize_dynamics(&binary, &score);
    // Navigation signs, ending labels and dynamic loops must not become noteheads.
    report(9, "Match notes and rests with symbols", None);
    for (index, staff) in score.staves.iter().enumerate() {
        report(
            9,
            "Match notes and rests with symbols",
            Some((index + 1, staff_count)),
        );
        let bounds: Vec<_> = score
            .dynamics
            .iter()
            .filter(|dynamic| dynamic.staff == index)
            .filter_map(|dynamic| dynamic.bounds.clone())
            .chain(
                score
                    .repeat_marks
                    .iter()
                    .filter(|mark| mark.staff == index)
                    .filter_map(|mark| mark.bounds.clone()),
            )
            .collect();
        if bounds.is_empty() {
            continue;
        }
        let mut local = staff.clone();
        local.header.key_bounds.extend(bounds);
        local.header.key_bounds.extend(
            score
                .key_changes
                .iter()
                .filter(|change| change.staff == index)
                .flat_map(|change| change.bounds.clone()),
        );
        local.header.key_bounds.extend(
            score
                .time_changes
                .iter()
                .filter(|change| change.staff == index)
                .flat_map(|change| change.bounds.clone()),
        );
        let rests = crate::symbols::recognize_rests(&binary, &local, index);
        score.notes.retain(|note| note.staff != index);
        score
            .notes
            .extend(detect_notes(&binary, &local, index, &rests));
        score.rests.retain(|rest| rest.staff != index);
        score.rests.extend(rests);
        score.barlines.retain(|bar| bar.staff != index);
        score
            .barlines
            .extend(detect_barlines(&binary, &local, index, &score.notes));
    }
    report(10, "Assign accidentals and articulations", None);
    // Reanalysis appends changed rows; retain document order for navigation,
    // exports, and callers iterating the stored objects directly.
    score
        .notes
        .sort_by(|a, b| a.staff.cmp(&b.staff).then(a.x.total_cmp(&b.x)));
    score
        .rests
        .sort_by(|a, b| a.staff.cmp(&b.staff).then(a.x.total_cmp(&b.x)));
    score
        .barlines
        .sort_by(|a, b| a.staff.cmp(&b.staff).then(a.x.total_cmp(&b.x)));
    score.notes.retain(|note| {
        !is_detached_open_head(&binary, &score.staves[note.staff], note)
            || !score.barlines.iter().any(|bar| {
                bar.staff == note.staff
                    && (bar.x - note.x).abs() < score.staves[note.staff].spacing() * 0.85
            })
    });
    score.recompute_accidentals();
    crate::symbols::recognize_articulations(&binary, &mut score);
    report(11, "Detect slurs, ties and dynamic wedges", None);
    score.spans = crate::spans::recognize(&binary, &score);
    report(12, "Recognition complete", None);
    score
}

fn is_detached_open_head(binary: &GrayImage, staff: &Staff, note: &Note) -> bool {
    let gap = staff.spacing();
    note.duration == Duration::Half
        && (note.y < staff.lines[0] - gap * 0.6 || note.y > staff.lines[4] + gap * 0.6)
        && find_stem_with_tolerance(binary, note.x, note.y, gap, 0.06).is_none()
}

fn detect_barlines(
    binary: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    notes: &[Note],
) -> Vec<Barline> {
    let gap = staff.spacing();
    let top = staff.lines[0].round() as i32;
    let bottom = staff.lines[4].round() as i32;
    let mut strokes: Vec<(i32, i32)> = Vec::new();
    // The vertical line before an opening clef closes the staff, not a measure.
    let left = staff
        .header
        .clef_bounds
        .as_ref()
        .map_or(staff.left, |bounds| bounds.right.max(staff.left));
    for column in left.ceil() as i32..=staff.right.floor() as i32 {
        let covered = (top..=bottom)
            .filter(|&row| dark(binary, column, row))
            .count();
        if covered as f32 / ((bottom - top + 1) as f32) < 0.95 {
            continue;
        }
        if notes.iter().any(|note| {
            note.staff == staff_index
                && (note.x - column as f32).abs() < gap * 0.85
                && !is_detached_open_head(binary, staff, note)
        }) || [&staff.header.clef_bounds, &staff.header.time_bounds]
            .into_iter()
            .flatten()
            .chain(staff.header.key_bounds.iter())
            .any(|bounds| column as f32 >= bounds.left && column as f32 <= bounds.right)
        {
            continue;
        }
        if let Some(last) = strokes.last_mut()
            && last.1 + 1 == column
        {
            last.1 = column;
        } else {
            strokes.push((column, column));
        }
    }
    let mut groups: Vec<Vec<(i32, i32)>> = Vec::new();
    for (left, right) in strokes {
        if (right - left + 1) as f32 > gap * 0.65 {
            continue;
        }
        if let Some(last) = groups.last_mut()
            && (left - last.last().unwrap().1) as f32 <= gap * 0.8
        {
            last.push((left, right));
        } else {
            groups.push(vec![(left, right)]);
        }
    }
    groups
        .into_iter()
        .map(|group| {
            let first = group[0];
            let last = *group.last().unwrap();
            let dots_left = has_repeat_dots(binary, staff, first.0 as f32, -1.0);
            let dots_right = has_repeat_dots(binary, staff, last.1 as f32, 1.0);
            let first_width = (first.1 - first.0 + 1) as f32;
            let last_width = (last.1 - last.0 + 1) as f32;
            let kind = match (dots_left, dots_right) {
                (true, true) => BarlineKind::RepeatBoth,
                (true, false) => BarlineKind::RepeatEnd,
                (false, true) => BarlineKind::RepeatStart,
                (false, false) if group.len() == 1 => BarlineKind::Single,
                (false, false)
                    if last_width >= first_width * 1.6
                        && last_width - first_width >= gap * 0.08 =>
                {
                    BarlineKind::Final
                }
                _ => BarlineKind::Double,
            };
            Barline {
                staff: staff_index,
                x: (first.0 + last.1) as f32 / 2.0,
                kind,
                confirmed: false,
            }
        })
        .collect()
}

fn has_repeat_dots(binary: &GrayImage, staff: &Staff, edge: f32, direction: f32) -> bool {
    let gap = staff.spacing();
    let near = edge + direction * gap * 0.1;
    let far = edge + direction * gap * 1.7;
    let left = near.min(far).max(0.0) as u32;
    let right = near.max(far).min(binary.width() as f32).max(0.0) as u32;
    if right <= left {
        return false;
    }
    let mut dots = Vec::new();
    for position in [1.5, 2.5] {
        let center_y = staff.lines[0] + gap * position;
        let top = (center_y - gap * 0.45).max(0.0) as u32;
        let bottom = (center_y + gap * 0.45).min(binary.height() as f32).max(0.0) as u32;
        if bottom <= top {
            return false;
        }
        let crop =
            image::imageops::crop_imm(binary, left, top, right - left, bottom - top).to_image();
        let labels = imageproc::region_labelling::connected_components(
            &crop,
            imageproc::region_labelling::Connectivity::Eight,
            Luma([255]),
        );
        let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
        let mut bounds = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32); maximum + 1];
        for (column, row, label) in labels.enumerate_pixels() {
            if label[0] == 0 {
                continue;
            }
            let bound = &mut bounds[label[0] as usize];
            bound.0 = bound.0.min(column);
            bound.1 = bound.1.min(row);
            bound.2 = bound.2.max(column);
            bound.3 = bound.3.max(row);
            bound.4 += 1;
        }
        let mut centers = Vec::new();
        for (min_x, min_y, max_x, max_y, count) in bounds.into_iter().skip(1) {
            let width = (max_x - min_x + 1) as f32;
            let height = (max_y - min_y + 1) as f32;
            if min_x == 0
                || min_y == 0
                || max_x + 1 == crop.width()
                || max_y + 1 == crop.height()
                || !(0.12..=0.65).contains(&(width / gap))
                || !(0.12..=0.65).contains(&(height / gap))
                || !(0.65..=1.55).contains(&(width / height))
                || !(0.015..=0.3).contains(&(count as f32 / (gap * gap)))
                || (top as f32 + (min_y + max_y) as f32 / 2.0 - center_y).abs() > gap * 0.2
            {
                continue;
            }
            centers.push(left as f32 + (min_x + max_x) as f32 / 2.0);
        }
        dots.push(centers);
    }
    dots[0].iter().any(|upper| {
        dots[1]
            .iter()
            .any(|lower| (upper - lower).abs() <= gap * 0.2)
    })
}

fn detect_staves(binary: &GrayImage) -> Vec<Staff> {
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for y in 0..binary.height() {
        let dark = (0..binary.width())
            .filter(|&x| binary.get_pixel(x, y)[0] == 0)
            .count();
        if dark as f32 > binary.width() as f32 * 0.45 {
            if let Some(last) = bands.last_mut()
                && last.1 + 1 == y
            {
                last.1 = y;
                continue;
            }
            bands.push((y, y));
        }
    }
    let centers: Vec<f32> = bands
        .iter()
        .map(|&(top, bottom)| (top + bottom) as f32 / 2.0)
        .collect();
    let mut staves = Vec::new();
    let mut index = 0;
    while index + 5 <= centers.len() {
        let lines: [f32; 5] = centers[index..index + 5].try_into().unwrap();
        let gap = (lines[4] - lines[0]) / 4.0;
        if gap >= 4.0
            && lines
                .windows(2)
                .all(|pair| ((pair[1] - pair[0]) - gap).abs() <= gap * 0.2)
        {
            // Follow slightly curved or slanted staff ends instead of using
            // one row, which can cut off a courtesy meter at the right edge.
            let on_staff = |x: u32| {
                lines
                    .iter()
                    .filter(|&&line| {
                        ((line - gap * 0.35).max(0.0) as u32
                            ..=((line + gap * 0.35) as u32).min(binary.height() - 1))
                            .any(|y| binary.get_pixel(x, y)[0] == 0)
                    })
                    .count()
                    >= 4
            };
            let middle = lines[2] as u32;
            let left = (0..binary.width())
                .find(|&x| binary.get_pixel(x, middle)[0] == 0)
                .unwrap_or(0);
            let right = (0..binary.width())
                .rev()
                .find(|&x| on_staff(x))
                .unwrap_or(binary.width().saturating_sub(1));
            staves.push(Staff {
                lines,
                left: left as f32,
                right: right as f32,
                header: StaffHeader::default(),
            });
            index += 5;
        } else {
            index += 1;
        }
    }
    staves
}

fn detect_notes(
    binary: &GrayImage,
    staff: &Staff,
    staff_index: usize,
    rests: &[Rest],
) -> Vec<Note> {
    let gap = staff.spacing();
    let mut notes = Vec::new();
    let mut heads = crate::symbols::recognize_noteheads(binary, staff);
    let mut flags = HashMap::new();
    // Beam/flag ends can resemble heads with a stem on either side. Keep
    // these geometric checks unoriented; attachment preference is for the
    // surviving heads when determining their actual note values.
    let filled_stems: Vec<_> = heads
        .iter()
        .filter(|head| head.kind == crate::symbols::HeadKind::Filled)
        .filter_map(|head| {
            find_stem_with_tolerance(binary, head.x, head.y, gap, 0.18).map(|stem| (head.y, stem))
        })
        .collect();
    remove_open_stem_artifacts(&mut heads, &filled_stems, gap);
    heads.retain(|head| {
        if head.kind == crate::symbols::HeadKind::Filled
            && find_stem_with_tolerance(binary, head.x, head.y, gap, 0.18).is_some_and(|stem| {
                filled_stems.iter().any(|(filled_y, other)| {
                    stem.direction == -other.direction
                        && ((stem.x - other.x).abs() as f32) < gap * 0.35
                        && (stem.tip as f32 - filled_y).abs() < gap * 0.7
                        && (head.y - filled_y).abs() > gap * 1.5
                        && (((head.y - other.tip as f32).abs() < gap * 0.6
                            && has_beam_extension(binary, *other, gap))
                            || ((head.y - other.tip as f32).abs() < gap * 0.9
                                && cached_flags(binary, staff, *other, &mut flags) > 0))
                })
            })
        {
            return false;
        }
        head.kind != crate::symbols::HeadKind::Open
            || !find_stem_with_tolerance(binary, head.x, head.y, gap, 0.18).is_some_and(|stem| {
                filled_stems.iter().any(|(_, other)| {
                    stem.direction == other.direction
                        && ((stem.x - other.x).abs() as f32) < gap * 0.3
                        && ((stem.tip - other.tip).abs() as f32) < gap * 0.5
                })
            })
    });
    let accidentals: Vec<_> = heads
        .iter()
        .map(|head| crate::symbols::recognize_accidental(binary, staff, head.x, head.y))
        .collect();
    let accidental_bounds: Vec<_> = accidentals
        .iter()
        .filter_map(|accidental| accidental.as_ref().map(|(_, bounds)| bounds.clone()))
        .collect();
    for (head, accidental) in heads.iter().zip(&accidentals) {
        let (x, y) = (head.x, head.y);
        if accidental_bounds.iter().any(|bounds| bounds.contains(x, y)) {
            continue;
        }
        if !has_required_ledger(binary, staff, x, y) {
            continue;
        }
        if [&staff.header.clef_bounds, &staff.header.time_bounds]
            .into_iter()
            .flatten()
            .chain(staff.header.key_bounds.iter())
            .any(|bounds| {
                x >= bounds.left - gap * 0.3
                    && x <= bounds.right + gap * 0.3
                    && y >= bounds.top - gap * 0.3
                    && y <= bounds.bottom + gap * 0.3
            })
            || rests
                .iter()
                .filter_map(|rest| rest.bounds.as_ref())
                .any(|bounds| bounds.contains(x, y))
        {
            continue;
        }
        let pitch = staff.pitch_at(y);
        if !(0..=69).contains(&pitch) {
            continue;
        }
        let stem = find_stem(binary, x, y, gap);
        if head.kind == crate::symbols::HeadKind::Open
            && stem.is_some_and(|stem| {
                heads.iter().any(|other| {
                    other.kind == crate::symbols::HeadKind::Filled
                        && (other.x - stem.x as f32).abs() < gap
                        && (other.y - stem.tip as f32).abs() < gap * 0.7
                })
            })
        {
            continue;
        }
        let duration = match head.kind {
            crate::symbols::HeadKind::Whole if stem.is_none() => Duration::Whole,
            crate::symbols::HeadKind::Open | crate::symbols::HeadKind::Whole if stem.is_some() => {
                Duration::Half
            }
            crate::symbols::HeadKind::Filled if stem.is_some() => {
                let stem = stem.unwrap();
                let beams = count_beams(binary, staff, stem, y, &heads, &accidental_bounds);
                // A clearly connected beam replaces flags. Searching flags
                // inside it can mistake chord/arpeggio strokes for extra hooks.
                let flags = if beams < 4 && (beams == 0 || !has_beam_extension(binary, stem, gap)) {
                    cached_flags(binary, staff, stem, &mut flags)
                } else {
                    0
                };
                match beams.max(flags) {
                    0 => Duration::Quarter,
                    1 => Duration::Eighth,
                    2 => Duration::Sixteenth,
                    3 => Duration::ThirtySecond,
                    _ => Duration::SixtyFourth,
                }
            }
            _ => continue,
        };
        notes.push(Note {
            x,
            y,
            staff: staff_index,
            pitch,
            accidental: accidental
                .as_ref()
                .map(|(value, _)| *value)
                .unwrap_or_else(|| staff.accidental_at(pitch)),
            accidental_explicit: accidental.is_some(),
            duration,
            dots: detect_dots(binary, staff, x, y),
            triplet: false,
            tuplet: None,
            grace: false,
            small: false,
            articulation: Articulation::None,
            confirmed: false,
        });
    }
    notes.sort_by(|first, second| first.x.total_cmp(&second.x));
    assign_triplets(binary, staff, rests, &mut notes);
    notes
}

fn remove_open_stem_artifacts(
    heads: &mut Vec<crate::symbols::Notehead>,
    filled_stems: &[(f32, Stem)],
    gap: f32,
) {
    // A short secondary beam can enclose a loop with a staff line. Reject
    // that open-head candidate before it can mask the beam during counting.
    heads.retain(|head| {
        head.kind != crate::symbols::HeadKind::Open
            || !filled_stems.iter().any(|(filled_y, stem)| {
                (head.x - stem.x as f32).abs() < gap * 0.9
                    && (head.y - filled_y) * stem.direction as f32 > gap * 0.8
                    && (stem.tip as f32 - head.y) * stem.direction as f32 > gap * 0.25
            })
    });
}

fn assign_triplets(binary: &GrayImage, staff: &Staff, rests: &[Rest], notes: &mut [Note]) {
    let gap = staff.spacing();
    for start in 0..notes.len().saturating_sub(2) {
        let group = &notes[start..start + 3];
        if group[0].duration.flag_count() == 0
            || group
                .iter()
                .any(|note| note.triplet || note.dots != 0 || note.duration != group[0].duration)
            || rests
                .iter()
                .any(|rest| rest.x > group[0].x && rest.x < group[2].x)
        {
            continue;
        }
        let first_gap = group[1].x - group[0].x;
        let second_gap = group[2].x - group[1].x;
        if !(0.9..=8.0).contains(&(first_gap / gap))
            || !(0.9..=8.0).contains(&(second_gap / gap))
            || !(0.5..=2.0).contains(&(first_gap / second_gap))
        {
            continue;
        }
        let stems: Vec<_> = group
            .iter()
            .filter_map(|note| find_stem(binary, note.x, note.y, gap))
            .collect();
        if stems.len() != 3
            || stems
                .iter()
                .any(|stem| stem.direction != stems[0].direction)
        {
            continue;
        }
        let first = stems[0];
        let last = stems[2];
        let connected = (1..10)
            .filter(|sample| {
                let fraction = *sample as f32 / 10.0;
                let x = first.x as f32 + (last.x - first.x) as f32 * fraction;
                let y = first.tip as f32 + (last.tip - first.tip) as f32 * fraction
                    - first.direction as f32 * gap * 0.15;
                (-(gap * 0.2).ceil() as i32..=(gap * 0.2).ceil() as i32)
                    .any(|offset| dark(binary, x.round() as i32, y.round() as i32 + offset))
            })
            .count();
        if connected < 8 {
            continue;
        }
        let center_x = (group[0].x + group[2].x) / 2.0;
        let center_y = stems.iter().map(|stem| stem.tip as f32).sum::<f32>() / 3.0
            + first.direction as f32 * gap * 0.75;
        if crate::symbols::has_triplet_marker(binary, staff, center_x, center_y) {
            for note in &mut notes[start..start + 3] {
                note.triplet = true;
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Stem {
    x: i32,
    tip: i32,
    direction: i32,
}

fn cached_flags(
    binary: &GrayImage,
    staff: &Staff,
    stem: Stem,
    cache: &mut HashMap<(i32, i32, i32), usize>,
) -> usize {
    *cache
        .entry((stem.x, stem.tip, stem.direction))
        .or_insert_with(|| {
            crate::symbols::recognize_flags(binary, staff, stem.x, stem.tip, stem.direction)
        })
}

fn detect_dots(image: &GrayImage, staff: &Staff, x: f32, y: f32) -> u8 {
    let gap = staff.spacing();
    let snapped = staff.y_at_pitch(staff.pitch_at(y));
    let relative = (snapped - staff.lines[0]) / gap;
    let dot_y = if (relative - relative.round()).abs() < 0.25 {
        snapped - gap * 0.5
    } else {
        snapped
    };
    let left = (x + gap * 0.6).clamp(0.0, image.width() as f32) as u32;
    let right = (x + gap * 2.6).clamp(0.0, image.width() as f32) as u32;
    let top = (dot_y - gap * 0.35).clamp(0.0, image.height() as f32) as u32;
    let bottom = (dot_y + gap * 0.35).clamp(0.0, image.height() as f32) as u32;
    if right <= left || bottom <= top {
        return 0;
    }
    let crop = image::imageops::crop_imm(image, left, top, right - left, bottom - top).to_image();
    let labels = imageproc::region_labelling::connected_components(
        &crop,
        imageproc::region_labelling::Connectivity::Eight,
        Luma([255]),
    );
    let maximum = labels.pixels().map(|label| label[0]).max().unwrap_or(0) as usize;
    let mut bounds = vec![(u32::MAX, u32::MAX, 0, 0, 0_u32); maximum + 1];
    for (column, row, label) in labels.enumerate_pixels() {
        if label[0] == 0 {
            continue;
        }
        let bound = &mut bounds[label[0] as usize];
        bound.0 = bound.0.min(column);
        bound.1 = bound.1.min(row);
        bound.2 = bound.2.max(column);
        bound.3 = bound.3.max(row);
        bound.4 += 1;
    }
    let mut centers = Vec::new();
    for (min_x, min_y, max_x, max_y, count) in bounds.into_iter().skip(1) {
        let width = (max_x - min_x + 1) as f32;
        let height = (max_y - min_y + 1) as f32;
        if min_x == 0
            || min_y == 0
            || max_x + 1 == crop.width()
            || max_y + 1 == crop.height()
            || !(0.12..=0.5).contains(&(width / gap))
            || !(0.12..=0.5).contains(&(height / gap))
            || !(0.02..=0.2).contains(&(count as f32 / (gap * gap)))
            || !(0.5..=2.0).contains(&(width / height))
        {
            continue;
        }
        let center_x = left as f32 + (min_x + max_x) as f32 / 2.0;
        let center_y = top as f32 + (min_y + max_y) as f32 / 2.0;
        if (center_y - dot_y).abs() <= gap * 0.22 {
            centers.push(center_x);
        }
    }
    centers.sort_by(f32::total_cmp);
    let Some(first) = centers
        .iter()
        .copied()
        .find(|center| (0.75..=1.9).contains(&((*center - x) / gap)))
    else {
        return 0;
    };
    if centers
        .iter()
        .any(|center| (0.3..=0.8).contains(&((*center - first) / gap)))
    {
        2
    } else {
        1
    }
}

fn has_required_ledger(image: &GrayImage, staff: &Staff, x: f32, y: f32) -> bool {
    let gap = staff.spacing();
    let relative = (y - staff.lines[0]) / gap;
    let ledger = if relative <= -0.75 {
        relative.ceil()
    } else if relative >= 4.75 {
        relative.floor()
    } else {
        return true;
    };
    let row = staff.lines[0] + ledger * gap;
    for offset in -(gap * 0.15).ceil() as i32..=(gap * 0.15).ceil() as i32 {
        let row = row.round() as i32 + offset;
        let left = (x - gap * 0.85).round() as i32;
        let right = (x + gap * 0.85).round() as i32;
        let ink = (left..=right)
            .filter(|&column| dark(image, column, row))
            .count();
        if ink as f32 >= (right - left + 1) as f32 * 0.85 {
            return true;
        }
    }
    false
}

fn dark(image: &GrayImage, x: i32, y: i32) -> bool {
    x >= 0
        && y >= 0
        && x < image.width() as i32
        && y < image.height() as i32
        && image.get_pixel(x as u32, y as u32)[0] == 0
}

fn find_stem(image: &GrayImage, x: f32, y: f32, gap: f32) -> Option<Stem> {
    let (longest, attached) = trace_stems(image, x, y, gap, 0.18);
    attached.or(longest)
}

fn find_stem_with_tolerance(
    image: &GrayImage,
    x: f32,
    y: f32,
    gap: f32,
    blank_gap: f32,
) -> Option<Stem> {
    trace_stems(image, x, y, gap, blank_gap).0
}

fn trace_stems(
    image: &GrayImage,
    x: f32,
    y: f32,
    gap: f32,
    blank_gap: f32,
) -> (Option<Stem>, Option<Stem>) {
    let mut best = None;
    let mut longest = gap * 1.7;
    let mut attached = None;
    let mut attached_length = gap * 1.7;
    for column in (x - gap * 0.9).round() as i32..=(x + gap * 0.9).round() as i32 {
        if ((column as f32 - x) / gap).abs() < 0.3 {
            continue;
        }
        for direction in [-1, 1] {
            let mut blank = 0;
            let mut tip = y.round() as i32;
            let start = y.round() as i32 + direction * (gap * 0.4).round() as i32;
            // Upper heads in wide chords can be much farther from the common
            // beam than a single note. Stop at whitespace, not at five spaces.
            for distance in 0..(gap * 9.0).ceil() as i32 {
                let row = start + direction * distance;
                if dark(image, column, row) {
                    tip = row;
                    blank = 0;
                } else {
                    blank += 1;
                }
                if blank as f32 > gap * blank_gap {
                    break;
                }
            }
            let length = (tip as f32 - y).abs();
            if length > longest {
                longest = length;
                best = Some(Stem {
                    x: column,
                    tip,
                    direction,
                });
            }
            // On an ordinary head the stem rises at the right edge or falls
            // at the left edge. In a wide chord the continuation towards the
            // other heads can be longer than the part leading to the beam.
            // Prefer the attached direction rather than reversing that stem.
            // Keep the other direction as a fallback for displaced heads.
            let attachment_direction = if column as f32 > x { -1 } else { 1 };
            if direction == attachment_direction && length > attached_length {
                attached_length = length;
                attached = Some(Stem {
                    x: column,
                    tip,
                    direction,
                });
            }
        }
    }
    (best, attached)
}

fn has_beam_extension(image: &GrayImage, stem: Stem, gap: f32) -> bool {
    [-1.0, 1.0].iter().any(|side| {
        [-0.5, -0.25, 0.0, 0.25, 0.5].iter().any(|slope| {
            [-0.25, 0.0, 0.25, 0.5].iter().any(|offset| {
                (0..8)
                    .filter(|sample| {
                        let distance = gap * (0.5 + *sample as f32 * 0.25);
                        let column = (stem.x as f32 + side * distance).round() as i32;
                        let row = stem.tip as f32 + offset * gap + slope * distance;
                        [-0.12, 0.0, 0.12].iter().all(|thickness| {
                            dark(image, column, (row + thickness * gap).round() as i32)
                        })
                    })
                    .count()
                    >= 7
            })
        })
    })
}

fn count_beams(
    image: &GrayImage,
    staff: &Staff,
    stem: Stem,
    head_y: f32,
    heads: &[crate::symbols::Notehead],
    accidentals: &[SymbolBounds],
) -> usize {
    let gap = staff.spacing();
    let near_head = (head_y + stem.direction as f32 * gap * 0.9).round() as i32;
    let near_tip = stem.tip + stem.direction * (gap * 0.25).round() as i32;
    let mut maximum = 0;
    for side in [-1, 1] {
        for offset in [0.5, 0.8, 1.1] {
            let column = stem.x + side * (gap * offset).round() as i32;
            let mut count = 0;
            let mut length = 0;
            let mut blank = 0;
            let mut connected = false;
            for row in near_head.min(near_tip)..=near_head.max(near_tip) + (gap * 0.3).ceil() as i32
            {
                let relative = (row as f32 - staff.lines[0]) / gap;
                let mut thickness = 0;
                if dark(image, column, row) {
                    thickness = 1;
                    for direction in [-1, 1] {
                        for distance in 1..=(gap * 0.3).ceil() as i32 {
                            if !dark(image, column, row + direction * distance) {
                                break;
                            }
                            thickness += 1;
                        }
                    }
                }
                let on_line = (relative - relative.round()).abs() * gap <= (gap * 0.12).max(1.0)
                    && (thickness as f32) < gap * 0.28;
                let on_head = heads.iter().any(|head| {
                    (head.x - column as f32).abs() < gap * 0.85
                        && (head.y - row as f32).abs() < gap * 0.65
                });
                let on_accidental = accidentals
                    .iter()
                    .any(|bounds| bounds.contains(column as f32, row as f32));
                if !on_line && !on_head && !on_accidental && dark(image, column, row) {
                    length += 1;
                    blank = 0;
                    let ink = (stem.x.min(column)..=stem.x.max(column))
                        .filter(|&x| dark(image, x, row))
                        .count();
                    // At a staff-line crossing even an arpeggio/barline has
                    // a horizontal connection to the stem. A genuine beam
                    // must also connect away from those thin staff rows.
                    let on_staff_row = staff
                        .lines
                        .iter()
                        .any(|line| (*line - row as f32).abs() <= (gap * 0.12).max(1.0));
                    connected |= !on_staff_row
                        && ink as f32 >= (stem.x - column).unsigned_abs() as f32 * 0.7;
                } else {
                    blank += 1;
                }
                if blank as f32 > gap * 0.08 {
                    // A nearby barline/stem can connect across a staff line,
                    // but its long vertical run is not a horizontal beam.
                    if (gap * 0.12..=gap * 0.8).contains(&(length as f32)) && connected {
                        count += 1;
                    }
                    length = 0;
                    connected = false;
                }
            }
            if (gap * 0.12..=gap * 0.8).contains(&(length as f32)) && connected {
                count += 1;
            }
            maximum = maximum.max(count);
        }
    }
    maximum.min(4)
}

pub fn demo_image() -> GrayImage {
    let mut image = GrayImage::from_pixel(1000, 460, Luma([255]));
    for top in [100, 280] {
        for line in 0..5 {
            for x in 60..940 {
                image.put_pixel(x, top + line * 16, Luma([0]));
            }
        }
        crate::symbols::draw_demo_header(
            &mut image,
            top,
            if top == 100 { Clef::Treble } else { Clef::Bass },
        );
        for (index, offset) in [0, 1, 2, 4, 3, 5, 7, 8].iter().enumerate() {
            let x = 230 + index as i32 * 95;
            let y = top as i32 + 64 - offset * 8;
            imageproc::drawing::draw_filled_ellipse_mut(&mut image, (x, y), 10, 6, Luma([0]));
            for stem_y in (y - 42)..y {
                image.put_pixel((x + 9) as u32, stem_y as u32, Luma([0]));
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_secondary_beams_are_not_masked_by_open_head_candidates() {
        let mut image = GrayImage::from_pixel(320, 260, Luma([255]));
        let staff = Staff {
            lines: [100.0, 120.0, 140.0, 160.0, 180.0],
            left: 20.0,
            right: 300.0,
            header: Default::default(),
        };
        for &line in &staff.lines {
            for x in 20..300 {
                image.put_pixel(x, line as u32, Luma([0]));
            }
        }
        for (x, y, tip) in [(100, 180, 90), (180, 140, 70), (180, 210, 70)] {
            imageproc::drawing::draw_filled_ellipse_mut(&mut image, (x, y), 12, 7, Luma([0]));
            for row in tip..=y {
                for column in x + 10..=x + 11 {
                    image.put_pixel(column as u32, row as u32, Luma([0]));
                }
            }
        }
        for x in 160..200 {
            image.put_pixel(x, 200, Luma([0]));
        }
        for x in 110..=191 {
            let top = 90 - (x - 110) / 4;
            for y in top..top + 9 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        for x in 171..=191 {
            let top = 93 - (x - 171) / 2;
            for y in top..top + 9 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        let mut heads: Vec<_> = [(100.0, 180.0), (180.0, 140.0), (180.0, 210.0)]
            .into_iter()
            .map(|(x, y)| crate::symbols::Notehead {
                x,
                y,
                kind: crate::symbols::HeadKind::Filled,
            })
            .collect();
        let filled_stems: Vec<_> = heads
            .iter()
            .map(|head| {
                (
                    head.y,
                    find_stem(&image, head.x, head.y, staff.spacing()).unwrap(),
                )
            })
            .collect();
        let stem = filled_stems[2].1;
        // A hole between the partial beam and the top staff line initially
        // looks like an open head. A separate real half-note candidate stays.
        heads.push(crate::symbols::Notehead {
            x: 184.0,
            y: 95.5,
            kind: crate::symbols::HeadKind::Open,
        });
        heads.push(crate::symbols::Notehead {
            x: 240.0,
            y: 180.0,
            kind: crate::symbols::HeadKind::Open,
        });
        assert_eq!(count_beams(&image, &staff, stem, 210.0, &heads, &[]), 1);
        remove_open_stem_artifacts(&mut heads, &filled_stems, staff.spacing());
        assert_eq!(heads.len(), 4);
        assert!(
            heads
                .iter()
                .any(|head| head.kind == crate::symbols::HeadKind::Open && head.x == 240.0)
        );
        for &head_y in &[140.0, 210.0] {
            assert_eq!(count_beams(&image, &staff, stem, head_y, &heads, &[]), 2);
        }
    }

    #[test]
    fn shared_stems_keep_the_same_direction_and_beams_for_every_chord_head() {
        for down in [false, true] {
            for beams in 0..=4 {
                for scale in [0.75, 1.0, 1.5] {
                    let mut image = GrayImage::from_pixel(360, 300, Luma([255]));
                    for line in [100, 120, 140, 160, 180] {
                        for x in 20..340 {
                            image.put_pixel(x, line, Luma([0]));
                        }
                    }
                    let tip = if down { 240 } else { 60 };
                    let stem_x = if down { 89 } else { 111 };
                    for y in [120, 180] {
                        imageproc::drawing::draw_filled_ellipse_mut(
                            &mut image,
                            (100, y),
                            12,
                            7,
                            Luma([0]),
                        );
                    }
                    for y in 120.min(tip)..=180.max(tip) {
                        for x in stem_x..=stem_x + 1 {
                            image.put_pixel(x, y, Luma([0]));
                        }
                    }
                    for beam in 0..beams {
                        let row = if down {
                            tip - beam * 12
                        } else {
                            tip + beam * 12
                        };
                        for y in row..row + 6 {
                            for x in stem_x..=260 {
                                image.put_pixel(x, y, Luma([0]));
                            }
                        }
                    }
                    let image = image::imageops::resize(
                        &image,
                        (360.0 * scale) as u32,
                        (300.0 * scale) as u32,
                        image::imageops::FilterType::Nearest,
                    );
                    let staff = Staff {
                        lines: [100.0, 120.0, 140.0, 160.0, 180.0].map(|y| y * scale),
                        left: 20.0 * scale,
                        right: 340.0 * scale,
                        header: Default::default(),
                    };
                    let heads: Vec<_> = [120.0, 180.0]
                        .into_iter()
                        .map(|y| crate::symbols::Notehead {
                            x: 100.0 * scale,
                            y: y * scale,
                            kind: crate::symbols::HeadKind::Filled,
                        })
                        .collect();
                    for head in &heads {
                        let stem = find_stem(&image, head.x, head.y, staff.spacing()).unwrap();
                        assert_eq!(
                            stem.direction,
                            if down { 1 } else { -1 },
                            "down {down}, beams {beams}, scale {scale}, head {}",
                            head.y
                        );
                        assert_eq!(
                            count_beams(&image, &staff, stem, head.y, &heads, &[]),
                            beams as usize,
                            "down {down}, beams {beams}, scale {scale}, head {}",
                            head.y
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn public_bach_partita_clefs_on_all_pages() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/public-domain/bach-bwv1013.pdf");
        let pages = crate::document_import::open_document(&path).unwrap();
        assert_eq!(pages.len(), 5);
        let mut failures = Vec::new();
        for (page, expected_staves) in pages.iter().zip([11, 12, 14, 14, 13]) {
            let image = page.render().unwrap();
            let binary = threshold(&image, 170, ThresholdType::Binary);
            let staves = detect_staves(&binary);
            assert_eq!(staves.len(), expected_staves, "{}", page.name());
            for (index, staff) in staves.iter().enumerate() {
                let header = crate::symbols::recognize_header(&binary, staff);
                if header.clef != Clef::Treble {
                    failures.push(format!(
                        "{}, staff {}: {staff:?} {header:?}",
                        page.name(),
                        index + 1
                    ));
                } else {
                    let bounds = header.clef_bounds.unwrap();
                    assert!(bounds.left < staff.left + staff.spacing() * 1.5);
                    assert!(bounds.right < staff.left + staff.spacing() * 4.0);
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn public_bach_partita_header_symbols_are_not_notes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/public-domain/bach-bwv1013.pdf");
        let pages = crate::document_import::open_document(&path).unwrap();
        let image = pages[0].render().unwrap();
        let binary = threshold(&image, 170, ThresholdType::Binary);
        let staff = &detect_staves(&binary)[0];
        let top = (staff.lines[0] - staff.spacing() * 2.5) as u32;
        let bottom = (staff.lines[4] + staff.spacing() * 3.0) as u32;
        let excerpt =
            image::imageops::crop_imm(&image, 0, top, image.width(), bottom - top).to_image();
        let score = recognize(&excerpt);
        assert_eq!(score.staves.len(), 1);
        let header = &score.staves[0].header;
        assert_eq!(header.clef, Clef::Treble);
        assert_eq!(
            header.time_signature,
            Some(crate::model::TimeSignature::Common)
        );
        for bounds in [&header.clef_bounds, &header.time_bounds]
            .into_iter()
            .flatten()
        {
            assert!(
                score
                    .notes
                    .iter()
                    .all(|note| !bounds.contains(note.x, note.y))
            );
            assert!(
                score
                    .rests
                    .iter()
                    .all(|rest| !bounds.contains(rest.x, rest.y))
            );
        }
        let first_note = score
            .notes
            .iter()
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        assert_eq!(first_note.pitch_label(), "E5");
        assert_eq!(first_note.duration, Duration::Sixteenth);
    }

    #[test]
    #[ignore = "requires LIBREOMR_KULTTOUR_TEST_PDF pointing to the Kulttour bass PDF"]
    fn external_kulttour_headers_on_all_pages() {
        let path = std::env::var_os("LIBREOMR_KULTTOUR_TEST_PDF").unwrap();
        let pages = crate::document_import::open_document(std::path::Path::new(&path)).unwrap();
        assert_eq!(pages.len(), 4);
        let signatures: [&[i8]; 4] = [
            &[-3; 9],
            &[-3, -1, -1, -1, -1, -1, -1, -1, -3, -3, -3, -3],
            &[-3, -3, -3, -3, -3, -3, -3, -3, -3, -3, -1, -1],
            &[-1; 10],
        ];
        for (page, fifths) in pages.iter().zip(signatures) {
            let image = straighten(page.render().unwrap());
            let binary = threshold(&image, 170, ThresholdType::Binary);
            let staves = detect_staves(&binary);
            assert_eq!(staves.len(), fifths.len(), "{}", page.name());
            for (index, (staff, &fifths)) in staves.iter().zip(fifths).enumerate() {
                let header = crate::symbols::recognize_header(&binary, staff);
                let location = format!("{}, staff {}: {header:?}", page.name(), index + 1);
                assert_eq!(header.clef, Clef::Bass, "{location}");
                assert_eq!(
                    header.key_signature,
                    Some(crate::model::KeySignature { fifths }),
                    "{location}"
                );
                assert_eq!(
                    header.key_bounds.len(),
                    fifths.unsigned_abs() as usize,
                    "{location}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_HOLY_NIGHT_TEST_IMAGE pointing to the Holy Night bass scan"]
    fn external_holy_night_shared_stem_note_values() {
        let path = std::env::var_os("LIBREOMR_HOLY_NIGHT_TEST_IMAGE")
            .expect("set LIBREOMR_HOLY_NIGHT_TEST_IMAGE");
        let image =
            crate::image_import::decode_image(image::ImageReader::open(path).unwrap()).unwrap();
        let score = recognize(&straighten(image));
        assert_eq!(score.staves.len(), 12);
        // Octave chords with two beams, including the partial
        // second beams; check both heads, not just the lower one.
        for (staff, x) in [
            (0, 3222.0),
            (2, 1063.0),
            (2, 3324.0),
            (3, 2306.0),
            (4, 1053.0),
            (5, 1531.0),
            (8, 4022.0),
            (9, 2766.0),
        ] {
            let chord: Vec<_> = score
                .notes
                .iter()
                .filter(|note| note.staff == staff && (note.x - x).abs() < 12.0)
                .collect();
            assert_eq!(chord.len(), 2, "staff {staff}, x {x}: {chord:?}");
            assert!(
                chord
                    .iter()
                    .all(|note| note.duration == Duration::Sixteenth && note.dots == 0),
                "staff {staff}, x {x}: {chord:?}"
            );
        }
        let first_measure = &score.measure_validities(4)[0];
        assert_eq!(first_measure.actual_ticks, 768);
        assert_eq!(first_measure.expected_ticks, Some(768));
        assert_eq!(first_measure.is_valid(), Some(true));
        let mut staff = score.staves[4].clone();
        staff.right = first_measure.right;
        let measure_score = Score {
            staves: vec![staff],
            notes: score
                .notes
                .iter()
                .filter(|note| note.staff == 4 && note.x < first_measure.right)
                .cloned()
                .map(|mut note| {
                    note.staff = 0;
                    note
                })
                .collect(),
            ..Default::default()
        };
        let export =
            crate::musicxml::export(&measure_score, "Holy Night, System 5, Takt 1").unwrap();
        assert_eq!(export.xml.matches("<chord/>").count(), 1);
        assert_eq!(export.xml.matches("<type>16th</type>").count(), 2);
        assert!(
            !export
                .warnings
                .iter()
                .any(|warning| warning.contains("Dauer"))
        );
        // Shared stems also carry flags and open/quarter heads. They must
        // retain their own values rather than inheriting a nearby beam.
        for (staff, x, duration, dots) in [
            (0, 4240.0, Duration::Eighth, 0),
            (1, 1395.0, Duration::Eighth, 0),
            (3, 4226.0, Duration::Eighth, 0),
            (5, 3022.0, Duration::Eighth, 0),
            (5, 1350.0, Duration::Eighth, 1),
            (0, 3980.0, Duration::Quarter, 1),
            (1, 1065.0, Duration::Quarter, 1),
            (2, 4303.0, Duration::Quarter, 0),
            (1, 692.0, Duration::Half, 0),
            (10, 1675.0, Duration::Half, 1),
        ] {
            let chord: Vec<_> = score
                .notes
                .iter()
                .filter(|note| note.staff == staff && (note.x - x).abs() < 12.0)
                .collect();
            assert_eq!(chord.len(), 2, "staff {staff}, x {x}: {chord:?}");
            assert!(
                chord
                    .iter()
                    .all(|note| note.duration == duration && note.dots == dots),
                "staff {staff}, x {x}: {chord:?}"
            );
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    fn upper_chord_heads_reach_the_common_beam_on_long_stems() {
        let mut image = GrayImage::from_pixel(300, 240, Luma([255]));
        let staff = Staff {
            lines: [60.0, 76.0, 92.0, 108.0, 124.0],
            left: 20.0,
            right: 280.0,
            header: Default::default(),
        };
        for &line in &staff.lines {
            for x in 20..280 {
                image.put_pixel(x, line as u32, Luma([0]));
            }
        }
        let heads: Vec<_> = [52.0, 100.0]
            .into_iter()
            .map(|y| {
                imageproc::drawing::draw_filled_ellipse_mut(
                    &mut image,
                    (100, y as i32),
                    10,
                    6,
                    Luma([0]),
                );
                crate::symbols::Notehead {
                    x: 100.0,
                    y,
                    kind: crate::symbols::HeadKind::Filled,
                }
            })
            .collect();
        for y in 52..=180 {
            for x in 91..=92 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        for y in 176..=180 {
            for x in 91..=220 {
                image.put_pixel(x, y, Luma([0]));
            }
        }
        for head in &heads {
            let stem = find_stem(&image, head.x, head.y, staff.spacing()).unwrap();
            assert_eq!(stem.direction, 1);
            assert_eq!(stem.tip, 180);
            assert_eq!(count_beams(&image, &staff, stem, head.y, &heads, &[]), 1);
        }
    }

    #[test]
    fn nearby_barlines_and_ornaments_do_not_turn_quarters_into_eighths() {
        for (scale, solid) in [(1, true), (2, true), (1, false), (2, false)] {
            let mut image = GrayImage::from_pixel(260 * scale, 180 * scale, Luma([255]));
            let gap = 18.0 * scale as f32;
            let staff = Staff {
                lines: [40.0, 58.0, 76.0, 94.0, 112.0].map(|y| y * scale as f32),
                left: 20.0 * scale as f32,
                right: 240.0 * scale as f32,
                header: Default::default(),
            };
            for &line in &staff.lines {
                for x in 20 * scale..240 * scale {
                    for offset in 0..3 * scale {
                        image.put_pixel(x, line as u32 + offset - (3 * scale / 2), Luma([0]));
                    }
                }
            }
            for y in 40 * scale..=112 * scale {
                for x in 120 * scale..=122 * scale {
                    if solid
                        || staff
                            .lines
                            .iter()
                            .any(|line| (*line - y as f32).abs() <= 5.0 * scale as f32)
                    {
                        image.put_pixel(x, y, Luma([0]));
                    }
                }
            }
            let stem = Stem {
                x: 140 * scale as i32,
                tip: 139 * scale as i32,
                direction: 1,
            };
            for y in 76 * scale..=139 * scale {
                image.put_pixel(stem.x as u32, y, Luma([0]));
            }
            let heads = [crate::symbols::Notehead {
                x: 151.0 * scale as f32,
                y: 76.0 * scale as f32,
                kind: crate::symbols::HeadKind::Filled,
            }];
            assert_eq!(
                count_beams(&image, &staff, stem, heads[0].y, &heads, &[]),
                0,
                "scale {scale}, solid {solid}"
            );
            // A real beam on the other side is still recognized with the bar
            // present: rejection must concern the vertical run, not the note.
            for x in 140 * scale..=200 * scale {
                for y in 133 * scale..=138 * scale {
                    image.put_pixel(x, y, Luma([0]));
                }
            }
            assert_eq!(
                count_beams(&image, &staff, stem, heads[0].y, &heads, &[]),
                1,
                "gap {gap}"
            );
        }
    }

    #[test]
    fn straightens_slanted_staves_and_preserves_aligned_or_nonmusic_images() {
        let mut original = GrayImage::from_pixel(1000, 500, Luma([255]));
        for row in [90, 110, 130, 150, 170, 290, 310, 330, 350, 370] {
            for column in 50..950 {
                for offset in 0..2 {
                    original.put_pixel(column, row + offset, Luma([0]));
                }
            }
        }
        imageproc::drawing::draw_filled_ellipse_mut(&mut original, (300, 140), 11, 7, Luma([0]));
        for row in 80..140 {
            original.put_pixel(310, row, Luma([0]));
        }
        assert_eq!(straighten(original.clone()), original);
        for scale in [0.75, 1.0, 1.25] {
            let scaled = image::imageops::resize(
                &original,
                (1000.0 * scale) as u32,
                (500.0 * scale) as u32,
                image::imageops::FilterType::Triangle,
            );
            for angle in [-0.009_f32, 0.009] {
                let tilted = imageproc::geometric_transformations::rotate_about_center(
                    &scaled,
                    angle,
                    imageproc::geometric_transformations::Interpolation::Bilinear,
                    Luma([255]),
                );
                assert!(detect_staves(&threshold(&tilted, 170, ThresholdType::Binary)).is_empty());
                let aligned = straighten(tilted);
                assert_eq!(aligned.dimensions(), scaled.dimensions());
                let score = recognize(&aligned);
                assert_eq!(score.staves.len(), 2, "angle {angle}, scale {scale}");
                assert!(score.notes.iter().any(|note| note.staff == 0
                    && (note.x - 300.0 * scale).abs() < 3.0
                    && (note.y - 140.0 * scale).abs() < 3.0));
                assert!(score.validate().is_ok());
                assert_eq!(straighten(aligned.clone()), aligned);
            }
        }
        let mut incomplete = GrayImage::from_pixel(1000, 500, Luma([255]));
        for row in [90, 110, 130, 150] {
            for column in 50..950 {
                incomplete.put_pixel(column, row, Luma([0]));
            }
        }
        let incomplete = imageproc::geometric_transformations::rotate_about_center(
            &incomplete,
            0.009,
            imageproc::geometric_transformations::Interpolation::Bilinear,
            Luma([255]),
        );
        assert_eq!(straighten(incomplete.clone()), incomplete);
        let small = GrayImage::from_pixel(20, 20, Luma([255]));
        assert_eq!(straighten(small.clone()), small);
    }

    #[test]
    fn recognizes_all_barline_types_without_confusing_single_dots_or_heads() {
        for scale in [0.75, 1.0, 1.4] {
            let mut image = GrayImage::from_pixel(840, 240, Luma([255]));
            for row in [60, 80, 100, 120, 140] {
                for column in 20..820 {
                    image.put_pixel(column, row, Luma([0]));
                }
            }
            for (index, kind) in BarlineKind::ALL.into_iter().enumerate() {
                let center = 100 + index as i32 * 120;
                let strokes: &[(i32, i32)] = match kind {
                    BarlineKind::Single => &[(0, 2)],
                    BarlineKind::Double => &[(-5, 2), (5, 2)],
                    BarlineKind::Final | BarlineKind::RepeatEnd => &[(-5, 2), (5, 6)],
                    BarlineKind::RepeatStart => &[(-5, 6), (5, 2)],
                    BarlineKind::RepeatBoth => &[(-10, 2), (-2, 6), (10, 2)],
                };
                for &(offset, width) in strokes {
                    for column in center + offset..center + offset + width {
                        for row in 60..=140 {
                            image.put_pixel(column as u32, row, Luma([0]));
                        }
                    }
                }
                for (direction, present) in [
                    (
                        -1,
                        matches!(kind, BarlineKind::RepeatEnd | BarlineKind::RepeatBoth),
                    ),
                    (
                        1,
                        matches!(kind, BarlineKind::RepeatStart | BarlineKind::RepeatBoth),
                    ),
                ] {
                    if present {
                        for row in [90, 110] {
                            imageproc::drawing::draw_filled_circle_mut(
                                &mut image,
                                (center + direction * 24, row),
                                3,
                                Luma([0]),
                            );
                        }
                    }
                }
            }
            imageproc::drawing::draw_filled_circle_mut(&mut image, (124, 90), 3, Luma([0]));
            for row in [90, 110] {
                imageproc::drawing::draw_filled_ellipse_mut(
                    &mut image,
                    (245, row),
                    10,
                    5,
                    Luma([0]),
                );
            }
            let image = image::imageops::resize(
                &image,
                (840.0 * scale) as u32,
                (240.0 * scale) as u32,
                image::imageops::FilterType::Triangle,
            );
            let score = recognize(&image);
            assert_eq!(
                score
                    .barlines
                    .iter()
                    .map(|bar| bar.kind)
                    .collect::<Vec<_>>(),
                BarlineKind::ALL,
                "scale {scale}: {:?}",
                score.barlines
            );
            assert!(score.validate().is_ok());
        }
    }

    #[test]
    fn barlines_are_not_note_stems_and_double_bars_are_one_boundary() {
        for scale in [0.75, 1.0, 1.4] {
            let mut image = GrayImage::from_pixel(600, 240, Luma([255]));
            for row in [60, 80, 100, 120, 140] {
                for column in 20..580 {
                    image.put_pixel(column, row, Luma([0]));
                }
            }
            for (left, width) in [(100, 2), (298, 2), (308, 5), (500, 6)] {
                for column in left..left + width {
                    for row in 60..=140 {
                        image.put_pixel(column, row, Luma([0]));
                    }
                }
            }
            imageproc::drawing::draw_filled_ellipse_mut(&mut image, (180, 150), 13, 8, Luma([0]));
            for row in 40..=150 {
                image.put_pixel(192, row, Luma([0]));
            }
            let image = image::imageops::resize(
                &image,
                (600.0 * scale) as u32,
                (240.0 * scale) as u32,
                image::imageops::FilterType::Triangle,
            );
            let score = recognize(&image);
            assert_eq!(
                score.barlines.len(),
                3,
                "scale {scale}: {:?}",
                score.barlines
            );
            for (bar, center) in score.barlines.iter().zip([100.5, 305.0, 502.5]) {
                assert!((bar.x - center * scale).abs() < 3.0, "{bar:?}");
            }
            assert_eq!(score.notes.len(), 1, "{:?}", score.notes);
            assert!(score.validate().is_ok());
        }
    }

    #[test]
    fn detached_heads_do_not_hide_bars_but_connected_half_notes_do() {
        for below in [false, true] {
            for connected in [false, true] {
                let mut image = GrayImage::from_pixel(240, 260, Luma([255]));
                let staff = Staff {
                    lines: [80.0, 110.0, 140.0, 170.0, 200.0],
                    left: 20.0,
                    right: 220.0,
                    header: StaffHeader::default(),
                };
                let head_y = if below { 224 } else { 56 };
                let stem_x = if below { 112 } else { 88 };
                let mut score = Score {
                    staves: vec![staff.clone()],
                    ..Score::default()
                };
                score.add_note(100.0, head_y as f32);
                score.notes[0].duration = Duration::Half;
                for row in 80..=200 {
                    image.put_pixel(stem_x, row, Luma([0]));
                }
                let (start, end) = if below { (201, 212) } else { (68, 79) };
                for row in start..=end {
                    if connected || !(74..=77).contains(&row) && !(203..=206).contains(&row) {
                        image.put_pixel(stem_x, row, Luma([0]));
                    }
                }
                assert!(find_stem(&image, 100.0, head_y as f32, 30.0).is_some());
                assert_eq!(
                    is_detached_open_head(&image, &staff, &score.notes[0]),
                    !connected
                );
                assert_eq!(
                    detect_barlines(&image, &staff, 0, &score.notes).len(),
                    usize::from(!connected)
                );
            }
        }
    }

    #[test]
    fn dense_beams_do_not_create_hollow_heads() {
        let mut image = GrayImage::from_pixel(500, 240, Luma([255]));
        for line in [60, 80, 100, 120, 140] {
            for x in 20..480 {
                image.put_pixel(x, line, Luma([0]));
            }
        }
        for center in [200, 240, 280] {
            imageproc::drawing::draw_filled_ellipse_mut(&mut image, (center, 90), 13, 8, Luma([0]));
            for y in 25..90 {
                for x in center + 12..=center + 13 {
                    image.put_pixel(x as u32, y, Luma([0]));
                }
            }
        }
        for top in [25, 38] {
            for y in top..top + 7 {
                for x in 212..294 {
                    image.put_pixel(x, y, Luma([0]));
                }
            }
        }
        let score = recognize(&image);
        assert_eq!(score.notes.len(), 3, "{:?}", score.notes);
        assert!(
            score
                .notes
                .iter()
                .all(|note| note.duration == Duration::Sixteenth && !note.triplet),
            "{:?}",
            score.notes
        );
    }

    #[test]
    fn recognizes_staccato_without_confusing_augmentation_dots() {
        for scale in [0.75, 1.0, 1.4] {
            for below in [false, true] {
                let mut image = GrayImage::from_pixel(500, 300, Luma([255]));
                for line in [100, 120, 140, 160, 180] {
                    for column in 20..480 {
                        image.put_pixel(column, line, Luma([0]));
                    }
                }
                let head_y = if below { 130 } else { 150 };
                for center in [100, 230, 360] {
                    imageproc::drawing::draw_filled_ellipse_mut(
                        &mut image,
                        (center, head_y),
                        12,
                        7,
                        Luma([0]),
                    );
                    let stem_x = if below { center - 11 } else { center + 11 };
                    let rows = if below {
                        head_y..head_y + 60
                    } else {
                        head_y - 60..head_y
                    };
                    for row in rows {
                        image.put_pixel(stem_x as u32, row as u32, Luma([0]));
                    }
                }
                let mark_y = if below { 210 } else { 70 };
                imageproc::drawing::draw_filled_circle_mut(&mut image, (100, mark_y), 3, Luma([0]));
                imageproc::drawing::draw_filled_circle_mut(&mut image, (256, head_y), 3, Luma([0]));
                let image = image::imageops::resize(
                    &image,
                    (500.0 * scale) as u32,
                    (300.0 * scale) as u32,
                    image::imageops::FilterType::Triangle,
                );
                let score = recognize(&image);
                assert_eq!(score.notes.len(), 3, "{scale}, {below}: {:?}", score.notes);
                assert_eq!(
                    score
                        .notes
                        .iter()
                        .map(|note| note.articulation)
                        .collect::<Vec<_>>(),
                    vec![
                        Articulation::Staccato,
                        Articulation::None,
                        Articulation::None
                    ],
                    "{scale}, {below}",
                );
                assert_eq!(
                    score.notes.iter().map(|note| note.dots).collect::<Vec<_>>(),
                    vec![0, 1, 0]
                );
            }
        }
    }

    #[test]
    fn recognizes_augmentation_dots_but_not_staccato() {
        let mut image = GrayImage::from_pixel(500, 240, Luma([255]));
        for line in [80, 100, 120, 140, 160] {
            for x in 20..480 {
                image.put_pixel(x, line, Luma([0]));
            }
        }
        for (index, center) in [100, 230, 360].into_iter().enumerate() {
            imageproc::drawing::draw_filled_ellipse_mut(
                &mut image,
                (center, 150),
                12,
                7,
                Luma([0]),
            );
            for y in 90..150 {
                image.put_pixel((center + 11) as u32, y, Luma([0]));
            }
            for dot in 0..index {
                imageproc::drawing::draw_filled_circle_mut(
                    &mut image,
                    (center + 26 + dot as i32 * 12, 150),
                    3,
                    Luma([0]),
                );
            }
        }
        imageproc::drawing::draw_filled_circle_mut(&mut image, (100, 60), 3, Luma([0]));
        let score = recognize(&image);
        assert_eq!(
            score.notes.iter().map(|note| note.dots).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(
            score
                .notes
                .iter()
                .all(|note| note.duration == Duration::Quarter)
        );
    }

    #[test]
    fn beam_extension_requires_long_thick_ink_not_a_staff_line_or_head() {
        let gap = 20.0;
        let stem = Stem {
            x: 120,
            tip: 80,
            direction: -1,
        };
        let mut image = GrayImage::from_pixel(240, 180, Luma([255]));
        for column in 20..220 {
            image.put_pixel(column, 80, Luma([0]));
        }
        assert!(!has_beam_extension(&image, stem, gap));
        imageproc::drawing::draw_filled_ellipse_mut(&mut image, (120, 80), 13, 8, Luma([0]));
        assert!(!has_beam_extension(&image, stem, gap));
        for side in [-1.0, 1.0] {
            for slope in [-0.25, 0.0, 0.25] {
                let mut image = GrayImage::from_pixel(240, 180, Luma([255]));
                for distance in 0..60 {
                    let column = (120.0 + side * distance as f32) as u32;
                    let center = (80.0 + slope * distance as f32).round() as u32;
                    for row in center - 3..=center + 3 {
                        image.put_pixel(column, row, Luma([0]));
                    }
                }
                assert!(
                    has_beam_extension(&image, stem, gap),
                    "side {side}, slope {slope}"
                );
            }
        }
    }

    #[test]
    fn detects_individual_heads_in_sloped_beams_up_to_sixty_fourths() {
        for down in [false, true] {
            for beams in 1_i32..=4 {
                let extension = (beams - 2).max(0) * 10;
                let mut image = GrayImage::from_pixel(400, 220, Luma([255]));
                for line in [60, 76, 92, 108, 124] {
                    for x in 20..380 {
                        image.put_pixel(x, line, Luma([0]));
                    }
                }
                for (index, center) in [100, 220].into_iter().enumerate() {
                    let head_y = if down {
                        76 + index as i32 * 16
                    } else {
                        124 - index as i32 * 16
                    };
                    let stem_x = center + if down { -9 } else { 9 };
                    let tip = if down {
                        132 + extension + index as i32 * 8
                    } else {
                        68 - extension - index as i32 * 8
                    };
                    imageproc::drawing::draw_filled_ellipse_mut(
                        &mut image,
                        (center, head_y),
                        10,
                        6,
                        Luma([0]),
                    );
                    for y in tip.min(head_y)..=tip.max(head_y) {
                        for x in stem_x..=stem_x + 1 {
                            image.put_pixel(x as u32, y as u32, Luma([0]));
                        }
                    }
                }
                let start_x = if down { 91 } else { 109 };
                for x in start_x..=start_x + 121 {
                    let slope = (x - start_x) * 8 / 120;
                    let tip = if down {
                        132 + extension + slope
                    } else {
                        68 - extension - slope
                    };
                    for beam in 0..beams {
                        let top = tip + if down { -beam * 10 } else { beam * 10 };
                        for y in top..top + 5 {
                            image.put_pixel(x as u32, y as u32, Luma([0]));
                        }
                    }
                }
                let score = recognize(&image);
                assert_eq!(
                    score.notes.len(),
                    2,
                    "down {down}, beams {beams}: {:?}",
                    score.notes
                );
                let expected = match beams {
                    1 => Duration::Eighth,
                    2 => Duration::Sixteenth,
                    3 => Duration::ThirtySecond,
                    _ => Duration::SixtyFourth,
                };
                assert!(
                    score.notes.iter().all(|note| note.duration == expected),
                    "down {down}, beams {beams}: {:?}",
                    score.notes
                );
            }
        }
    }

    #[test]
    fn separates_chord_heads_on_a_shared_stem() {
        let mut image = GrayImage::from_pixel(400, 220, Luma([255]));
        for line in [60, 76, 92, 108, 124] {
            for x in 20..380 {
                image.put_pixel(x, line, Luma([0]));
            }
        }
        for head_y in [108, 124] {
            imageproc::drawing::draw_filled_ellipse_mut(
                &mut image,
                (160, head_y),
                10,
                6,
                Luma([0]),
            );
        }
        for y in 60..124 {
            image.put_pixel(169, y, Luma([0]));
        }
        let score = recognize(&image);
        assert_eq!(score.notes.len(), 2, "{:?}", score.notes);
        assert!(
            score
                .notes
                .iter()
                .all(|note| note.duration == Duration::Quarter)
        );
        let mut pitches: Vec<_> = score.notes.iter().map(|note| note.pitch).collect();
        pitches.sort();
        assert_eq!(pitches, vec![30, 32]);
    }

    #[test]
    fn staff_start_lines_before_clefs_are_not_measure_boundaries() {
        for scale in [0.75, 1.0, 1.4] {
            let mut image = demo_image();
            for top in [100, 280] {
                for y in top..=top + 64 {
                    for x in [60, 470, 474] {
                        image.put_pixel(x, y, Luma([0]));
                    }
                }
            }
            let image = image::imageops::resize(
                &image,
                (1000.0 * scale) as u32,
                (460.0 * scale) as u32,
                image::imageops::FilterType::Triangle,
            );
            let score = recognize(&image);
            assert_eq!(score.staves.len(), 2);
            assert_eq!(
                score.barlines.len(),
                2,
                "scale {scale}: {:?}",
                score.barlines
            );
            for (index, staff) in score.staves.iter().enumerate() {
                let clef = staff.header.clef_bounds.as_ref().unwrap();
                let bars: Vec<_> = score
                    .barlines
                    .iter()
                    .filter(|bar| bar.staff == index)
                    .collect();
                assert!(bars.iter().all(|bar| bar.x > clef.right));
                assert!((bars[0].x / scale - 472.0).abs() < 3.0);
                assert_eq!(bars[0].kind, BarlineKind::Double);
            }
        }
    }

    #[test]
    fn detects_demo_staves_and_filled_noteheads() {
        let score = recognize(&demo_image());
        assert_eq!(score.staves.len(), 2);
        assert_eq!(score.notes.len(), 16);
        assert_eq!(score.notes[0].pitch_label(), "E4");
        assert_eq!(score.notes[1].pitch_label(), "F4");
        assert_eq!(score.staves[0].header.clef, Clef::Treble);
        assert_eq!(score.staves[1].header.clef, Clef::Bass);
        assert_eq!(
            score.staves[0].header.time_signature,
            Some(crate::model::TimeSignature::Common)
        );
        assert!(score.staves[1].header.time_signature.is_none());
        assert_eq!(score.notes[8].pitch_label(), "G2");
        assert!(score.notes.iter().all(|note| !note.confirmed));
        assert!(score.validate().is_ok());
    }

    #[test]
    fn blank_and_tiny_images_produce_no_candidates() {
        for size in [1, 100] {
            let score = recognize(&GrayImage::from_pixel(size, size, Luma([255])));
            assert!(score.staves.is_empty());
            assert!(score.notes.is_empty());
        }
    }
}
