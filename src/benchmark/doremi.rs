use std::collections::BTreeMap;

use quick_xml::{Reader, events::Event};

use crate::model::{Clef, Duration, Score, SymbolBounds};

use super::{Object, Reference, primus};

fn class(name: &str) -> Option<(&'static str, String)> {
    let (family, label) = match name {
        "noteheadBlack" => ("note", "note.black".into()),
        "noteheadHalf" => ("note", "note.half".into()),
        "noteheadWhole" => ("note", "note.whole".into()),
        "gClef" => ("clef", "clef.treble".into()),
        "fClef" => ("clef", "clef.bass".into()),
        "restWhole" => ("rest", "rest.whole".into()),
        "restHalf" => ("rest", "rest.half".into()),
        "restQuarter" => ("rest", "rest.quarter".into()),
        "rest8th" => ("rest", "rest.eighth".into()),
        "rest16th" => ("rest", "rest.sixteenth".into()),
        "rest32nd" => ("rest", "rest.thirty_second".into()),
        "rest64th" => ("rest", "rest.sixty_fourth".into()),
        "dynamicPiano" => ("dynamic", "dynamic.p".into()),
        "dynamicForte" => ("dynamic", "dynamic.f".into()),
        "dynamicFortePiano" => ("dynamic", "dynamic.fp".into()),
        "dynamicForzando" => ("dynamic", "dynamic.fz".into()),
        // DoReMi's category denotes printed sfz (verified on the v1.0
        // Mendelssohn/Beethoven pages), unlike the similarly named SMuFL sf glyph.
        "dynamicSforzato" => ("dynamic", "dynamic.sfz".into()),
        "dynamicSforzatoFF" => ("dynamic", "dynamic.sffz".into()),
        "dynamicRinforzando2" => ("dynamic", "dynamic.rfz".into()),
        // These names are unambiguous whole dynamic markings in DoReMi.
        "dynamicPP" | "dynamicPPP" | "dynamicPPPP" | "dynamicMP" | "dynamicMF" | "dynamicFF"
        | "dynamicFFF" | "dynamicFFFF" => {
            ("dynamic", format!("dynamic.{}", name[7..].to_lowercase()))
        }
        _ => return None,
    };
    Some((family, label))
}

#[derive(Default)]
struct Node {
    class: String,
    coordinates: BTreeMap<String, f32>,
    data: BTreeMap<String, String>,
}

fn finish(node: Node, reference: &mut Reference) -> Result<(), String> {
    let Some((family, class)) = class(&node.class) else {
        *reference.ignored.entry(node.class).or_default() += 1;
        return Ok(());
    };
    let coordinate = |name| {
        node.coordinates
            .get(name)
            .copied()
            .ok_or_else(|| format!("Missing DoReMi coordinate {name} for {}", node.class))
    };
    let (left, top, width, height) = (
        coordinate("Left")?,
        coordinate("Top")?,
        coordinate("Width")?,
        coordinate("Height")?,
    );
    if ![left, top, width, height].iter().all(|n| n.is_finite())
        || left < 0.0
        || top < 0.0
        || width <= 0.0
        || height <= 0.0
    {
        return Err(format!("Invalid DoReMi bounding box for {}", node.class));
    }
    let mut object = Object::new(family, &class);
    object.bounds = Some(SymbolBounds {
        left,
        top,
        right: left + width,
        bottom: top + height,
    });
    if family == "note" {
        if let Some(value) = node.data.get("duration_beats") {
            let beats: f64 = value
                .parse()
                .map_err(|_| format!("Invalid duration_beats: {value}"))?;
            if !beats.is_finite() || beats <= 0.0 {
                return Err(format!("Invalid duration_beats: {value}"));
            }
            object
                .attributes
                .insert("duration_ticks".into(), (beats * 192.0).round().to_string());
        }
        if let (Some(step), Some(octave)) = (
            node.data.get("normalized_pitch_step"),
            node.data.get("pitch_octave"),
        ) {
            let letter = step
                .get(..1)
                .ok_or_else(|| format!("Invalid pitch step: {step}"))?;
            let step_index = ["C", "D", "E", "F", "G", "A", "B"]
                .iter()
                .position(|s| *s == letter)
                .ok_or_else(|| format!("Invalid pitch step: {step}"))?;
            let octave: i32 = octave
                .parse()
                .map_err(|_| format!("Invalid octave: {octave}"))?;
            if !(0..=9).contains(&octave) {
                return Err("Octave outside supported range".into());
            }
            object
                .attributes
                .insert("pitch".into(), (octave * 7 + step_index as i32).to_string());
            if let Some(midi) = node.data.get("midi_pitch_code") {
                let midi: i32 = midi
                    .parse()
                    .map_err(|_| format!("Invalid MIDI pitch: {midi}"))?;
                let natural = (octave + 1) * 12 + [0, 2, 4, 5, 7, 9, 11][step_index];
                object
                    .attributes
                    .insert("accidental".into(), (midi - natural).to_string());
            }
        }
    }
    reference.objects.push(object);
    Ok(())
}

