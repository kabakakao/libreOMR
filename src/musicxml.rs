use quick_xml::{
    Writer,
    events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event},
};

use crate::diagnostics::{Issue, Object, Severity};
use crate::model::{
    Articulation, BarlineKind, Clef, Duration, Note, RepeatMark, RepeatMarkKind, Score, SpanKind,
    TimeSignature,
};

#[derive(Debug)]
pub struct Export {
    pub xml: String,
    pub warnings: Vec<String>,
    pub issues: Vec<Issue>,
}

#[derive(Default)]
struct Warnings {
    messages: Vec<String>,
    issues: Vec<Issue>,
}

impl Warnings {
    fn report(&mut self, issue: Issue) {
        self.messages.push(issue.staff.map_or_else(
            || issue.message.clone(),
            |staff| {
                crate::i18n::format(
                    "Staff {0}: {1}",
                    &[
                        ("0", format!("{}", staff + 1)),
                        ("1", issue.message.to_string()),
                    ],
                )
            },
        ));
        self.issues.push(issue);
    }
}

// Inspection runs the same timeline and checks without allocating XML output.
struct Xml(Option<Writer<Vec<u8>>>);

impl Xml {
    fn start(&mut self, name: &str, attributes: &[(&str, &str)]) {
        let Some(writer) = &mut self.0 else { return };
        let mut element = BytesStart::new(name);
        element.extend_attributes(attributes.iter().copied());
        writer.write_event(Event::Start(element)).unwrap();
    }

    fn end(&mut self, name: &str) {
        if let Some(writer) = &mut self.0 {
            writer.write_event(Event::End(BytesEnd::new(name))).unwrap();
        }
    }

    fn empty(&mut self, name: &str, attributes: &[(&str, &str)]) {
        let Some(writer) = &mut self.0 else { return };
        let mut element = BytesStart::new(name);
        element.extend_attributes(attributes.iter().copied());
        writer.write_event(Event::Empty(element)).unwrap();
    }

    fn text(&mut self, name: &str, value: impl ToString) {
        self.start(name, &[]);
        self.content(&value.to_string());
        self.end(name);
    }

    fn content(&mut self, value: &str) {
        if let Some(writer) = &mut self.0 {
            writer
                .write_event(Event::Text(BytesText::new(value)))
                .unwrap();
        }
    }
}

#[derive(Clone, Copy)]
enum Item {
    Note(usize),
    Rest(usize),
    Key(usize),
    Time(usize),
    Dynamic(usize),
    RepeatMark(usize),
}

#[derive(Clone)]
struct Mark {
    kind: SpanKind,
    phase: &'static str,
    number: String,
    placement: &'static str,
}

#[derive(Clone)]
struct WedgeEndpoint {
    x: f32,
    note: Option<usize>,
    mark: Mark,
}

fn span_marks(score: &Score, warnings: &mut Warnings) -> Vec<Vec<Mark>> {
    let mut marks: Vec<Vec<Mark>> = (0..score.notes.len()).map(|_| vec![]).collect();
    let mut occupied: Vec<(usize, usize, u8)> = vec![];
    for (span_index, span) in score
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| matches!(span.kind, SpanKind::Slur | SpanKind::Tie))
    {
        let gap = score.staves[span.staff].spacing();
        let anchor = |point: [f32; 2]| {
            score
                .notes
                .iter()
                .enumerate()
                .filter(|(_, note)| {
                    note.staff == span.staff && (note.x - point[0]).abs() <= gap * 2.0
                })
                .min_by(|(_, first), (_, second)| {
                    let distance =
                        |note: &Note| (note.x - point[0]).abs() + (note.y - point[1]).abs() * 0.05;
                    distance(first).total_cmp(&distance(second))
                })
                .map(|(index, _)| index)
        };
        let resolved = score.resolved_span(span);
        let endpoints = span
            .start_anchor
            .as_ref()
            .map(|a| a.note)
            .or_else(|| anchor(resolved.start))
            .zip(
                span.end_anchor
                    .as_ref()
                    .map(|a| a.note)
                    .or_else(|| anchor(resolved.end)),
            );
        let Some((start, end)) =
            endpoints.filter(|(start, end)| score.bow_note_order(*start, *end))
        else {
            warnings.report(Issue::warning(
                span.staff,
                Some(Object::Span(span_index)),
                Some(span.start[0]),
                crate::i18n::format("{0} without clear note anchors; omitted.", &[("0", span.kind.label().to_string())]),
                crate::i18n::tr("Drag the start and end to two different notes in the scan, or delete the slur/tie."),
            ));
            continue;
        };
        let first = &score.notes[start];
        if span.kind == SpanKind::Tie
            && (!score.tie_compatible(start, end)
                || first.accidental != score.notes[end].accidental)
        {
            warnings.report(Issue::warning(
                span.staff, Some(Object::Span(span_index)), Some(span.start[0]),
                crate::i18n::tr("Tie does not connect adjacent notes of the same pitch; omitted."),
                crate::i18n::tr("Check pitches and accidentals; change to a slur or move the anchors if needed."),
            ));
            continue;
        }
        let Some(number) = (1..=16).find(|number| {
            !occupied.iter().any(|&(left, right, used)| {
                used == *number
                    && score
                        .bow_staves(first.staff)
                        .contains(&score.notes[left].staff)
                    && !score.bow_note_order(right, start)
                    && !score.bow_note_order(end, left)
            })
        }) else {
            warnings.report(Issue::warning(
                span.staff, Some(Object::Span(span_index)), Some(span.start[0]),
                crate::i18n::tr("Too many overlapping slurs/ties; symbol omitted."),
                crate::i18n::tr("Delete duplicate slurs/ties or correct their anchors; MusicXML permits at most 16 simultaneous slurs/ties here."),
            ));
            continue;
        };
        occupied.push((start, end, number));
        for (index, phase) in [(start, "start"), (end, "stop")] {
            marks[index].push(Mark {
                kind: span.kind,
                phase,
                number: number.to_string(),
                placement: if span.height < 0.0 { "above" } else { "below" },
            });
        }
    }
    marks
}

// Allocate one number per musical wedge, including both ends across systems.
fn wedge_marks(score: &Score, warnings: &mut Warnings) -> Vec<Vec<WedgeEndpoint>> {
    let mut marks: Vec<Vec<WedgeEndpoint>> = (0..score.staves.len()).map(|_| vec![]).collect();
    let mut occupied: Vec<(usize, f32, usize, f32, u8)> = vec![];
    for (span_index, original) in score.spans.iter().enumerate().filter(|(_, s)| !s.is_bow()) {
        let span = score.resolved_span(original);
        let end_staff = span
            .end_anchor
            .as_ref()
            .map_or(span.staff, |a| score.notes[a.note].staff);
        let musical_x = |anchor: &Option<crate::model::NoteAnchor>, point: [f32; 2]| {
            anchor.as_ref().map_or(point[0], |a| score.notes[a.note].x)
        };
        let start_x = musical_x(&span.start_anchor, span.start);
        let end_x = musical_x(&span.end_anchor, span.end);
        if [span.staff, end_staff].iter().any(|&staff| {
            !score.notes.iter().any(|n| n.staff == staff)
                && !score.rests.iter().any(|r| r.staff == staff)
        }) {
            warnings.report(Issue::warning(
                span.staff,
                Some(Object::Span(span_index)),
                Some(span.start[0]),
                crate::i18n::format(
                    "{0} without notes or rests; omitted.",
                    &[("0", span.kind.label().to_string())],
                ),
                crate::i18n::tr(
                    "Add missing notes/rests in the staff or delete the dynamic wedge.",
                ),
            ));
            continue;
        }
        let rows = score.bow_staves(span.staff);
        let position = |staff, x| rows.iter().position(|&s| s == staff).map(|row| (row, x));
        let start = position(span.staff, start_x).unwrap();
        let end = position(end_staff, end_x).unwrap();
        let Some(number) = (1..=16).find(|number| {
            !occupied.iter().any(|&(a, x, b, y, used)| {
                used == *number
                    && position(a, x)
                        .zip(position(b, y))
                        .is_some_and(|(left, right)| left <= end && right >= start)
            })
        }) else {
            warnings.report(Issue::warning(span.staff, Some(Object::Span(span_index)), Some(span.start[0]),
                crate::i18n::tr("Too many overlapping wedges; symbol omitted."),
                crate::i18n::tr("Delete duplicate dynamic wedges or correct their length; at most 16 may overlap.")));
            continue;
        };
        occupied.push((span.staff, start_x, end_staff, end_x, number));
        for (staff, point, anchor, phase) in [
            (span.staff, span.start, &span.start_anchor, "start"),
            (end_staff, span.end, &span.end_anchor, "stop"),
        ] {
            marks[staff].push(WedgeEndpoint {
                x: musical_x(anchor, point),
                note: anchor.as_ref().map(|a| a.note),
                mark: Mark {
                    kind: span.kind,
                    phase,
                    number: number.to_string(),
                    placement: if point[1] < score.staves[staff].lines[2] {
                        "above"
                    } else {
                        "below"
                    },
                },
            });
        }
    }
    marks
}

