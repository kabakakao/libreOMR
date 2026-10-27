use super::*;

fn fixture(
    scale: f32,
    meter: Option<TimeSignature>,
    incomplete: bool,
) -> (GrayImage, Staff, Vec<Barline>) {
    let mut image = GrayImage::from_pixel(1120, 220, Luma([255]));
    let mut staff = Staff {
        lines: [60.0, 80.0, 100.0, 120.0, 140.0],
        left: 20.0,
        right: 1100.0,
        header: Default::default(),
    };
    let mut bars = vec![];
    for x in [180, 400, 620, 840] {
        for y in 60..=140 {
            image.put_pixel(x, y, Luma([0]));
        }
        bars.push(Barline {
            staff: 0,
            x: x as f32,
            kind: crate::model::BarlineKind::Single,
            confirmed: false,
        });
        if x == 400 && meter.is_some() {
            continue;
        }
        imageproc::drawing::draw_filled_ellipse_mut(
            &mut image,
            (x as i32 + 50, 120),
            12,
            8,
            Luma([0]),
        );
        for y in 70..120 {
            image.put_pixel(x + 60, y, Luma([0]));
        }
    }
    if let Some(meter) = meter {
        let (height, top) = match meter {
            TimeSignature::Common => (40.0, 80),
            TimeSignature::Cut => (64.0, 68),
            _ => (78.0, 61),
        };
        let symbol = resized(&meter_template(meter), height);
        for (x, y, pixel) in symbol.enumerate_pixels() {
            if pixel[0] > 127 && (!incomplete || y < symbol.height() / 2) {
                image.put_pixel(420 + x, top + y, Luma([0]));
            }
        }
    }
    for &line in &staff.lines {
        for x in 20..1100 {
            image.put_pixel(x, line as u32, Luma([0]));
        }
    }
    let image = imageops::resize(
        &image,
        (image.width() as f32 * scale) as u32,
        (image.height() as f32 * scale) as u32,
        FilterType::Triangle,
    );
    let binary =
        imageproc::contrast::threshold(&image, 170, imageproc::contrast::ThresholdType::Binary);
    staff.lines = staff.lines.map(|y| y * scale);
    staff.left *= scale;
    staff.right *= scale;
    for bar in &mut bars {
        bar.x *= scale;
    }
    (binary, staff, bars)
}

#[test]
fn optimized_time_changes_match_the_exhaustive_search() {
    for scale in [0.75, 1.0, 1.4] {
        for meter in [
            None,
            Some(TimeSignature::Common),
            Some(TimeSignature::Cut),
            Some(TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            }),
            Some(TimeSignature::Meter {
                beats: 6,
                beat_unit: 8,
            }),
            Some(TimeSignature::Meter {
                beats: 12,
                beat_unit: 8,
            }),
            Some(TimeSignature::Meter {
                beats: 32,
                beat_unit: 32,
            }),
        ] {
            let (image, staff, bars) = fixture(scale, meter, false);
            let reference = reference_time_changes(&image, &staff, 0, &bars);
            let actual = recognize_time_changes(&image, &staff, 0, &bars);
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&reference).unwrap(),
                "{meter:?}, scale {scale}"
            );
            assert_eq!(
                actual.len(),
                usize::from(meter.is_some()),
                "{meter:?}, scale {scale}: {actual:?}"
            );
            if let Some(meter) = meter {
                assert_eq!(actual[0].time_signature, meter);
            }
        }
    }
    let (image, staff, bars) = fixture(
        1.0,
        Some(TimeSignature::Meter {
            beats: 6,
            beat_unit: 8,
        }),
        true,
    );
    assert!(recognize_time_changes(&image, &staff, 0, &bars).is_empty());
    assert!(reference_time_changes(&image, &staff, 0, &bars).is_empty());
}

#[test]
#[ignore = "manual before/after timing; run with --ignored --nocapture --test-threads=1"]
fn time_change_benchmark() {
    use std::{hint::black_box, time::Instant};
    let start = Instant::now();
    let templates = time_changes::templates();
    eprintln!(
        "cache_init_ms={:.3}, meters={}, scaled_templates={}",
        start.elapsed().as_secs_f64() * 1000.0,
        templates.len(),
        templates
            .iter()
            .map(|(_, sizes)| sizes.len())
            .sum::<usize>()
    );
    for (name, meter) in [
        ("notes_without_change", None),
        (
            "numeric_change",
            Some(TimeSignature::Meter {
                beats: 12,
                beat_unit: 8,
            }),
        ),
        ("common_time_change", Some(TimeSignature::Common)),
    ] {
        let (image, staff, bars) = fixture(1.0, meter, false);
        let expected = reference_time_changes(&image, &staff, 0, &bars);
        let actual = recognize_time_changes(&image, &staff, 0, &bars);
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            serde_json::to_value(actual).unwrap()
        );
        let start = Instant::now();
        for _ in 0..3 {
            black_box(reference_time_changes(black_box(&image), &staff, 0, &bars));
        }
        let before = start.elapsed().as_secs_f64() / 3.0;
        let start = Instant::now();
        for _ in 0..3 {
            black_box(recognize_time_changes(black_box(&image), &staff, 0, &bars));
        }
        let after = start.elapsed().as_secs_f64() / 3.0;
        eprintln!(
            "{name}: before_ms={:.3} after_ms={:.3} speedup={:.2}x",
            before * 1000.0,
            after * 1000.0,
            before / after
        );
    }
}

// Exhaustive implementation retained only as a regression and timing oracle.
fn reference_time_changes(
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
