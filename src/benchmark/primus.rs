use std::collections::BTreeMap;

use crate::model::{Clef, Duration, Score, SymbolBounds, TimeSignature};

use super::{Object, Reference};

pub(super) fn duration_name(duration: Duration) -> &'static str {
    match duration {
        Duration::Whole => "whole",
        Duration::Half => "half",
        Duration::Quarter => "quarter",
        Duration::Eighth => "eighth",
        Duration::Sixteenth => "sixteenth",
        Duration::ThirtySecond => "thirty_second",
        Duration::SixtyFourth => "sixty_fourth",
    }
}

pub(super) fn duration_ticks(duration: Duration, dots: u8, triplet: bool) -> i64 {
    let base = match duration {
        Duration::Whole => 768,
        Duration::Half => 384,
        Duration::Quarter => 192,
        Duration::Eighth => 96,
        Duration::Sixteenth => 48,
        Duration::ThirtySecond => 24,
        Duration::SixtyFourth => 12,
    };
    let dotted = match dots {
        1 => base * 3 / 2,
        2 => base * 7 / 4,
        _ => base,
    };
    if triplet { dotted * 2 / 3 } else { dotted }
}

fn duration(text: &str) -> Result<(&str, u8), String> {
    let name = text.trim_end_matches('.');
    let dots = text.len() - name.len();
    if dots > 2
        || ![
            "whole",
            "half",
            "quarter",
            "eighth",
            "sixteenth",
            "thirty_second",
            "sixty_fourth",
        ]
        .contains(&name)
    {
        return Err(format!("Unsupported PrIMuS duration: {text}"));
    }
    Ok((name, dots as u8))
}

fn pitch(text: &str) -> Result<(i32, i8), String> {
    let step = match text.chars().next() {
        Some('C') => 0,
        Some('D') => 1,
        Some('E') => 2,
        Some('F') => 3,
        Some('G') => 4,
        Some('A') => 5,
        Some('B') => 6,
        _ => return Err(format!("Invalid pitch: {text}")),
    };
    let rest = &text[1..];
    let (accidental, octave) = if let Some(s) = rest.strip_prefix('#') {
        (1, s)
    } else if let Some(s) = rest.strip_prefix('b') {
        (-1, s)
    } else if let Some(s) = rest.strip_prefix('N') {
        (0, s)
    } else {
        (0, rest)
    };
    let octave: i32 = octave
        .parse()
        .map_err(|_| format!("Unsupported pitch: {text}"))?;
    if !(0..=9).contains(&octave) {
        return Err(format!("Unsupported octave: {text}"));
    }
    Ok((octave * 7 + step, accidental))
}

pub fn parse(text: &str) -> Result<Reference, String> {
    let mut reference = Reference::default();
    let mut seen_note = false;
    for token in text.split_whitespace() {
        let mut object = if let Some(value) = token.strip_prefix("clef-") {
            if seen_note {
                return Err("Mid-staff clef changes are not supported".into());
            }
            let class = match value {
                "G2" => "clef.treble",
                "F4" => "clef.bass",
                _ => return Err(format!("Unsupported PrIMuS clef: {value}")),
            };
            Object::new("clef", class)
        } else if let Some(value) = token.strip_prefix("keySignature-") {
            if seen_note {
                return Err(
                    "Mid-staff PrIMuS key changes need an explicit measure position".into(),
                );
            }
            let names = [
                "CbM", "GbM", "DbM", "AbM", "EbM", "BbM", "FM", "CM", "GM", "DM", "AM", "EM", "BM",
                "F#M", "C#M",
            ];
            let fifths = names
                .iter()
                .position(|&name| name == value)
                .ok_or_else(|| format!("Unsupported PrIMuS key: {value}"))?
                as i32
                - 7;
            let mut object = Object::new("key", "key");
            object
                .attributes
                .insert("fifths".into(), fifths.to_string());
            object
        } else if let Some(value) = token.strip_prefix("timeSignature-") {
            if seen_note {
                return Err(
                    "Mid-staff PrIMuS time changes need an explicit measure position".into(),
                );
            }
            let mut object = Object::new("time", "time");
            let meter = match value {
                "C" => "4/4",
                "C/" => "2/2",
                _ => value,
            };
            let (beats, unit) = meter
                .split_once('/')
                .ok_or_else(|| format!("Unsupported meter: {value}"))?;
            let beats: u8 = beats
                .parse()
                .map_err(|_| format!("Invalid meter: {value}"))?;
            let unit: u8 = unit
                .parse()
                .map_err(|_| format!("Invalid meter: {value}"))?;
            if beats == 0 || ![1, 2, 4, 8, 16, 32].contains(&unit) {
                return Err(format!("Invalid meter: {value}"));
            }
            object
                .attributes
                .insert("meter".into(), format!("{beats}/{unit}"));
            object
        } else if let Some(value) = token.strip_prefix("note-") {
            seen_note = true;
            let (pitched, rhythm) = value
                .split_once('_')
                .ok_or_else(|| format!("Invalid note: {token}"))?;
            let (pitch, accidental) = pitch(pitched)?;
            let (duration, dots) = duration(rhythm)?;
            let mut object = Object::new("note", "note");
            object.attributes = BTreeMap::from([
                ("pitch".into(), pitch.to_string()),
                ("accidental".into(), accidental.to_string()),
                ("duration".into(), duration.into()),
                ("dots".into(), dots.to_string()),
                ("triplet".into(), "false".into()),
            ]);
            object
        } else if let Some(value) = token.strip_prefix("rest-") {
            seen_note = true;
            let (duration, dots) = duration(value)?;
            let mut object = Object::new("rest", "rest");
            object.attributes.insert("duration".into(), duration.into());
            object.attributes.insert("dots".into(), dots.to_string());
            object
        } else if token == "barline" {
            Object::new("barline", "barline")
        } else {
            return Err(format!("Unsupported PrIMuS token: {token}"));
        };
        // PrIMuS supplies sequence/staff positions, not pixel boxes.
        object.bounds = None;
        if ["clef", "key", "time"].contains(&object.family.as_str()) {
            if reference.headers.iter().any(|a| a.family == object.family) {
                return Err(format!("Repeated header: {}", object.family));
            }
            reference.headers.push(object);
        } else {
            reference.objects.push(object);
        }
    }
    if !reference
        .objects
        .iter()
        .any(|a| a.family == "note" || a.family == "rest")
    {
        return Err("No supported musical events in PrIMuS reference".into());
    }
    // No printed key signature represents zero fifths; an unknown recognition
    // result must still count as a miss rather than being assumed correct.
    if !reference.headers.iter().any(|a| a.family == "key") {
        let mut key = Object::new("key", "key");
        key.attributes.insert("fifths".into(), "0".into());
        reference.headers.push(key);
    }
    Ok(reference)
}