// Route free endpoints to existing measures; anchors use their note's onset.
fn wedge_endpoints(
    items: &[(f32, Item)],
    bars: &[&crate::model::Barline],
    marks: &[WedgeEndpoint],
) -> Vec<Vec<WedgeEndpoint>> {
    let mut endpoints: Vec<Vec<WedgeEndpoint>> = (0..=bars.len()).map(|_| vec![]).collect();
    let mut rhythmic_segments = vec![];
    for (x, item) in items {
        if matches!(item, Item::Note(_) | Item::Rest(_)) {
            let segment = bars.partition_point(|bar| bar.x <= *x);
            if rhythmic_segments.last() != Some(&segment) {
                rhythmic_segments.push(segment);
            }
        }
    }
    for endpoint in marks {
        let target = bars.partition_point(|bar| bar.x <= endpoint.x);
        if let Some(&segment) = rhythmic_segments
            .iter()
            .min_by_key(|&&segment| segment.abs_diff(target))
        {
            endpoints[segment].push(endpoint.clone());
        }
    }
    for segment in &mut endpoints {
        segment.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    endpoints
}

fn write_wedge(xml: &mut Xml, mark: &Mark, offset: i128) {
    let kind = if mark.phase == "stop" {
        "stop"
    } else if mark.kind == SpanKind::Crescendo {
        "crescendo"
    } else {
        "diminuendo"
    };
    xml.start("direction", &[("placement", mark.placement)]);
    xml.start("direction-type", &[]);
    xml.empty("wedge", &[("type", kind), ("number", &mark.number)]);
    xml.end("direction-type");
    if offset != 0 {
        xml.text("offset", offset);
    }
    xml.end("direction");
}

fn interpolate_time(x: f32, left: (f32, u128), right: (f32, u128)) -> u128 {
    if x <= left.0 || right.0 <= left.0 {
        return left.1;
    }
    if x >= right.0 {
        return right.1;
    }
    let fraction = f64::from(x - left.0) / f64::from(right.0 - left.0);
    left.1 + ((right.1 - left.1) as f64 * fraction).round() as u128
}

fn wedge_time(x: f32, onsets: &[(f32, u128)], right: f32, elapsed: u128) -> u128 {
    let next = onsets.partition_point(|&(position, _)| position <= x);
    if next == 0 {
        return 0;
    }
    interpolate_time(
        x,
        onsets[next - 1],
        onsets.get(next).copied().unwrap_or((right, elapsed)),
    )
}

fn value(duration: Duration) -> (u32, &'static str) {
    let name = match duration {
        Duration::Whole => "whole",
        Duration::Half => "half",
        Duration::Quarter => "quarter",
        Duration::Eighth => "eighth",
        Duration::Sixteenth => "16th",
        Duration::ThirtySecond => "32nd",
        Duration::SixtyFourth => "64th",
    };
    (duration.ticks(), name)
}

fn note_duration(note: &Note, divisions: u128) -> u128 {
    note.ticks(divisions)
}

fn meter_value(meter: TimeSignature) -> (u32, u32) {
    match meter {
        TimeSignature::Common => (4, 4),
        TimeSignature::Cut => (2, 2),
        TimeSignature::Meter { beats, beat_unit } => (beats.into(), beat_unit.into()),
    }
}

fn write_time(xml: &mut Xml, meter: TimeSignature) {
    let (beats, beat_unit) = meter_value(meter);
    let symbol = match meter {
        TimeSignature::Common => "common",
        TimeSignature::Cut => "cut",
        _ => "normal",
    };
    xml.start("time", &[("symbol", symbol)]);
    xml.text("beats", beats);
    xml.text("beat-type", beat_unit);
    xml.end("time");
}

fn write_key(xml: &mut Xml, fifths: i8) {
    xml.start("key", &[]);
    xml.text("fifths", fifths);
    xml.end("key");
}

fn write_bar(xml: &mut Xml, kind: BarlineKind, left: bool, endings: &[&RepeatMark]) {
    let repeat = if left { "forward" } else { "backward" };
    xml.start(
        "barline",
        &[("location", if left { "left" } else { "right" })],
    );
    xml.text(
        "bar-style",
        match kind {
            BarlineKind::Single => "regular",
            BarlineKind::Double => "light-light",
            BarlineKind::Final => "light-heavy",
            _ if left => "heavy-light",
            _ => "light-heavy",
        },
    );
    for mark in endings {
        let numbers = mark
            .numbers
            .split(',')
            .map(|number| number.trim().parse::<u8>().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(",");
        xml.empty(
            "ending",
            &[
                ("number", numbers.as_str()),
                (
                    "type",
                    if left {
                        "start"
                    } else if mark.closed {
                        "stop"
                    } else {
                        "discontinue"
                    },
                ),
            ],
        );
    }
    if matches!(
        kind,
        BarlineKind::RepeatStart | BarlineKind::RepeatEnd | BarlineKind::RepeatBoth
    ) {
        xml.empty("repeat", &[("direction", repeat)]);
    }
    xml.end("barline");
}

fn write_note(
    xml: &mut Xml,
    note: &Note,
    chord: bool,
    marks: &[Mark],
    divisions: u128,
    lyrics: &[&crate::model::Lyric],
    polyphonic: bool,
) {
    xml.start("note", &[]);
    if note.grace {
        xml.empty("grace", &[]);
    }
    if chord {
        xml.empty("chord", &[]);
    }
    xml.start("pitch", &[]);
    xml.text(
        "step",
        ["C", "D", "E", "F", "G", "A", "B"][note.pitch.rem_euclid(7) as usize],
    );
    xml.text("alter", note.accidental);
    xml.text("octave", note.pitch.div_euclid(7));
    xml.end("pitch");
    if !note.grace {
        xml.text("duration", note_duration(note, divisions));
    }
    for phase in ["stop", "start"] {
        if marks
            .iter()
            .any(|mark| mark.kind == SpanKind::Tie && mark.phase == phase)
        {
            xml.empty("tie", &[("type", phase)]);
        }
    }
    xml.text("voice", note.voice);
    xml.start("type", if note.small { &[("size", "cue")] } else { &[] });
    xml.content(value(note.duration).1);
    xml.end("type");
    for _dot in 0..note.dots {
        xml.empty("dot", &[]);
    }
    if note.accidental_explicit {
        xml.text(
            "accidental",
            match note.accidental {
                -1 => "flat",
                1 => "sharp",
                _ => "natural",
            },
        );
    }
    if let Some(ratio) = note.tuplet_ratio() {
        xml.start("time-modification", &[]);
        xml.text("actual-notes", ratio.actual);
        xml.text("normal-notes", ratio.normal);
        xml.end("time-modification");
    }
    if polyphonic && note.duration != Duration::Whole {
        xml.text(
            "stem",
            if note.voice.is_multiple_of(2) {
                "down"
            } else {
                "up"
            },
        );
    }
    if note.articulation != Articulation::None
        || marks
            .iter()
            .any(|mark| matches!(mark.kind, SpanKind::Tie | SpanKind::Slur))
    {
        xml.start("notations", &[]);
        for mark in marks
            .iter()
            .filter(|mark| matches!(mark.kind, SpanKind::Tie | SpanKind::Slur))
        {
            xml.empty(
                if mark.kind == SpanKind::Tie {
                    "tied"
                } else {
                    "slur"
                },
                &[
                    ("type", mark.phase),
                    ("number", &mark.number),
                    ("placement", mark.placement),
                ],
            );
        }
        if note.articulation == Articulation::Fermata {
            xml.empty("fermata", &[]);
        } else if note.articulation != Articulation::None {
            xml.start("articulations", &[]);
            let kinds: &[&str] = match note.articulation {
                Articulation::Staccato => &["staccato"],
                Articulation::Accent => &["accent"],
                Articulation::Tenuto => &["tenuto"],
                Articulation::Marcato => &["strong-accent"],
                Articulation::Staccatissimo => &["staccatissimo"],
                Articulation::Portato => &["tenuto", "staccato"],
                Articulation::AccentStaccato => &["accent", "staccato"],
                Articulation::TenutoAccent => &["tenuto", "accent"],
                Articulation::MarcatoStaccato => &["strong-accent", "staccato"],
                _ => &[],
            };
            for kind in kinds {
                xml.empty(
                    kind,
                    if *kind == "strong-accent" {
                        &[("type", "up")]
                    } else {
                        &[]
                    },
                );
            }
            xml.end("articulations");
        }
        xml.end("notations");
    }
    for lyric in lyrics {
        let number = lyric.verse.to_string();
        let mut attributes = vec![("number", number.as_str())];
        if lyric.placement == crate::model::LyricPlacement::Above {
            attributes.push(("placement", "above"));
        }
        xml.start("lyric", &attributes);
        xml.text("syllabic", lyric.syllabic.xml());
        let text = lyric.verse_label.as_ref().map_or_else(
            || lyric.text.clone(),
            |label| format!("{label} {}", lyric.text),
        );
        xml.text("text", &text);
        if lyric.extend {
            xml.empty("extend", &[]);
        }
        xml.end("lyric");
    }
    xml.end("note");
}

/// Export pages in document order, preserving assigned parts and lyrics.
pub fn export_pages(pages: &[&Score], title: &str) -> Result<Export, String> {
    let (combined, order, page_starts) = combine_pages(pages, false)?;
    export_ordered(&combined, title, order, &page_starts, false)
}

type CombinedPages = (Score, Vec<usize>, std::collections::HashSet<usize>);

fn combine_pages(pages: &[&Score], inspection: bool) -> Result<CombinedPages, String> {
    let mut combined = Score::default();
    let mut order = Vec::new();
    let mut page_starts = std::collections::HashSet::new();
    for (index, page) in pages.iter().enumerate() {
        page.validate().map_err(|error| {
            crate::i18n::format(
                "Page {0}: {error}",
                &[
                    ("0", format!("{}", index + 1)),
                    ("error", error.to_string()),
                ],
            )
        })?;
        if !inspection
            && (page.staves.is_empty() || (page.notes.is_empty() && page.rests.is_empty()))
        {
            return Err(crate::i18n::format(
                "Page {0}: No notes or rests available for export.",
                &[("0", format!("{}", index + 1))],
            ));
        }
        let offset = combined.staves.len();
        let note_offset = combined.notes.len();
        combined
            .lyrics
            .extend(page.lyrics.iter().cloned().map(|mut lyric| {
                lyric.note += note_offset;
                lyric
            }));
        let mut staff_order: Vec<_> = (0..page.staves.len()).collect();
        staff_order.sort_by(|&a, &b| page.staves[a].lines[0].total_cmp(&page.staves[b].lines[0]));
        // Give the export-only combined score a continuous system order across pages.
        // Source image y coordinates restart on every page.
        for (rank, &staff) in staff_order.iter().enumerate() {
            let original = page.staff_assignments.iter().find(|a| a.staff == staff);
            combined
                .staff_assignments
                .push(crate::model::StaffAssignment {
                    staff: offset + staff,
                    system: offset + rank,
                    part: original.map_or(0, |a| a.part),
                    name: original.map_or_else(String::new, |a| a.name.clone()),
                });
        }
        if let Some(first) = staff_order.first() {
            page_starts.insert(offset + first);
        }
        let mut seen_parts = std::collections::HashSet::new();
        for staff in &staff_order {
            if let Some(assignment) = page.staff_assignments.iter().find(|a| a.staff == *staff)
                && seen_parts.insert(assignment.part)
            {
                page_starts.insert(offset + staff);
            }
        }
        order.extend(staff_order.into_iter().map(|staff| offset + staff));
        combined.staves.extend(page.staves.iter().cloned());
        // Coordinates stay local to each page; only staff references need remapping.
        macro_rules! append {
            ($($field:ident),* $(,)?) => {$(
                combined.$field.extend(page.$field.iter().cloned().map(|mut object| {
                    object.staff += offset;
                    object
                }));
            )*};
        }
        append!(
            notes,
            rests,
            barlines,
            key_changes,
            time_changes,
            dynamics,
            repeat_marks
        );
        combined
            .spans
            .extend(page.spans.iter().cloned().map(|mut span| {
                span.staff += offset;
                for anchor in [&mut span.start_anchor, &mut span.end_anchor]
                    .into_iter()
                    .flatten()
                {
                    anchor.note += note_offset;
                }
                span
            }));
    }
    Ok((combined, order, page_starts))
}

/// Read-only export preflight. Keep all findings even if one measure blocks export.
pub(crate) fn inspect_pages(pages: &[&Score]) -> Vec<Issue> {
    let mut issues = vec![];
    let mut valid = vec![];
    for (page, score) in pages.iter().enumerate() {
        if let Err(error) = score.validate() {
            issues.push(Issue::error(page, error));
            continue;
        }
        if score.staves.is_empty() || (score.notes.is_empty() && score.rests.is_empty()) {
            issues.push(Issue::error(
                page,
                crate::i18n::tr("No notes or rests available for export.").into(),
            ));
        }
        valid.push((page, *score));
    }
    let scores: Vec<_> = valid.iter().map(|(_, score)| *score).collect();
    // These scores were validated above; inspection permits empty pages.
    if let Ok((combined, order, starts)) = combine_pages(&scores, true)
        && let Ok(export) = export_ordered(&combined, "", order, &starts, true)
    {
        let mut staff_offset = 0;
        let mut offsets = [0; 8];
        for &(page, score) in &valid {
            let local_index = |object| match object {
                Object::Note(i) => Object::Note(i - offsets[0]),
                Object::Rest(i) => Object::Rest(i - offsets[1]),
                Object::Barline(i) => Object::Barline(i - offsets[2]),
                Object::Span(i) => Object::Span(i - offsets[3]),
                Object::KeyChange(i) => Object::KeyChange(i - offsets[4]),
                Object::TimeChange(i) => Object::TimeChange(i - offsets[5]),
                Object::Dynamic(i) => Object::Dynamic(i - offsets[6]),
                Object::RepeatMark(i) => Object::RepeatMark(i - offsets[7]),
            };
            for issue in &export.issues {
                if let Some(staff) = issue.staff
                    && (staff_offset..staff_offset + score.staves.len()).contains(&staff)
                {
                    let mut issue = issue.clone();
                    issue.page = page;
                    issue.staff = Some(staff - staff_offset);
                    issue.object = issue.object.map(local_index);
                    issues.push(issue);
                }
            }
            staff_offset += score.staves.len();
            for (offset, count) in offsets.iter_mut().zip([
                score.notes.len(),
                score.rests.len(),
                score.barlines.len(),
                score.spans.len(),
                score.key_changes.len(),
                score.time_changes.len(),
                score.dynamics.len(),
                score.repeat_marks.len(),
            ]) {
                *offset += count;
            }
        }
    }
    for (page, score) in valid {
        for lyric in score
            .lyrics
            .iter()
            .filter(|l| !l.confirmed || l.text.trim().is_empty())
        {
            let note = &score.notes[lyric.note];
            let mut issue = Issue::warning(
                note.staff,
                Some(Object::Note(lyric.note)),
                Some(note.x),
                crate::i18n::tr("Lyrics need review."),
                crate::i18n::tr(
                    "Compare the syllable and verse with the scan in the Lyrics editor.",
                ),
            );
            issue.page = page;
            issue.severity = if lyric.text.trim().is_empty() {
                Severity::Warning
            } else {
                Severity::Review
            };
            issues.push(issue);
        }
        macro_rules! review {
            ($field:ident, $variant:ident, $position:expr, $label:expr) => {
                for (index, object) in score.$field.iter().enumerate().filter(|(_, object)| !object.confirmed) {
                    let mut issue = Issue::warning(object.staff, Some(Object::$variant(index)),
                        Some(($position)(object)), crate::i18n::format("{0} not yet reviewed.", &[("0", format!("{}", ($label)(object)))]),
                        crate::i18n::tr("Compare recognition with the scan, correct if necessary and mark as Reviewed in the Correction panel."));
                    issue.page = page;
                    issue.severity = Severity::Review;
                    issues.push(issue);
                }
            };
        }
        review!(notes, Note, |n: &Note| n.x, |_: &Note| crate::i18n::tr(
            "Note"
        ));
        review!(
            rests,
            Rest,
            |r: &crate::model::Rest| r.x,
            |_: &crate::model::Rest| crate::i18n::tr("Rest")
        );
        review!(
            barlines,
            Barline,
            |b: &crate::model::Barline| b.x,
            |_: &crate::model::Barline| crate::i18n::tr("Barline")
        );
        review!(
            spans,
            Span,
            |s: &crate::model::Span| s.start[0],
            |s: &crate::model::Span| s.kind.label()
        );
        review!(
            key_changes,
            KeyChange,
            |c: &crate::model::KeyChange| c.x,
            |_: &crate::model::KeyChange| crate::i18n::tr("Key change")
        );
        review!(
            time_changes,
            TimeChange,
            |c: &crate::model::TimeChange| c.x,
            |_: &crate::model::TimeChange| crate::i18n::tr("Time change")
        );
        review!(
            dynamics,
            Dynamic,
            |d: &crate::model::Dynamic| d.x,
            |_: &crate::model::Dynamic| crate::i18n::tr("Dynamic marking")
        );
        review!(
            repeat_marks,
            RepeatMark,
            |m: &RepeatMark| m.x,
            |m: &RepeatMark| m.kind.label()
        );
    }
    issues.sort_by(|a, b| {
        let y = |issue: &Issue| {
            issue
                .staff
                .and_then(|staff| pages[issue.page].staves.get(staff))
                .map_or(f32::NEG_INFINITY, |staff| staff.lines[0])
        };
        a.page
            .cmp(&b.page)
            .then(y(a).total_cmp(&y(b)))
            .then(a.x.unwrap_or(0.0).total_cmp(&b.x.unwrap_or(0.0)))
    });
    issues
}

pub fn export(score: &Score, title: &str) -> Result<Export, String> {
    let mut order: Vec<_> = (0..score.staves.len()).collect();
    order.sort_by(|&a, &b| score.staves[a].lines[0].total_cmp(&score.staves[b].lines[0]));
    export_ordered(
        score,
        title,
        order,
        &std::collections::HashSet::new(),
        false,
    )
}

fn export_ordered(
    score: &Score,
    title: &str,
    staff_order: Vec<usize>,
    page_starts: &std::collections::HashSet<usize>,
    inspection: bool,
) -> Result<Export, String> {
    score.validate()?;
    let tied_score = score
        .spans
        .iter()
        .any(|s| s.kind == SpanKind::Tie)
        .then(|| {
            let mut normalized = score.clone();
            normalized.carry_tie_accidentals();
            normalized
        });
    let score = tied_score.as_ref().unwrap_or(score);
    let divisions = score.rhythm_divisions();
    if title.chars().any(|character| !matches!(character, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
        return Err(crate::i18n::tr("The title contains invalid XML characters.").into());
    }
    if !inspection
        && (score.staves.is_empty() || (score.notes.is_empty() && score.rests.is_empty()))
    {
        return Err(crate::i18n::tr("No notes or rests available for export.").into());
    }
    let mut xml = Xml((!inspection).then(|| Writer::new(Vec::new())));
    if let Some(writer) = &mut xml.0 {
        writer
            .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
            .unwrap();
    }
    xml.start("score-partwise", &[("version", "4.0")]);
    xml.start("work", &[]);
    xml.text("work-title", title);
    xml.end("work");
    xml.start("identification", &[]);
    xml.start("encoding", &[]);
    xml.text("software", "libreOMR");
    xml.end("encoding");
    xml.end("identification");
    let mut parts: std::collections::BTreeMap<usize, (String, Vec<usize>)> = Default::default();
    for staff in staff_order {
        let assignment = score.staff_assignments.iter().find(|a| a.staff == staff);
        let part = assignment.map_or(0, |a| a.part);
        let entry = parts.entry(part).or_insert_with(|| {
            (
                assignment.map_or_else(String::new, |a| a.name.clone()),
                Vec::new(),
            )
        });
        entry.1.push(staff);
    }
    xml.start("part-list", &[]);
    for (index, (name, _)) in parts.values().enumerate() {
        xml.start("score-part", &[("id", &format!("P{}", index + 1))]);
        if name.is_empty() {
            xml.empty("part-name", &[("print-object", "no")]);
            xml.empty("part-name-display", &[("print-object", "no")]);
        } else {
            xml.text("part-name", name);
        }
        xml.end("score-part");
    }
    xml.end("part-list");
    let mut warnings = Warnings::default();
    let marks = span_marks(score, &mut warnings);
    let wedge_marks = wedge_marks(score, &mut warnings);
    let unchecked = score.notes.iter().filter(|note| !note.confirmed).count()
        + score.rests.iter().filter(|rest| !rest.confirmed).count()
        + score.barlines.iter().filter(|bar| !bar.confirmed).count()
        + score
            .key_changes
            .iter()
            .filter(|change| !change.confirmed)
            .count()
        + score
            .time_changes
            .iter()
            .filter(|change| !change.confirmed)
            .count()
        + score.spans.iter().filter(|span| !span.confirmed).count()
        + score
            .dynamics
            .iter()
            .filter(|dynamic| !dynamic.confirmed)
            .count()
        + score
            .repeat_marks
            .iter()
            .filter(|mark| !mark.confirmed)
            .count()
        + score.lyrics.iter().filter(|lyric| !lyric.confirmed).count();
    if unchecked > 0 {
        warnings.messages.push(crate::i18n::format(
            "{unchecked} unreviewed objects will also be exported.",
            &[("unchecked", format!("{}", unchecked))],
        ));
    }
    for (part_index, (_, staff_order)) in parts.values().enumerate() {
        xml.start("part", &[("id", &format!("P{}", part_index + 1))]);
        let mut number = 1_u32;
        let mut meter = TimeSignature::Common;
        let mut courtesy_meter = false;
        let mut first = true;
        let mut next_left = None;
        let mut next_left_source = None;
        let mut clef = None;
        let mut new_page = false;
        for &staff_index in staff_order {
            new_page |= page_starts.contains(&staff_index);
            let staff = &score.staves[staff_index];
            let mut items: Vec<_> = score
                .notes
                .iter()
                .enumerate()
                .filter(|(_, note)| note.staff == staff_index)
                .map(|(index, note)| (note.x, Item::Note(index)))
                .chain(
                    score
                        .rests
                        .iter()
                        .enumerate()
                        .filter(|(_, rest)| rest.staff == staff_index)
                        .map(|(index, rest)| (rest.x, Item::Rest(index))),
                )
                .chain(
                    score
                        .key_changes
                        .iter()
                        .enumerate()
                        .filter(|(_, change)| change.staff == staff_index)
                        .map(|(index, change)| (change.x, Item::Key(index))),
                )
                .chain(
                    score
                        .time_changes
                        .iter()
                        .enumerate()
                        .filter(|(_, change)| change.staff == staff_index)
                        .map(|(index, change)| (change.x, Item::Time(index))),
                )
                .chain(
                    score
                        .dynamics
                        .iter()
                        .enumerate()
                        .filter(|(_, dynamic)| dynamic.staff == staff_index)
                        .map(|(index, dynamic)| {
                            let x = score
                                .notes
                                .iter()
                                .filter(|note| note.staff == staff_index)
                                .map(|note| note.x)
                                .chain(
                                    score
                                        .rests
                                        .iter()
                                        .filter(|rest| rest.staff == staff_index)
                                        .map(|rest| rest.x),
                                )
                                .filter(|x| {
                                    (*x - dynamic.x).abs() <= staff.spacing() * 2.0
                                        && !score.barlines.iter().any(|bar| {
                                            bar.staff == staff_index
                                                && bar.x > dynamic.x.min(*x)
                                                && bar.x <= dynamic.x.max(*x)
                                        })
                                })
                                .min_by(|a, b| {
                                    (*a - dynamic.x).abs().total_cmp(&(*b - dynamic.x).abs())
                                })
                                .unwrap_or(dynamic.x);
                            (x, Item::Dynamic(index))
                        }),
                )
                .chain(
                    score
                        .repeat_marks
                        .iter()
                        .enumerate()
                        .filter(|(_, mark)| {
                            mark.staff == staff_index && mark.kind != RepeatMarkKind::Ending
                        })
                        .map(|(index, mark)| {
                            let x = score
                                .notes
                                .iter()
                                .filter(|note| note.staff == staff_index)
                                .map(|note| note.x)
                                .chain(
                                    score
                                        .rests
                                        .iter()
                                        .filter(|rest| rest.staff == staff_index)
                                        .map(|rest| rest.x),
                                )
                                .filter(|x| {
                                    (*x - mark.x).abs() <= staff.spacing() * 2.0
                                        && !score.barlines.iter().any(|bar| {
                                            bar.staff == staff_index
                                                && bar.x > mark.x.min(*x)
                                                && bar.x <= mark.x.max(*x)
                                        })
                                })
                                .min_by(|a, b| (*a - mark.x).abs().total_cmp(&(*b - mark.x).abs()))
                                .unwrap_or(mark.x);
                            (x, Item::RepeatMark(index))
                        }),
                )
                .collect();
            let priority = |item| match item {
                Item::Key(_) | Item::Time(_) => 0,
                Item::Dynamic(_) | Item::RepeatMark(_) => 1,
                _ => 2,
            };
            items.sort_by(|a, b| a.0.total_cmp(&b.0).then(priority(a.1).cmp(&priority(b.1))));
            let mut bars: Vec<_> = score
                .barlines
                .iter()
                .filter(|bar| bar.staff == staff_index)
                .collect();
            bars.sort_by(|first, second| first.x.total_cmp(&second.x));
            let wedges = wedge_endpoints(&items, &bars, &wedge_marks[staff_index]);
            if first && staff.header.time_signature.is_none() {
                warnings.report(Issue::warning(
                staff_index,
                None,
                Some(staff.left),
                crate::i18n::tr("No initial time signature: assuming 4/4."),
                crate::i18n::tr("Set the initial time signature under Staff information; check note values and time changes."),
            ));
            }
            if first && staff.header.key_signature.is_none() {
                warnings.report(Issue::warning(staff_index, None, Some(staff.left),
                crate::i18n::tr("No initial key signature recognized; stored pitches are exported unchanged."),
                crate::i18n::tr("Set the key signature under Staff information, explicitly including C major / A minor.")));
            }
            if staff.header.clef == Clef::Unknown {
                warnings.report(Issue::warning(
                    staff_index,
                    None,
                    Some(staff.left),
                    crate::i18n::format(
                        "Unknown clef, {0}.",
                        &[(
                            "0",
                            (if clef.is_some() {
                                crate::i18n::tr("keeping previous clef")
                            } else {
                                crate::i18n::tr("assuming treble")
                            })
                            .to_string(),
                        )],
                    ),
                    crate::i18n::tr(
                        "Set the clef under Staff information and check the resulting pitches.",
                    ),
                ));
            }
            if let Some(time) = staff.header.time_signature {
                meter = time;
            }
            let mut ending_starts: Vec<(f32, &RepeatMark)> = vec![];
            let mut ending_stops: Vec<(f32, &RepeatMark)> = vec![];
            for (mark_index, mark) in score.repeat_marks.iter().enumerate().filter(|(_, mark)| {
                mark.staff == staff_index && mark.kind == RepeatMarkKind::Ending
            }) {
                let nearest = |x: f32| {
                    bars.iter()
                        .map(|bar| bar.x)
                        .chain([staff.left, staff.right])
                        .min_by(|a, b| (*a - x).abs().total_cmp(&(*b - x).abs()))
                        .unwrap()
                };
                let start = nearest(mark.x);
                let stop = nearest(mark.end_x);
                if mark.numbers.is_empty()
                    || (start - mark.x).abs() > staff.spacing() * 2.0
                    || (stop - mark.end_x).abs() > staff.spacing() * 2.0
                    || start >= stop
                {
                    warnings.report(Issue::warning(staff_index, Some(Object::RepeatMark(mark_index)), Some(mark.x),
                    crate::i18n::tr("Volta bracket without reliable number/measure boundaries; omitted."),
                    crate::i18n::tr("Enter ending numbers and place start/end at measure boundaries (tolerance: two line spacings).")));
                    continue;
                }
                ending_starts.push((start, mark));
                ending_stops.push((stop, mark));
            }
            let mut segment_left = staff.left;
            let mut cursor = 0;
            let mut first_in_staff = true;
            for (segment_index, boundary) in bars
                .into_iter()
                .map(Some)
                .chain(std::iter::once(None))
                .enumerate()
            {
                let end = boundary.map_or(items.len(), |bar| {
                    items.partition_point(|item| item.0 < bar.x)
                });
                let segment_right = boundary.map_or(staff.right, |bar| bar.x);
                let left_endings: Vec<_> = ending_starts
                    .iter()
                    .filter(|(x, _)| *x == segment_left)
                    .map(|(_, mark)| *mark)
                    .collect();
                let right_endings: Vec<_> = ending_stops
                    .iter()
                    .filter(|(x, _)| *x == segment_right)
                    .map(|(_, mark)| *mark)
                    .collect();
                let measure_left = segment_left;
                segment_left = segment_right;
                if cursor == end {
                    if let Some(bar) = boundary
                        && matches!(bar.kind, BarlineKind::RepeatStart | BarlineKind::RepeatBoth)
                    {
                        next_left = Some(BarlineKind::RepeatStart);
                        next_left_source = score
                            .barlines
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, bar));
                    }
                    continue;
                }
                let segment = &items[cursor..end];
                cursor = end;
                if boundary.is_none()
                    && segment
                        .iter()
                        .all(|(_, item)| matches!(item, Item::Time(_)))
                {
                    // A meter printed after the last bar announces the next row;
                    // it must not create a measure containing only attributes.
                    for (_, item) in segment {
                        if let Item::Time(index) = *item {
                            meter = score.time_changes[index].time_signature;
                        }
                    }
                    courtesy_meter = true;
                    continue;
                }
                let multirests: Vec<_> = segment
                    .iter()
                    .filter_map(|(_, item)| match item {
                        Item::Rest(index) => score.rests[*index].measures,
                        _ => None,
                    })
                    .collect();
                if !multirests.is_empty()
                    && (multirests.len() != 1
                        || segment
                            .iter()
                            .filter(|(_, item)| {
                                !matches!(item, Item::Note(index) if score.notes[*index].grace)
                                    && !matches!(
                                        item,
                                        Item::Key(_)
                                            | Item::Time(_)
                                            | Item::Dynamic(_)
                                            | Item::RepeatMark(_)
                                    )
                            })
                            .count()
                            != 1)
                {
                    let message = crate::i18n::format(
                        "Staff {0}: Multimeasure rest and other notes/rests in the same measure. Please correct barlines.",
                        &[("0", format!("{}", staff_index + 1))],
                    );
                    if !inspection {
                        return Err(message);
                    }
                    let rest = segment.iter().find_map(|(_, item)| match item {
                        Item::Rest(index) if score.rests[*index].measures.is_some() => Some(*index),
                        _ => None,
                    });
                    let mut issue = Issue::warning(
                        staff_index,
                        rest.map(Object::Rest),
                        Some(measure_left),
                        crate::i18n::tr(
                            "Multimeasure rest and other notes/rests in the same measure; export blocked.",
                        ),
                        crate::i18n::tr(
                            "Correct measure boundaries or delete incorrectly recognized notes/rests. A multimeasure rest must be alone in its measure.",
                        ),
                    );
                    issue.severity = Severity::Error;
                    issue.measure = Some(number);
                    warnings.report(issue);
                }
                let count = multirests.first().copied().unwrap_or(1);
                for repetition in 0..count {
                    let (beats, beat_unit) = meter_value(meter);
                    let mut measure_duration =
                        divisions * 4 * u128::from(beats) / u128::from(beat_unit);
                    xml.start("measure", &[("number", &number.to_string())]);
                    if first_in_staff && !first {
                        xml.empty(
                            "print",
                            &[(if new_page { "new-page" } else { "new-system" }, "yes")],
                        );
                    }
                    new_page = false;
                    if first_in_staff || (count > 1 && repetition == 0) {
                        xml.start("attributes", &[]);
                        if first {
                            xml.text("divisions", divisions);
                        }
                        if first_in_staff {
                            if let Some(key) = staff.header.key_signature {
                                write_key(&mut xml, key.fifths);
                            }
                            if first || staff.header.time_signature.is_some() || courtesy_meter {
                                write_time(&mut xml, meter);
                                courtesy_meter = false;
                            }
                            let current_clef = match staff.header.clef {
                                Clef::Unknown => clef.unwrap_or(Clef::Treble),
                                known => known,
                            };
                            if clef != Some(current_clef) {
                                xml.start("clef", &[]);
                                xml.text(
                                    "sign",
                                    if current_clef == Clef::Bass { "F" } else { "G" },
                                );
                                xml.text("line", if current_clef == Clef::Bass { 4 } else { 2 });
                                if current_clef == Clef::Treble8vb {
                                    xml.text("clef-octave-change", -1);
                                }
                                xml.end("clef");
                                clef = Some(current_clef);
                            }
                        }
                        if count > 1 {
                            xml.start("measure-style", &[]);
                            xml.text("multiple-rest", count);
                            xml.end("measure-style");
                        }
                        xml.end("attributes");
                    }
                    if repetition == 0 {
                        if let Some(kind) = next_left.take() {
                            write_bar(&mut xml, kind, true, &left_endings);
                        } else if !left_endings.is_empty() {
                            write_bar(&mut xml, BarlineKind::Single, true, &left_endings);
                        }
                    }
                    let mut voices: Vec<_> = segment
                        .iter()
                        .filter_map(|(_, item)| match *item {
                            Item::Note(i) => Some(score.notes[i].voice),
                            Item::Rest(i) => Some(score.rests[i].voice),
                            _ => None,
                        })
                        .collect();
                    voices.sort_unstable();
                    voices.dedup();
                    if voices.is_empty() {
                        voices.push(1);
                    }
                    let polyphonic = voices.len() > 1;
                    let mut previous_elapsed = 0;
                    for (voice_index, voice) in voices.into_iter().enumerate() {
                        if previous_elapsed > 0 {
                            xml.start("backup", &[]);
                            xml.text("duration", previous_elapsed);
                            xml.end("backup");
                        }
                        let voice_segment: Vec<_> = segment
                            .iter()
                            .copied()
                            .filter(|(_, item)| match *item {
                                Item::Note(i) => score.notes[i].voice == voice,
                                Item::Rest(i) => score.rests[i].voice == voice,
                                _ => voice_index == 0,
                            })
                            .collect();
                        let mut elapsed = 0;
                        let mut onsets = vec![];
                        let mut anchor_onsets = std::collections::HashMap::new();
                        let mut previous_note_time = 0;
                        let mut previous_note: Option<&Note> = None;
                        for (position, item) in &voice_segment {
                            match *item {
                                Item::RepeatMark(index) => {
                                    if repetition == 0 {
                                        let mark = &score.repeat_marks[index];
                                        xml.start("direction", &[("placement", "above")]);
                                        // MuseScore carries words formatting into the following symbol,
                                        // but ignores font-size on segno/coda themselves.
                                        xml.start("direction-type", &[]);
                                        xml.empty("words", &[("font-size", "24")]);
                                        xml.end("direction-type");
                                        xml.start("direction-type", &[]);
                                        xml.empty(
                                            if mark.kind == RepeatMarkKind::Segno {
                                                "segno"
                                            } else {
                                                "coda"
                                            },
                                            &[("font-size", "24")],
                                        );
                                        xml.end("direction-type");
                                        xml.end("direction");
                                    }
                                }
                                Item::Dynamic(index) => {
                                    if repetition == 0 {
                                        let dynamic = &score.dynamics[index];
                                        xml.start(
                                            "direction",
                                            &[(
                                                "placement",
                                                if dynamic.y < staff.lines[2] {
                                                    "above"
                                                } else {
                                                    "below"
                                                },
                                            )],
                                        );
                                        xml.start("direction-type", &[]);
                                        xml.start("dynamics", &[]);
                                        if matches!(dynamic.kind.label(), "sfzp") {
                                            xml.text("other-dynamics", dynamic.kind.label());
                                        } else {
                                            xml.empty(dynamic.kind.label(), &[]);
                                        }
                                        xml.end("dynamics");
                                        xml.end("direction-type");
                                        xml.end("direction");
                                    }
                                }
                                Item::Time(index) => {
                                    if repetition == 0 {
                                        meter = score.time_changes[index].time_signature;
                                        let (beats, beat_unit) = meter_value(meter);
                                        measure_duration = divisions * 4 * u128::from(beats)
                                            / u128::from(beat_unit);
                                        xml.start("attributes", &[]);
                                        write_time(&mut xml, meter);
                                        xml.end("attributes");
                                        previous_note = None;
                                    }
                                }
                                Item::Key(index) => {
                                    if repetition == 0 {
                                        xml.start("attributes", &[]);
                                        write_key(
                                            &mut xml,
                                            score.key_changes[index].key_signature.fifths,
                                        );
                                        xml.end("attributes");
                                    }
                                }
                                Item::Note(index) => {
                                    let note = &score.notes[index];
                                    let chord = previous_note.is_some_and(|previous| {
                                        note.grace == previous.grace
                                            && (*position - previous.x).abs()
                                                <= staff.spacing() * 0.35
                                            && note_duration(previous, divisions)
                                                == note_duration(note, divisions)
                                    });
                                    if !chord {
                                        previous_note_time = elapsed;
                                    }
                                    anchor_onsets.insert(index, previous_note_time);
                                    if !chord && !note.grace {
                                        onsets.push((*position, elapsed));
                                    }
                                    let lyrics: Vec<_> = score
                                        .lyrics
                                        .iter()
                                        .filter(|lyric| lyric.note == index)
                                        .collect();
                                    write_note(
                                        &mut xml,
                                        note,
                                        chord,
                                        &marks[index],
                                        divisions,
                                        &lyrics,
                                        score
                                            .notes
                                            .iter()
                                            .any(|n| n.staff == staff_index && n.voice > 1)
                                            || score
                                                .rests
                                                .iter()
                                                .any(|r| r.staff == staff_index && r.voice > 1),
                                    );
                                    if !chord {
                                        elapsed += note_duration(note, divisions);
                                        if !note.grace {
                                            previous_note = Some(note);
                                        }
                                    }
                                }
                                Item::Rest(index) => {
                                    let rest = &score.rests[index];
                                    let full_measure = rest.measures.is_some()
                                || (rest.duration == Duration::Whole
                                    && rest.dots == 0
                                    && voice_segment
                                        .iter()
                                        .filter(|(_, item)| {
                                            !matches!(item, Item::Note(index) if score.notes[*index].grace) && !matches!(
                                                item,
                                                Item::Key(_)
                                                    | Item::Time(_)
                                                    | Item::Dynamic(_)
                                                    | Item::RepeatMark(_)
                                            )
                                        })
                                        .count()
                                        == 1);
                                    let duration = if full_measure {
                                        measure_duration
                                    } else {
                                        rest.ticks(divisions)
                                    };
                                    onsets.push((*position, elapsed));
                                    xml.start("note", &[]);
                                    xml.empty(
                                        "rest",
                                        if full_measure {
                                            &[("measure", "yes")]
                                        } else {
                                            &[]
                                        },
                                    );
                                    xml.text("duration", duration);
                                    xml.text("voice", rest.voice);
                                    if !full_measure {
                                        xml.text("type", value(rest.duration).1);
                                        for _ in 0..rest.dots {
                                            xml.empty("dot", &[]);
                                        }
                                    }
                                    xml.end("note");
                                    elapsed += duration;
                                    previous_note = None;
                                }
                            }
                        }
                        for endpoint in wedges[segment_index].iter().filter(|e| {
                            e.note
                                .map_or(voice_index == 0, |i| score.notes[i].voice == voice)
                        }) {
                            let time = if let Some(onset) =
                                endpoint.note.and_then(|i| anchor_onsets.get(&i))
                            {
                                *onset
                            } else if count > 1 {
                                // A multi-measure rest represents the entire printed interval.
                                let total = measure_duration * u128::from(count);
                                let time = interpolate_time(
                                    endpoint.x,
                                    (measure_left, 0),
                                    (segment_right, total),
                                );
                                let target = (time / measure_duration).min(u128::from(count - 1));
                                if target != u128::from(repetition) {
                                    continue;
                                }
                                time - u128::from(repetition) * measure_duration
                            } else {
                                wedge_time(endpoint.x, &onsets, segment_right, elapsed)
                            };
                            write_wedge(&mut xml, &endpoint.mark, time as i128 - elapsed as i128);
                        }
                        if elapsed != measure_duration {
                            let mut issue = Issue::warning(
                                staff_index,
                                None,
                                Some(measure_left),
                                crate::i18n::format(
                                    "Measure {number}: duration {0} instead of {1} ({2}); check note values/measure boundaries.",
                                    &[
                                        ("number", format!("{}", number)),
                                        (
                                            "0",
                                            crate::rhythm::duration_fraction(elapsed, divisions)
                                                .to_string(),
                                        ),
                                        (
                                            "1",
                                            crate::rhythm::duration_fraction(
                                                measure_duration,
                                                divisions,
                                            )
                                            .to_string(),
                                        ),
                                        (
                                            "2",
                                            (if elapsed < measure_duration {
                                                crate::i18n::tr("too short")
                                            } else {
                                                crate::i18n::tr("too long")
                                            })
                                            .to_string(),
                                        ),
                                    ],
                                ),
                                crate::i18n::tr(
                                    "Check note values, rests, dots, tuplets, chords and barlines. A pickup may be intentionally shorter.",
                                ),
                            );
                            if polyphonic {
                                issue.message = format!(
                                    "{}: {}",
                                    crate::i18n::format(
                                        "Voice {voice}",
                                        &[("voice", voice.to_string())]
                                    ),
                                    issue.message
                                );
                            }
                            issue.measure = Some(number);
                            warnings.report(issue);
                        }
                        previous_elapsed = elapsed;
                    }
                    if repetition + 1 == count
                        && let Some(bar) = boundary
                    {
                        if matches!(bar.kind, BarlineKind::RepeatStart | BarlineKind::RepeatBoth) {
                            next_left = Some(BarlineKind::RepeatStart);
                            next_left_source = score
                                .barlines
                                .iter()
                                .position(|candidate| std::ptr::eq(candidate, bar));
                        }
                        if bar.kind != BarlineKind::RepeatStart || !right_endings.is_empty() {
                            write_bar(
                                &mut xml,
                                if bar.kind == BarlineKind::RepeatBoth {
                                    BarlineKind::RepeatEnd
                                } else if bar.kind == BarlineKind::RepeatStart {
                                    BarlineKind::Single
                                } else {
                                    bar.kind
                                },
                                false,
                                &right_endings,
                            );
                        }
                    }
                    if repetition + 1 == count && boundary.is_none() && !right_endings.is_empty() {
                        write_bar(&mut xml, BarlineKind::Single, false, &right_endings);
                    }
                    xml.end("measure");
                    number += 1;
                    first = false;
                    first_in_staff = false;
                }
            }
        }
        if next_left.is_some()
            && let Some(index) = next_left_source
        {
            let bar = &score.barlines[index];
            warnings.report(Issue::warning(
                bar.staff,
                Some(Object::Barline(index)),
                Some(bar.x),
                crate::i18n::tr("Repeat start without a following measure."),
                crate::i18n::tr("Check the barline type or add missing measures on another page."),
            ));
        }
        xml.end("part");
    }
    xml.end("score-partwise");
    Ok(Export {
        xml: xml
            .0
            .map(|writer| String::from_utf8(writer.into_inner()).unwrap())
            .unwrap_or_default(),
        warnings: warnings.messages,
        issues: warnings.issues,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn multipage_musicxml_preserves_page_order_staff_order_and_all_object_references() {
        use crate::model::{Dynamic, DynamicKind, RepeatMark, RepeatMarkKind, TimeChange};
        let mut first = score();
        for note in &mut first.notes {
            note.pitch = 28; // C4 in the upper system.
            note.staff = 1;
        }
        let mut lower = first.staves[0].clone();
        lower.lines = lower.lines.map(|y| y + 100.0);
        first.staves.insert(0, lower); // Deliberately store the lower system first.
        first.add_note(50.0, 140.0).unwrap();
        let last = first.notes.last_mut().unwrap();
        last.pitch = 29; // D4 in the lower system.
        last.duration = Duration::Whole;
        let mut second = score();
        for note in &mut second.notes {
            note.pitch = 32;
        } // G4, same coordinates as page one.
        second.add_rest(300.0, 40.0, Duration::Whole).unwrap();
        second.barlines.push(Barline {
            staff: 0,
            x: 250.0,
            kind: BarlineKind::Double,
            confirmed: true,
        });
        second.spans.push(Span {
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            staff: 0,
            kind: SpanKind::Tie,
            start: [50.0, 50.0],
            end: [100.0, 50.0],
            height: -10.0,
            confirmed: true,
        });
        second.key_changes.push(KeyChange {
            staff: 0,
            x: 150.0,
            key_signature: KeySignature { fifths: 1 },
            bounds: vec![],
            confirmed: true,
        });
        second.time_changes.push(TimeChange {
            staff: 0,
            x: 250.0,
            time_signature: TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
            bounds: vec![],
            confirmed: true,
        });
        second.dynamics.push(Dynamic {
            staff: 0,
            x: 100.0,
            y: 70.0,
            kind: DynamicKind::Mf,
            bounds: None,
            confirmed: true,
        });
        second.repeat_marks.push(RepeatMark {
            staff: 0,
            x: 100.0,
            y: 0.0,
            kind: RepeatMarkKind::Segno,
            end_x: 100.0,
            numbers: String::new(),
            closed: false,
            bounds: None,
            confirmed: true,
        });
        let before = serde_json::to_value((&first, &second)).unwrap();
        let result = export_pages(&[&first, &second], "Pages").unwrap();
        assert_xml(&result.xml);
        let steps: Vec<_> = result
            .xml
            .split("<step>")
            .skip(1)
            .map(|value| value.split("</step>").next().unwrap())
            .collect();
        assert_eq!(steps, vec!["C", "C", "C", "C", "D", "G", "G", "G", "G"]);
        assert_eq!(result.xml.matches("<part id=\"P1\">").count(), 1);
        assert_eq!(result.xml.matches("<print new-system=\"yes\"/>").count(), 1);
        assert_eq!(result.xml.matches("<print new-page=\"yes\"/>").count(), 1);
        assert_eq!(result.xml.matches("<measure number=").count(), 4);
        let page_two = result
            .xml
            .split("<print new-page=\"yes\"/>")
            .nth(1)
            .unwrap();
        assert!(page_two.contains("<mf/>") && page_two.contains("<segno "));
        assert!(
            page_two.contains("<tie type=\"start\"/>") && page_two.contains("<tie type=\"stop\"/>")
        );
        assert!(page_two.contains("<fifths>1</fifths>"));
        assert!(page_two.contains("<beats>3</beats><beat-type>4</beat-type>"));
        assert!(page_two.contains("<bar-style>light-light</bar-style>"));
        assert!(
            page_two.contains("<rest measure=\"yes\"/>")
                && page_two.contains("<duration>576</duration>")
        );
        assert_eq!(serde_json::to_value((&first, &second)).unwrap(), before);
        let reversed = export_pages(&[&second, &first], "Reordered").unwrap();
        assert_xml(&reversed.xml);
        assert!(
            reversed.xml.find("<step>G</step>").unwrap()
                < reversed.xml.find("<step>C</step>").unwrap()
        );
    }

    #[test]
    fn multipage_musicxml_carries_meter_across_pages_and_reports_empty_pages() {
        let mut first = score();
        first.notes.truncate(3);
        first.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        });
        let mut second = first.clone();
        second.staves[0].header.time_signature = None;
        let result = export_pages(&[&first, &second], "Meter").unwrap();
        assert_xml(&result.xml);
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("Dauer")),
            "{:?}",
            result.warnings
        );
        assert_eq!(result.xml.matches("<beats>3</beats>").count(), 1);
        let error = export_pages(&[&first, &Score::default()], "Empty page").unwrap_err();
        assert!(error.starts_with("Seite 2:"), "{error}");
        assert!(export_pages(&[], "Empty document").is_err());
    }

    #[test]
    fn septuplets_and_grace_notes_export_exactly_without_duration_warnings() {
        use crate::model::Tuplet;
        let mut score = score();
        score.notes.clear();
        for index in 0..7 {
            score.add_note(100.0 + index as f32 * 20.0, 40.0);
            let note = score.notes.last_mut().unwrap();
            note.duration = Duration::Eighth;
            note.tuplet = Some(Tuplet {
                actual: 7,
                normal: 4,
            });
        }
        score.add_rest(300.0, 40.0, Duration::Half);
        for x in [50.0, 65.0, 80.0] {
            score.add_note(x, 40.0);
            let note = score.notes.last_mut().unwrap();
            note.duration = Duration::Sixteenth;
            note.grace = true;
            note.small = true;
        }
        let result = export(&score, "Septolen und Vorschläge").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(result.xml.contains("<divisions>1344</divisions>"));
        assert_eq!(result.xml.matches("<duration>384</duration>").count(), 7);
        assert_eq!(
            result
                .xml
                .matches("<actual-notes>7</actual-notes><normal-notes>4</normal-notes>")
                .count(),
            7
        );
        assert_eq!(result.xml.matches("<grace/>").count(), 3);
        assert_eq!(
            result.xml.matches("<type size=\"cue\">16th</type>").count(),
            3
        );
        for note in result
            .xml
            .split("<note>")
            .skip(1)
            .filter(|note| note.contains("<grace/>"))
        {
            assert!(!note.split("</note>").next().unwrap().contains("<duration>"));
        }
        assert_eq!(score.measure_validities(0)[0].is_valid(), Some(true));
        score.notes.retain(|note| note.grace);
        score.rests[0].duration = Duration::Whole;
        assert!(
            export(&score, "Vorschlag und Taktpause")
                .unwrap()
                .warnings
                .is_empty()
        );
    }

    use super::*;
    use crate::model::{Barline, KeyChange, KeySignature, Span, Staff, StaffHeader};

    fn score() -> Score {
        let mut score = Score::default();
        score.staves.push(Staff {
            lines: [10.0, 20.0, 30.0, 40.0, 50.0],
            left: 0.0,
            right: 500.0,
            header: StaffHeader {
                time_signature: Some(TimeSignature::Common),
                key_signature: Some(KeySignature { fifths: -2 }),
                ..Default::default()
            },
        });
        for position in [50.0, 100.0, 150.0, 200.0] {
            score.add_note(position, 50.0).unwrap();
        }
        score
    }

    fn assert_xml(document: &str) {
        let mut reader = quick_xml::Reader::from_str(document);
        loop {
            match reader.read_event().unwrap() {
                Event::Eof => break,
                Event::Text(text) => {
                    text.decode().unwrap();
                }
                _ => {}
            }
        }
        if let Some(schema) = std::env::var_os("LIBREOMR_MUSICXML_SCHEMA") {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new().unwrap();
            file.write_all(document.as_bytes()).unwrap();
            let schema = std::path::PathBuf::from(schema);
            let output = std::process::Command::new("xmllint")
                .args(["--nonet", "--noout", "--path"])
                .arg(schema.parent().unwrap())
                .arg("--schema")
                .arg(&schema)
                .arg(file.path())
                .output()
                .expect("xmllint fuer die optionale MusicXML-Schemapruefung");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn repeat_marks_export_signs_and_endings_without_changing_rhythm() {
        use crate::model::{RepeatMark, RepeatMarkKind};
        let mut score = score();
        score.barlines = vec![
            Barline {
                staff: 0,
                x: 125.0,
                kind: BarlineKind::RepeatEnd,
                confirmed: true,
            },
            Barline {
                staff: 0,
                x: 250.0,
                kind: BarlineKind::Final,
                confirmed: true,
            },
        ];
        let base = RepeatMark {
            staff: 0,
            x: 50.0,
            y: -20.0,
            kind: RepeatMarkKind::Segno,
            end_x: 125.0,
            numbers: String::new(),
            closed: true,
            bounds: None,
            confirmed: true,
        };
        score.repeat_marks = vec![
            base.clone(),
            RepeatMark {
                kind: RepeatMarkKind::Coda,
                x: 150.0,
                ..base.clone()
            },
            RepeatMark {
                kind: RepeatMarkKind::Ending,
                x: 0.0,
                numbers: "1,2".into(),
                ..base.clone()
            },
            RepeatMark {
                kind: RepeatMarkKind::Ending,
                x: 125.0,
                end_x: 250.0,
                numbers: "3".into(),
                closed: false,
                ..base.clone()
            },
        ];
        let before = serde_json::to_value(&score).unwrap();
        let result = export(&score, "Test").unwrap();
        assert_eq!(result.xml.matches("<note>").count(), 4);
        assert_eq!(result.xml.matches("<measure number=").count(), 2);
        assert!(
            result
                .xml
                .contains("<direction-type><segno font-size=\"24\"/></direction-type>")
        );
        assert!(
            result
                .xml
                .contains("<direction-type><coda font-size=\"24\"/></direction-type>")
        );
        assert_eq!(result.xml.matches("<words font-size=\"24\"/>").count(), 2);
        assert!(
            result
                .xml
                .contains("<ending number=\"1,2\" type=\"start\"/>")
        );
        assert!(
            result
                .xml
                .contains("<ending number=\"1,2\" type=\"stop\"/><repeat direction=\"backward\"/>")
        );
        assert!(
            result
                .xml
                .contains("<ending number=\"3\" type=\"discontinue\"/>")
        );
        assert_eq!(serde_json::to_value(&score).unwrap(), before);
        score.repeat_marks[2].numbers.clear();
        let result = export(&score, "Test").unwrap();
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("Voltenklammer"))
        );
        assert!(!result.xml.contains("<ending number=\"\""));
    }

    #[test]
    fn exports_dynamics_and_extended_articulations_without_changing_rhythm() {
        use crate::model::{Dynamic, DynamicKind};
        for (kind, expected) in [
            (Articulation::Marcato, "<strong-accent type=\"up\"/>"),
            (Articulation::Staccatissimo, "<staccatissimo/>"),
            (Articulation::Portato, "<tenuto/><staccato/>"),
            (Articulation::AccentStaccato, "<accent/><staccato/>"),
            (Articulation::TenutoAccent, "<tenuto/><accent/>"),
            (
                Articulation::MarcatoStaccato,
                "<strong-accent type=\"up\"/><staccato/>",
            ),
        ] {
            let mut score = score();
            score.notes[0].articulation = kind;
            let result = export(&score, "Test").unwrap();
            assert_xml(&result.xml);
            assert!(result.xml.contains(expected), "{}", result.xml);
        }
        for kind in DynamicKind::ALL {
            let mut score = score();
            score.dynamics.push(Dynamic {
                staff: 0,
                x: score.notes[1].x + 2.0,
                y: 100.0,
                kind,
                bounds: None,
                confirmed: false,
            });
            let original = serde_json::to_value(&score).unwrap();
            let result = export(&score, "Test").unwrap();
            assert_xml(&result.xml);
            assert_eq!(result.xml.matches("<note>").count(), 4);
            let tag = if kind == DynamicKind::Sfzp {
                "<other-dynamics>sfzp</other-dynamics>".into()
            } else {
                format!("<{}/>", kind.label())
            };
            assert!(
                result.xml.contains(&format!("<dynamics>{tag}</dynamics>")),
                "{}",
                result.xml
            );
            assert!(
                result.xml.find("<direction ").unwrap()
                    < result.xml.match_indices("<note>").nth(1).unwrap().0
            );
            assert_eq!(
                result.warnings,
                ["1 ungeprüfte Objekte werden mit exportiert."]
            );
            assert_eq!(serde_json::to_value(&score).unwrap(), original);
            score.notes.clear();
            score.rests = vec![crate::model::Rest {
                staff: 0,
                voice: 1,
                x: 100.0,
                y: 40.0,
                duration: Duration::Whole,
                dots: 0,
                measures: Some(3),
                bounds: None,
                confirmed: true,
            }];
            score.dynamics[0].x = 100.0;
            let result = export(&score, "Test").unwrap();
            assert_eq!(result.xml.matches("<dynamics>").count(), 1);
            assert_eq!(result.xml.matches("<rest measure=\"yes\"/>").count(), 3);
        }
    }

    #[test]
    fn musicxml_exports_pitch_meter_key_and_escaped_title_without_mutation() {
        let score = score();
        let before = serde_json::to_string(&score).unwrap();
        let result = export(&score, "A & B <Test> \"Stimme\"").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert!(result.xml.contains("A &amp; B &lt;Test&gt;"));
        assert!(result.xml.contains("<fifths>-2</fifths>"));
        assert!(
            result
                .xml
                .contains("<step>E</step><alter>-1</alter><octave>4</octave>")
        );
        assert_eq!(result.xml.matches("<note>").count(), 4);
        assert_eq!(before, serde_json::to_string(&score).unwrap());
    }

    #[test]
    fn musicxml_preserves_dots_triplets_accidentals_and_articulations() {
        let mut score = score();
        score.notes[0].dots = 2;
        score.notes[0].duration = Duration::Sixteenth;
        score.notes[0].triplet = true;
        score.notes[0].accidental = 0;
        score.notes[0].accidental_explicit = true;
        score.notes[0].articulation = Articulation::Staccato;
        score.notes[1].articulation = Articulation::Accent;
        score.notes[2].articulation = Articulation::Tenuto;
        score.notes[3].articulation = Articulation::Fermata;
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        for expected in [
            "<duration>56</duration>",
            "<dot/><dot/>",
            "<actual-notes>3</actual-notes>",
            "<accidental>natural</accidental>",
            "<staccato/>",
            "<accent/>",
            "<tenuto/>",
            "<fermata/>",
        ] {
            assert!(result.xml.contains(expected), "{expected}");
        }
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn musicxml_splits_measures_and_places_both_repeat_directions() {
        let mut score = score();
        score.barlines = vec![
            Barline {
                staff: 0,
                x: 0.0,
                kind: BarlineKind::RepeatStart,
                confirmed: false,
            },
            Barline {
                staff: 0,
                x: 125.0,
                kind: BarlineKind::RepeatBoth,
                confirmed: false,
            },
            Barline {
                staff: 0,
                x: 250.0,
                kind: BarlineKind::RepeatEnd,
                confirmed: false,
            },
        ];
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        assert_eq!(result.xml.matches("<measure ").count(), 2);
        assert_eq!(result.xml.matches("direction=\"forward\"").count(), 2);
        assert_eq!(result.xml.matches("direction=\"backward\"").count(), 2);
    }

    #[test]
    fn musicxml_rejects_empty_and_invalid_scores() {
        assert!(export(&Score::default(), "Test").is_err());
        let mut score = score();
        assert!(export(&score, "Bad\0title").is_err());
        score.notes[0].x = f32::NAN;
        assert!(export(&score, "Test").is_err());
    }

    #[test]
    fn musicxml_expands_multirests_with_meter_and_continuous_measure_numbers() {
        let mut score = score();
        score.notes.clear();
        score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        });
        let index = score.add_rest(50.0, 30.0, Duration::Whole).unwrap();
        score.rests[index].measures = Some(3);
        score.barlines.push(Barline {
            staff: 0,
            x: 125.0,
            kind: BarlineKind::Double,
            confirmed: true,
        });
        score.add_rest(150.0, 30.0, Duration::Whole).unwrap();
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.xml.matches("<measure ").count(), 4);
        assert_eq!(result.xml.matches("<rest measure=\"yes\"/>").count(), 4);
        assert_eq!(result.xml.matches("<duration>576</duration>").count(), 4);
        assert_eq!(
            result
                .xml
                .matches("<multiple-rest>3</multiple-rest>")
                .count(),
            1
        );
        assert!(result.xml.contains("<measure number=\"4\">"));
        score.add_note(60.0, 50.0).unwrap();
        assert!(
            export(&score, "Test")
                .unwrap_err()
                .contains("Mehrtaktpause")
        );
    }

    #[test]
    fn musicxml_orders_systems_events_and_carries_meter_without_empty_edge_measures() {
        let mut score = score();
        let mut upper = score.staves[0].clone();
        upper.lines = [-100.0, -90.0, -80.0, -70.0, -60.0];
        upper.header.clef = Clef::Bass;
        upper.header.time_signature = Some(TimeSignature::Cut);
        score.staves[0].header.time_signature = None;
        score.staves.push(upper);
        for position in [50.0, 100.0, 150.0, 200.0] {
            score.add_note(position, -60.0).unwrap();
        }
        score.notes.reverse();
        for staff in [0, 1] {
            for position in [0.0, 500.0] {
                score.barlines.push(Barline {
                    staff,
                    x: position,
                    kind: BarlineKind::Final,
                    confirmed: true,
                });
            }
        }
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.xml.matches("<measure ").count(), 2);
        assert_eq!(result.xml.matches("<time ").count(), 1);
        assert!(
            result
                .xml
                .contains("<time symbol=\"cut\"><beats>2</beats><beat-type>2</beat-type>")
        );
        assert!(result.xml.contains("<print new-system=\"yes\"/>"));
        assert!(
            result.xml.find("<sign>F</sign>").unwrap() < result.xml.find("<sign>G</sign>").unwrap()
        );
    }

    #[test]
    fn musicxml_hides_part_name_and_only_exports_actual_clef_changes() {
        let mut score = score();
        for (index, clef) in [
            Clef::Treble,
            Clef::Treble,
            Clef::Bass,
            Clef::Bass,
            Clef::Unknown,
            Clef::Treble,
        ]
        .into_iter()
        .enumerate()
        {
            let mut staff = score.staves[0].clone();
            staff.lines = staff.lines.map(|line| line + index as f32 * 100.0);
            staff.header.clef = clef;
            if index == 0 {
                score.staves[0] = staff;
            } else {
                score.staves.push(staff);
            }
            if index > 0 {
                for x in [50.0, 100.0, 150.0, 200.0] {
                    score.add_note(x, 50.0 + index as f32 * 100.0).unwrap();
                }
            }
        }
        let result = export(&score, "Schluesselwechsel").unwrap();
        assert_xml(&result.xml);
        assert!(!result.xml.contains("Stimme"));
        assert!(result.xml.contains("<part-name print-object=\"no\"/>"));
        assert!(
            result
                .xml
                .contains("<part-name-display print-object=\"no\"/>")
        );
        assert_eq!(result.xml.matches("<clef>").count(), 3);
        assert_eq!(result.xml.matches("<print new-system=\"yes\"/>").count(), 5);
        let measures: Vec<_> = result.xml.split("<measure ").skip(1).collect();
        for (index, sign) in [(0, "G"), (2, "F"), (5, "G")] {
            assert!(measures[index].contains(&format!("<clef><sign>{sign}</sign>")));
        }
        for index in [1, 3, 4] {
            assert!(!measures[index].contains("<clef>"));
        }
        assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
        assert!(result.warnings[0].contains("vorheriger Schlüssel beibehalten"));
    }

    #[test]
    fn musicxml_preserves_chords_rests_and_inline_keys_in_event_order() {
        let mut score = score();
        let mut chord = score.notes[0].clone();
        chord.pitch += 2;
        chord.y -= 10.0;
        score.notes.push(chord);
        score.notes.remove(2);
        score.add_rest(150.0, 30.0, Duration::Quarter).unwrap();
        score.key_changes.push(KeyChange {
            staff: 0,
            x: 100.0,
            key_signature: KeySignature { fifths: 1 },
            bounds: vec![],
            confirmed: true,
        });
        score.recompute_accidentals();
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
        assert!(result.warnings[0].contains("ungeprüfte"));
        assert_eq!(result.xml.matches("<chord/>").count(), 1);
        assert_eq!(result.xml.matches("<rest/>").count(), 1);
        let key = result.xml.find("<fifths>1</fifths>").unwrap();
        let rest = result.xml.find("<rest/>").unwrap();
        assert!(key < rest);
    }

    #[test]
    fn musicxml_exports_ties_slurs_and_both_wedges_with_matching_endpoints() {
        let mut score = score();
        for kind in SpanKind::ALL {
            score.spans.push(Span {
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                staff: 0,
                kind,
                start: [50.0, 45.0],
                end: [100.0, 45.0],
                height: if kind == SpanKind::Slur { -10.0 } else { 10.0 },
                confirmed: true,
            });
        }
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        for expected in [
            "<tie type=\"start\"/>",
            "<tie type=\"stop\"/>",
            "<tied type=\"start\"",
            "<tied type=\"stop\"",
            "<slur type=\"start\"",
            "<slur type=\"stop\"",
            "<wedge type=\"crescendo\"",
            "<wedge type=\"diminuendo\"",
        ] {
            assert!(result.xml.contains(expected), "{expected}");
        }
        assert_eq!(result.xml.matches("<wedge type=\"stop\"").count(), 2);
        assert!(
            result.xml.find("<tie type=\"start\"/>").unwrap()
                < result.xml.find("<type>quarter</type>").unwrap()
        );
        score.notes[1].pitch += 1;
        let result = export(&score, "Test").unwrap();
        assert!(!result.xml.contains("<tie "));
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("Haltebogen"))
        );
        score.spans[0].end[0] = 450.0;
        let result = export(&score, "Test").unwrap();
        assert_eq!(result.xml.matches("<wedge type=\"stop\"").count(), 2);
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("Notenanker"))
        );
        score.spans[2].end[0] = 450.0;
        assert!(
            export(&score, "Test")
                .unwrap()
                .warnings
                .iter()
                .any(|warning| warning.contains("Notenanker"))
        );
    }

    #[test]
    fn musicxml_wedges_inside_held_notes_and_rests_need_no_note_anchors() {
        for rest in [false, true] {
            let mut score = score();
            score.notes.truncate(1);
            score.notes[0].duration = Duration::Whole;
            if rest {
                score.notes.clear();
                score.add_rest(50.0, 30.0, Duration::Whole).unwrap();
                score.rests[0].confirmed = true;
            }
            score.spans.push(Span {
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                staff: 0,
                kind: SpanKind::Decrescendo,
                start: [200.0, 70.0],
                end: [350.0, 70.0],
                height: 10.0,
                confirmed: true,
            });
            let before = serde_json::to_string(&score).unwrap();
            let result = export(&score, "Gabel im gehaltenen Ton").unwrap();
            assert_xml(&result.xml);
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert!(result.xml.contains(
                "<wedge type=\"diminuendo\" number=\"1\"/></direction-type><offset>-512</offset>"
            ));
            assert!(result.xml.contains(
                "<wedge type=\"stop\" number=\"1\"/></direction-type><offset>-256</offset>"
            ));
            assert_eq!(result.xml.matches("<measure ").count(), 1);
            assert_eq!(result.xml.matches("<note>").count(), 1);
            assert!(result.xml.contains("<duration>768</duration>"));
            assert_eq!(serde_json::to_string(&score).unwrap(), before);
        }
    }

    #[test]
    fn musicxml_duplicate_visual_ties_emit_one_sound_tie_per_phase() {
        let mut score = score();
        for (start, end) in [(50.0, 100.0), (100.0, 150.0)] {
            for height in [-10.0, 10.0] {
                score.spans.push(Span {
                    start_anchor: None,
                    end_anchor: None,
                    continuation: None,
                    wedge_openings: None,
                    staff: 0,
                    kind: SpanKind::Tie,
                    start: [start, 45.0],
                    end: [end, 45.0],
                    height,
                    confirmed: true,
                });
            }
        }
        let result = export(&score, "Doppelte Halteboegen").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.xml.matches("<tie type=\"start\"/>").count(), 2);
        assert_eq!(result.xml.matches("<tie type=\"stop\"/>").count(), 2);
        assert_eq!(result.xml.matches("<tied ").count(), 8);
        assert!(
            result
                .xml
                .contains("<tie type=\"stop\"/><tie type=\"start\"/>")
        );
    }

    #[test]
    fn musicxml_wedges_cross_bars_and_stop_at_the_final_bar_without_extra_measures() {
        let mut score = score();
        for (note, x) in score.notes.iter_mut().zip([50.0, 150.0, 300.0, 400.0]) {
            note.x = x;
            note.duration = Duration::Half;
        }
        for x in [0.0, 250.0, 500.0] {
            score.barlines.push(Barline {
                staff: 0,
                x,
                kind: BarlineKind::Single,
                confirmed: true,
            });
        }
        score.spans.push(Span {
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            staff: 0,
            kind: SpanKind::Crescendo,
            start: [200.0, 0.0],
            end: [500.0, 0.0],
            height: 10.0,
            confirmed: true,
        });
        let result = export(&score, "Taktuebergreifende Gabel").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let measures: Vec<_> = result.xml.split("</measure>").collect();
        assert_eq!(result.xml.matches("<measure ").count(), 2);
        assert!(measures[0].contains(
            "<direction placement=\"above\"><direction-type><wedge type=\"crescendo\" number=\"1\"/></direction-type><offset>-192</offset>"
        ));
        assert!(
            measures[1]
                .contains("<wedge type=\"stop\" number=\"1\"/></direction-type></direction>")
        );
        assert_eq!(result.xml.matches("<duration>384</duration>").count(), 4);
    }

    #[test]
    fn musicxml_wedges_over_multirests_emit_endpoints_once_in_the_matching_measures() {
        let mut score = score();
        score.notes.clear();
        score.add_rest(250.0, 30.0, Duration::Whole).unwrap();
        score.rests[0].measures = Some(2);
        score.rests[0].confirmed = true;
        score.spans.push(Span {
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            staff: 0,
            kind: SpanKind::Crescendo,
            start: [125.0, 70.0],
            end: [375.0, 70.0],
            height: 10.0,
            confirmed: true,
        });
        let result = export(&score, "Gabel ueber Mehrtaktpause").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let measures: Vec<_> = result.xml.split("</measure>").collect();
        assert_eq!(result.xml.matches("<measure ").count(), 2);
        for (measure, kind) in [(measures[0], "crescendo"), (measures[1], "stop")] {
            assert!(measure.contains(&format!(
                "<wedge type=\"{kind}\" number=\"1\"/></direction-type><offset>-384</offset>"
            )));
        }
        assert_eq!(result.xml.matches("<wedge ").count(), 2);
        assert_eq!(result.xml.matches("<duration>768</duration>").count(), 2);
    }

    #[test]
    fn musicxml_overlapping_wedges_keep_time_with_chords_and_grace_notes() {
        let mut score = score();
        let mut chord = score.notes[0].clone();
        chord.pitch += 2;
        chord.y -= 10.0;
        score.notes.push(chord);
        let mut grace = score.notes[0].clone();
        grace.x = 75.0;
        grace.grace = true;
        score.notes.push(grace);
        for (kind, start, end) in [
            (SpanKind::Crescendo, 50.0, 150.0),
            (SpanKind::Decrescendo, 100.0, 200.0),
        ] {
            score.spans.push(Span {
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                staff: 0,
                kind,
                start: [start, 70.0],
                end: [end, 70.0],
                height: 10.0,
                confirmed: true,
            });
        }
        let result = export(&score, "Akkord und Vorschlag").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        for (kind, number, offset) in [
            ("crescendo", 1, -768),
            ("stop", 1, -384),
            ("diminuendo", 2, -576),
            ("stop", 2, -192),
        ] {
            assert!(result.xml.contains(&format!(
                "<wedge type=\"{kind}\" number=\"{number}\"/></direction-type><offset>{offset}</offset>"
            )));
        }
        assert_eq!(result.xml.matches("<chord/>").count(), 1);
        assert_eq!(result.xml.matches("<grace/>").count(), 1);
        assert_eq!(result.xml.matches("<duration>192</duration>").count(), 5);
    }

    #[test]
    fn courtesy_meters_at_staff_ends_do_not_create_empty_measures() {
        let mut score = score();
        score.barlines.push(Barline {
            staff: 0,
            x: 250.0,
            kind: BarlineKind::Double,
            confirmed: true,
        });
        score.time_changes.push(crate::model::TimeChange {
            staff: 0,
            x: 275.0,
            time_signature: TimeSignature::Meter {
                beats: 3,
                beat_unit: 4,
            },
            bounds: vec![],
            confirmed: true,
        });
        let mut next = score.staves[0].clone();
        next.lines = next.lines.map(|line| line + 100.0);
        next.header.time_signature = None;
        next.header.key_signature = None;
        score.staves.push(next);
        score.add_rest(100.0, 130.0, Duration::Whole);
        let result = export(&score, "Courtesy meter").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.xml.matches("<measure ").count(), 2);
        assert_eq!(result.xml.matches("<time ").count(), 2);
        assert_eq!(result.xml.matches("<duration>576</duration>").count(), 1);
        let second = result.xml.split("<measure ").nth(2).unwrap();
        assert!(
            second
                .find("<beats>3</beats><beat-type>4</beat-type>")
                .unwrap()
                < second.find("<rest measure=\"yes\"/>").unwrap()
        );
    }

    #[test]
    fn inline_meters_change_measure_lengths_and_carry_into_later_staves() {
        let mut score = score();
        score.barlines = [250.0, 350.0]
            .into_iter()
            .map(|x| Barline {
                staff: 0,
                x,
                kind: BarlineKind::Single,
                confirmed: true,
            })
            .collect();
        score.time_changes = vec![
            crate::model::TimeChange {
                staff: 0,
                x: 265.0,
                time_signature: TimeSignature::Meter {
                    beats: 3,
                    beat_unit: 4,
                },
                bounds: vec![],
                confirmed: true,
            },
            crate::model::TimeChange {
                staff: 0,
                x: 365.0,
                time_signature: TimeSignature::Meter {
                    beats: 5,
                    beat_unit: 8,
                },
                bounds: vec![],
                confirmed: true,
            },
        ];
        score.add_rest(300.0, 30.0, Duration::Whole);
        score.add_rest(450.0, 30.0, Duration::Whole);
        let mut next = score.staves[0].clone();
        next.lines = next.lines.map(|y| y + 100.0);
        next.header.time_signature = None;
        next.header.key_signature = None;
        score.staves.push(next);
        score.add_rest(100.0, 130.0, Duration::Whole);
        let result = export(&score, "Taktwechsel").unwrap();
        assert_xml(&result.xml);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.xml.matches("<measure ").count(), 4);
        assert_eq!(result.xml.matches("<time ").count(), 3);
        assert_eq!(result.xml.matches("<duration>576</duration>").count(), 1);
        assert_eq!(result.xml.matches("<duration>480</duration>").count(), 2);
        let meter = result
            .xml
            .find("<beats>3</beats><beat-type>4</beat-type>")
            .unwrap();
        let rest = result.xml.find("<duration>576</duration>").unwrap();
        assert!(meter < rest);
    }

    #[test]
    fn dotted_rests_export_dots_and_exact_duration_including_whole_rests() {
        for (duration, kind, ticks) in [
            (Duration::Whole, "whole", [1152, 1344]),
            (Duration::Half, "half", [576, 672]),
            (Duration::Quarter, "quarter", [288, 336]),
            (Duration::Eighth, "eighth", [144, 168]),
            (Duration::Sixteenth, "16th", [72, 84]),
            (Duration::ThirtySecond, "32nd", [36, 42]),
            (Duration::SixtyFourth, "64th", [18, 21]),
        ] {
            for dots in 1..=2 {
                let mut score = score();
                score.notes.clear();
                let index = score.add_rest(100.0, 30.0, duration).unwrap();
                score.rests[index].dots = dots;
                let result = export(&score, "Punktierte Pause").unwrap();
                assert_xml(&result.xml);
                assert!(
                    result.xml.contains(&format!(
                        "<rest/><duration>{}</duration><voice>1</voice><type>{kind}</type>{}",
                        ticks[usize::from(dots - 1)],
                        "<dot/>".repeat(usize::from(dots))
                    )),
                    "{}",
                    result.xml
                );
                assert!(!result.xml.contains("<rest measure=\"yes\"/>"));
            }
        }
    }

    #[test]
    fn short_values_export_exact_dotted_and_triplet_durations_and_roundtrip() {
        for (duration, kind, ticks) in [
            (Duration::ThirtySecond, "32nd", [24, 36, 42]),
            (Duration::SixtyFourth, "64th", [12, 18, 21]),
        ] {
            for dots in 0..=2 {
                for triplet in [false, true] {
                    let mut score = score();
                    score.notes.truncate(1);
                    score.notes[0].duration = duration;
                    score.notes[0].dots = dots;
                    score.notes[0].triplet = triplet;
                    score.add_rest(300.0, 30.0, duration).unwrap();
                    let restored: Score =
                        serde_json::from_str(&serde_json::to_string(&score).unwrap()).unwrap();
                    assert_eq!(restored.notes[0].duration, duration);
                    assert_eq!(restored.rests[0].duration, duration);
                    let result = export(&restored, "Short values").unwrap();
                    assert_xml(&result.xml);
                    let expected = ticks[dots as usize] * if triplet { 2 } else { 3 } / 3;
                    assert!(
                        result
                            .xml
                            .contains(&format!("<duration>{expected}</duration>"))
                    );
                    assert!(result.xml.contains(&format!(
                        "<rest/><duration>{}</duration><voice>1</voice><type>{kind}</type>",
                        ticks[0]
                    )));
                    assert_eq!(
                        result.xml.matches(&format!("<type>{kind}</type>")).count(),
                        2
                    );
                }
            }
        }
    }

    #[test]
    fn musicxml_covers_all_note_values_and_reports_assumptions_and_unchecked_objects() {
        let mut score = score();
        score.notes.clear();
        for (index, duration) in Duration::ALL.into_iter().enumerate() {
            score.add_note(50.0 + index as f32 * 50.0, 50.0).unwrap();
            score.notes[index].duration = duration;
        }
        score.staves[0].header = StaffHeader {
            clef: Clef::Unknown,
            ..Default::default()
        };
        score.notes[0].confirmed = false;
        let result = export(&score, "Test").unwrap();
        assert_xml(&result.xml);
        for duration in Duration::ALL {
            assert!(
                result
                    .xml
                    .contains(&format!("<type>{}</type>", value(duration).1))
            );
        }
        for expected in [
            "4/4",
            "Vorzeichnung",
            "Violin-Annahme",
            "ungeprüfte",
            "Takt 1:",
        ] {
            assert!(
                result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains(expected)),
                "{expected}"
            );
        }
    }
}