pub fn parse(xml: &str) -> Result<Reference, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut node = None::<Node>;
    let mut reference = Reference::default();
    let mut pages = 0;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) if e.name().as_ref() == b"Page" => pages += 1,
            Event::Start(e) if e.name().as_ref() == b"Node" => {
                if node.is_some() {
                    return Err("Nested DoReMi Node".into());
                }
                node = Some(Node::default());
            }
            Event::Start(e) if node.is_some() => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match name.as_str() {
                    "ClassName" => {
                        node.as_mut().unwrap().class = reader
                            .read_text(e.name())
                            .map_err(|e| e.to_string())?
                            .into_owned()
                    }
                    "Top" | "Left" | "Width" | "Height" => {
                        let text = reader.read_text(e.name()).map_err(|e| e.to_string())?;
                        let value = text
                            .parse()
                            .map_err(|_| format!("Invalid {name}: {text}"))?;
                        node.as_mut().unwrap().coordinates.insert(name, value);
                    }
                    "DataItem" => {
                        let mut key = None;
                        for attribute in e.attributes() {
                            let attribute = attribute.map_err(|e| e.to_string())?;
                            if attribute.key.as_ref() == b"key" {
                                key = Some(
                                    attribute
                                        .decode_and_unescape_value(reader.decoder())
                                        .map_err(|e| e.to_string())?
                                        .into_owned(),
                                );
                            }
                        }
                        let text = reader
                            .read_text(e.name())
                            .map_err(|e| e.to_string())?
                            .into_owned();
                        if let Some(key) = key {
                            node.as_mut().unwrap().data.insert(key, text);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.name().as_ref() == b"Node" => {
                finish(
                    node.take().ok_or("DoReMi Node without start")?,
                    &mut reference,
                )?;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if node.is_some() || pages != 1 {
        return Err("Expected exactly one complete DoReMi Page per sample".into());
    }
    if reference.objects.is_empty() {
        return Err("No supported DoReMi symbols on page".into());
    }
    Ok(reference)
}

pub fn predict(score: &Score) -> Vec<Object> {
    let mut objects = Vec::new();
    for note in &score.notes {
        let class = match note.duration {
            Duration::Whole => "note.whole",
            Duration::Half => "note.half",
            _ => "note.black",
        };
        let mut object = Object::new("note", class);
        object.bounds = Some(primus::note_bounds(score, note.staff, note.x, note.y));
        object.attributes = BTreeMap::from([
            ("pitch".into(), note.pitch.to_string()),
            ("accidental".into(), note.accidental.to_string()),
            (
                "duration_ticks".into(),
                primus::duration_ticks(note.duration, note.dots, note.triplet).to_string(),
            ),
        ]);
        objects.push(object);
    }
    for rest in &score.rests {
        let mut object = Object::new(
            "rest",
            &format!("rest.{}", primus::duration_name(rest.duration)),
        );
        object.bounds = rest
            .bounds
            .clone()
            .or_else(|| Some(primus::note_bounds(score, rest.staff, rest.x, rest.y)));
        objects.push(object);
    }
    for staff in &score.staves {
        if let Some(bounds) = &staff.header.clef_bounds {
            let mut object = Object::new(
                "clef",
                match staff.header.clef {
                    Clef::Treble => "clef.treble",
                    Clef::Bass => "clef.bass",
                    Clef::Unknown => "clef.unknown",
                },
            );
            object.bounds = Some(bounds.clone());
            objects.push(object);
        }
    }
    for dynamic in &score.dynamics {
        let mut object = Object::new("dynamic", &format!("dynamic.{}", dynamic.kind.label()));
        object.bounds = Some(dynamic.symbol_bounds(score.staves[dynamic.staff].spacing()));
        objects.push(object);
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_page_coordinates_and_optional_musical_metadata() {
        let xml = r#"<Page pageIndex="0"><Nodes><Node><ClassName>noteheadBlack</ClassName><Left>10</Left><Top>20</Top><Width>8</Width><Height>6</Height><Data><DataItem key="duration_beats">0.333333</DataItem><DataItem key="pitch_octave">4</DataItem><DataItem key="normalized_pitch_step">F#</DataItem><DataItem key="midi_pitch_code">66</DataItem></Data></Node><Node><ClassName>beam</ClassName></Node></Nodes></Page>"#;
        let reference = parse(xml).unwrap();
        assert_eq!(reference.objects[0].attributes["duration_ticks"], "64");
        assert_eq!(reference.objects[0].attributes["pitch"], "31");
        assert_eq!(reference.objects[0].attributes["accidental"], "1");
        assert_eq!(reference.ignored["beam"], 1);
    }

    #[test]
    fn rejects_multi_page_or_invalid_geometry() {
        assert!(parse("<Pages><Page/><Page/></Pages>").is_err());
        assert!(parse("<Page><Node><ClassName>noteheadBlack</ClassName></Node></Page>").is_err());
    }

    #[test]
    fn dataset_dynamic_names_follow_printed_markings() {
        assert_eq!(class("dynamicSforzato").unwrap().1, "dynamic.sfz");
        assert_eq!(class("dynamicMP").unwrap().1, "dynamic.mp");
    }
}
