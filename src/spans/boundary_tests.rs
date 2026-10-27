use super::tests::{draw, fixture};
use super::*;
use crate::model::{Barline, BarlineKind};

#[test]
fn short_ties_attach_at_notehead_edges_including_crossed_barlines() {
    for scale in [0.75, 1.0, 1.5] {
        for side in [-1.0, 1.0] {
            for boundary in [false, true] {
                let (mut image, mut score) = fixture(scale);
                score.notes[0].y = 140.0 * scale;
                score.notes[1].x = 214.0 * scale;
                score.notes[1].y = 140.0 * scale;
                let bow = Span {
                    staff: 0,
                    kind: SpanKind::Tie,
                    start_anchor: None,
                    end_anchor: None,
                    continuation: None,
                    wedge_openings: None,
                    start: [168.0 * scale, (140.0 + side * 9.0) * scale],
                    end: [198.0 * scale, (140.0 + side * 9.0) * scale],
                    height: side * 5.0 * scale,
                    confirmed: false,
                };
                draw(&mut image, &bow);
                if boundary {
                    score.barlines.push(Barline {
                        staff: 0,
                        x: 182.0 * scale,
                        kind: BarlineKind::Single,
                        confirmed: false,
                    });
                    imageproc::drawing::draw_line_segment_mut(
                        &mut image,
                        (182.0 * scale, 100.0 * scale),
                        (182.0 * scale, 180.0 * scale),
                        Luma([0]),
                    );
                }
                let detected = recognize(&image, &score);
                assert!(
                    detected.iter().any(|s| s.kind == SpanKind::Tie
                        && s.start_anchor.as_ref().is_some_and(|a| a.note == 0)
                        && s.end_anchor.as_ref().is_some_and(|a| a.note == 1)),
                    "scale {scale}, side {side}, boundary {boundary}: {detected:?}"
                );
            }
        }
    }
}

#[test]
fn short_continuations_start_after_the_next_system_header() {
    for scale in [0.75, 1.0, 1.5] {
        for side in [-1.0, 1.0] {
            let (_, mut score) = fixture(scale);
            let mut image =
                GrayImage::from_pixel((640.0 * scale) as u32, (700.0 * scale) as u32, Luma([255]));
            score.notes.clear();
            let mut next = score.staves[0].clone();
            next.lines = next.lines.map(|y| y + 300.0 * scale);
            next.header.key_bounds.push(crate::model::SymbolBounds {
                left: 90.0 * scale,
                right: 120.0 * scale,
                top: 400.0 * scale,
                bottom: 480.0 * scale,
            });
            score.staves.push(next);
            let y = if side > 0.0 { 210.0 } else { 70.0 };
            score.add_note(570.0 * scale, y * scale).unwrap();
            score.add_note(180.0 * scale, (y + 300.0) * scale).unwrap();
            let exit = Span {
                staff: 0,
                kind: SpanKind::Tie,
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                start: [586.0 * scale, (y + side * 14.0) * scale],
                end: [619.0 * scale, (y + side * 20.0) * scale],
                height: side * 7.0 * scale,
                confirmed: false,
            };
            let entry = Span {
                staff: 1,
                start: [130.0 * scale, (y + 300.0 + side * 20.0) * scale],
                end: [164.0 * scale, (y + 300.0 + side * 14.0) * scale],
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
            let pieces = score.span_segments(&spans[0]);
            assert_eq!(pieces.len(), 2);
            assert!(pieces[1].start[0] > 120.0 * scale);
        }
    }
}