pub(super) fn note_bounds(score: &Score, staff: usize, x: f32, y: f32) -> SymbolBounds {
    let gap = score.staves[staff].spacing();
    SymbolBounds {
        left: x - gap * 0.7,
        right: x + gap * 0.7,
        top: y - gap * 0.5,
        bottom: y + gap * 0.5,
    }
}

pub fn predict(score: &Score) -> (Vec<Object>, Vec<Object>) {
    let mut positioned = Vec::new();
    for note in &score.notes {
        let mut object = Object::new("note", "note");
        object.bounds = Some(note_bounds(score, note.staff, note.x, note.y));
        object.attributes = BTreeMap::from([
            ("pitch".into(), note.pitch.to_string()),
            ("accidental".into(), note.accidental.to_string()),
            ("duration".into(), duration_name(note.duration).into()),
            ("dots".into(), note.dots.to_string()),
            ("triplet".into(), note.triplet.to_string()),
        ]);
        positioned.push((note.staff, note.x, 1, object));
    }
    for rest in &score.rests {
        let mut object = Object::new("rest", "rest");
        object.bounds = rest
            .bounds
            .clone()
            .or_else(|| Some(note_bounds(score, rest.staff, rest.x, rest.y)));
        object
            .attributes
            .insert("duration".into(), duration_name(rest.duration).into());
        object
            .attributes
            .insert("dots".into(), rest.dots.to_string());
        positioned.push((rest.staff, rest.x, 1, object));
    }
    for bar in &score.barlines {
        let staff = &score.staves[bar.staff];
        let mut object = Object::new("barline", "barline");
        object.bounds = Some(SymbolBounds {
            left: bar.x - 1.0,
            right: bar.x + 1.0,
            top: staff.lines[0],
            bottom: staff.lines[4],
        });
        positioned.push((bar.staff, bar.x, 0, object));
    }
    positioned.sort_by(|a, b| {
        score.staves[a.0].lines[0]
            .total_cmp(&score.staves[b.0].lines[0])
            .then(a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    let mut headers = Vec::new();
    // Retain extra detected staves: their headers/events must not disappear.
    for staff in &score.staves {
        let mut clef = Object::new(
            "clef",
            match staff.header.clef {
                Clef::Treble | Clef::Treble8vb => "clef.treble",
                Clef::Bass => "clef.bass",
                Clef::Unknown => "clef.unknown",
            },
        );
        clef.bounds = staff.header.clef_bounds.clone();
        headers.push(clef);
        if let Some(key) = staff.header.key_signature {
            let mut object = Object::new("key", "key");
            object
                .attributes
                .insert("fifths".into(), key.fifths.to_string());
            headers.push(object);
        }
        if let Some(time) = staff.header.time_signature {
            let mut object = Object::new("time", "time");
            let meter = match time {
                TimeSignature::Common => "4/4".into(),
                TimeSignature::Cut => "2/2".into(),
                TimeSignature::Meter { beats, beat_unit } => format!("{beats}/{beat_unit}"),
            };
            object.attributes.insert("meter".into(), meter);
            object.bounds = staff.header.time_bounds.clone();
            headers.push(object);
        }
    }
    (
        positioned
            .into_iter()
            .map(|(_, _, _, object)| object)
            .collect(),
        headers,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_alterations_dots_and_short_values() {
        let reference = parse("clef-F4 keySignature-BbM timeSignature-C/ note-Bb3_eighth.. note-C#4_thirty_second rest-sixteenth.. barline").unwrap();
        assert_eq!(reference.objects[0].attributes["pitch"], "27");
        assert_eq!(reference.objects[0].attributes["accidental"], "-1");
        assert_eq!(reference.objects[0].attributes["dots"], "2");
        assert_eq!(reference.objects[2].attributes["duration"], "sixteenth");
        assert_eq!(reference.objects[2].attributes["dots"], "2");
        assert_eq!(reference.headers[2].attributes["meter"], "2/2");
        assert_eq!(duration_ticks(Duration::SixtyFourth, 2, true), 14);
    }

    #[test]
    fn unsupported_notation_is_never_silently_dropped() {
        for input in [
            "clef-C3 note-C4_quarter",
            "note-C4_breve",
            "gracenote-C4_eighth",
            "note-C4_quarter clef-F4",
            "note-C##4_quarter",
        ] {
            assert!(parse(input).is_err(), "{input}");
        }
    }
}
