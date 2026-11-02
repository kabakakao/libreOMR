//! Conservative system grouping and positioned lyric proposals, independent of PDF input.
use crate::model::{
    Clef, Lyric, LyricPlacement, Score, StaffAssignment, Syllabic, SymbolBounds, TextGlyph,
};
use image::GrayImage;

pub(crate) fn complete_system_headers(image: &GrayImage, staves: &mut [crate::model::Staff]) {
    let mut layout = Score {
        staves: staves.to_vec(),
        ..Default::default()
    };
    assign_parts(image, &mut layout);
    for a in &layout.staff_assignments {
        if staves[a.staff].header.time_signature.is_some() {
            continue;
        }
        let peers: Vec<_> = layout
            .staff_assignments
            .iter()
            .filter(|b| b.system == a.system)
            .filter_map(|b| {
                layout.staves[b.staff]
                    .header
                    .time_signature
                    .map(|m| (b.staff, m))
            })
            .collect();
        if peers.len() < 2 || peers.iter().any(|(_, m)| *m != peers[0].1) {
            continue;
        }
        let (peer, meter) = peers[0];
        staves[a.staff].header.time_signature = Some(meter);
        if let Some(bounds) = &layout.staves[peer].header.time_bounds {
            let dx = staves[a.staff].left - layout.staves[peer].left;
            let dy = staves[a.staff].lines[0] - layout.staves[peer].lines[0];
            let mut count = 0;
            let mut matched = 0;
            for y in bounds.top.max(0.0) as u32..bounds.bottom.min(image.height() as f32) as u32 {
                for x in bounds.left.max(0.0) as u32..bounds.right.min(image.width() as f32) as u32
                {
                    if image.get_pixel(x, y)[0] >= 170 {
                        continue;
                    }
                    count += 1;
                    let tx = (x as f32 + dx).round() as i32;
                    let ty = (y as f32 + dy).round() as i32;
                    if (-1..=1).any(|oy| {
                        (-1..=1).any(|ox| {
                            let (x, y) = (tx + ox, ty + oy);
                            x >= 0
                                && y >= 0
                                && x < image.width() as i32
                                && y < image.height() as i32
                                && image.get_pixel(x as u32, y as u32)[0] < 170
                        })
                    }) {
                        matched += 1;
                    }
                }
            }
            if count > 0 && matched as f32 / count as f32 > 0.9 {
                staves[a.staff].header.time_bounds = Some(SymbolBounds {
                    left: bounds.left + dx,
                    right: bounds.right + dx,
                    top: bounds.top + dy,
                    bottom: bounds.bottom + dy,
                });
            }
        }
    }
}

pub fn assign_parts(image: &GrayImage, score: &mut Score) {
    score.staff_assignments.clear();
    let mut order: Vec<_> = (0..score.staves.len()).collect();
    order.sort_by(|&a, &b| score.staves[a].lines[0].total_cmp(&score.staves[b].lines[0]));
    let connected = |upper: usize, lower: usize| {
        let a = &score.staves[upper];
        let b = &score.staves[lower];
        let gap = a.spacing();
        if (a.left - b.left).abs() > gap * 2.0 {
            return false;
        }
        let top = a.lines[4].ceil().max(0.0) as u32;
        let bottom = b.lines[0].floor().min(image.height() as f32 - 1.0) as u32;
        if bottom <= top {
            return false;
        }
        ((a.left - gap * 1.5).max(0.0) as u32
            ..=(a.left + gap).min(image.width() as f32 - 1.0) as u32)
            .any(|x| {
                (top..=bottom)
                    .filter(|&y| image.get_pixel(x, y)[0] < 170)
                    .count() as f32
                    / (bottom - top + 1) as f32
                    > 0.85
            })
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for staff in order {
        if groups
            .last()
            .and_then(|g| g.last())
            .is_some_and(|&prior| connected(prior, staff))
        {
            groups.last_mut().unwrap().push(staff);
        } else {
            groups.push(vec![staff]);
        }
    }
    // Unconnected single-instrument rows retain the original continuous-part behavior.
    let count = groups.iter().map(Vec::len).max().unwrap_or(0);
    if count < 2 {
        return;
    }
    for (system, group) in groups.into_iter().enumerate() {
        for (part, staff) in group.into_iter().enumerate() {
            score.staff_assignments.push(StaffAssignment {
                staff,
                part,
                system,
                name: format!("Part {}", part + 1),
            });
        }
    }
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    bounds: SymbolBounds,
}

fn tokens(glyphs: &[&TextGlyph], gap: f32) -> Vec<Token> {
    let mut sorted = glyphs.to_vec();
    sorted.sort_by(|a, b| a.bounds.left.total_cmp(&b.bounds.left));
    let mut result: Vec<Token> = Vec::new();
    let mut separated = true;
    for glyph in sorted {
        if glyph.text.trim().is_empty() {
            separated = true;
            continue;
        }
        if !separated
            && let Some(last) = result.last_mut()
            && glyph.bounds.left - last.bounds.right < gap * 0.45
            && last.text != "-"
            && glyph.text != "-"
        {
            last.text.push_str(&glyph.text);
            last.bounds.right = last.bounds.right.max(glyph.bounds.right);
            last.bounds.top = last.bounds.top.min(glyph.bounds.top);
            last.bounds.bottom = last.bounds.bottom.max(glyph.bounds.bottom);
        } else {
            result.push(Token {
                text: glyph.text.clone(),
                bounds: glyph.bounds.clone(),
            });
        }
        separated = false;
    }
    result
}

fn lyric_instruction(text: &str) -> bool {
    matches!(
        text.trim_matches(['.', ':', ',', ';'])
            .to_lowercase()
            .as_str(),
        "p" | "pp"
            | "ppp"
            | "mp"
            | "mf"
            | "f"
            | "ff"
            | "fff"
            | "sfz"
            | "cresc"
            | "dim"
            | "rit"
            | "ritard"
            | "ritardando"
            | "riten"
            | "ritenuto"
            | "rall"
            | "rallentando"
            | "accel"
            | "accelerando"
            | "allarg"
            | "allargando"
            | "marc"
            | "marcato"
            | "stacc"
            | "staccato"
            | "legato"
            | "tenuto"
            | "espr"
            | "espressivo"
            | "crescendo"
            | "diminuendo"
            | "decresc"
            | "decrescendo"
            | "smorzando"
            | "morendo"
            | "rubato"
            | "cantabile"
            | "sostenuto"
            | "sotto"
            | "voce"
            | "tempo"
            | "dolce"
            | "refrain"
            | "chorus"
            | "bongos"
    )
}

/// Recognize complete direction phrases, including punctuation joined by OCR.
/// Modifiers such as "a" and "e" are excluded only beside an actual direction.
fn instruction_words(words: &[&Token], gap: f32) -> Vec<bool> {
    let kinds: Vec<_> = words
        .iter()
        .map(|token| {
            let text = token.text.to_lowercase();
            let parts: Vec<_> = text
                .split(|c: char| !c.is_alphabetic())
                .filter(|part| !part.is_empty())
                .collect();
            let core = parts.iter().any(|part| lyric_instruction(part));
            let candidate = !parts.is_empty()
                && parts.iter().all(|part| {
                    lyric_instruction(part)
                        || matches!(
                            *part,
                            "e" | "ed"
                                | "a"
                                | "al"
                                | "in"
                                | "con"
                                | "di"
                                | "non"
                                | "poco"
                                | "molto"
                                | "piu"
                                | "più"
                                | "meno"
                                | "sempre"
                                | "subito"
                                | "ma"
                                | "troppo"
                                | "primo"
                                | "giusto"
                        )
                });
            (candidate, core)
        })
        .collect();
    let mut excluded = vec![false; words.len()];
    let mut start = 0;
    while start < words.len() {
        if !kinds[start].0 {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < words.len()
            && kinds[end].0
            && words[end].bounds.left - words[end - 1].bounds.right < gap * 4.5
        {
            end += 1;
        }
        if kinds[start..end].iter().any(|(_, core)| *core) {
            excluded[start..end].fill(true);
        }
        start = end;
    }
    excluded
}

fn publication_row(text: &str) -> bool {
    let text = text.to_lowercase();
    let words: Vec<_> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|word| !word.is_empty())
        .collect();
    text.contains(['©', '℗'])
        || text.contains("www.")
        || text.contains("http:")
        || text.contains("https:")
        || words.iter().any(|word| {
            matches!(
                *word,
                "copyright"
                    | "verlag"
                    | "musikverlag"
                    | "publisher"
                    | "publishing"
                    | "isbn"
                    | "ismn"
            )
        })
        || words.windows(3).any(|phrase| {
            phrase == ["all", "rights", "reserved"] || phrase == ["alle", "rechte", "vorbehalten"]
        })
}

/// Inspect full-width rows before cropping to the staff: a copyright mark or
/// publisher name can sit outside the notation margin. Keep all source glyphs.
fn footer_top(score: &Score) -> f32 {
    let Some(staff) = score
        .staves
        .iter()
        .max_by(|a, b| a.lines[4].total_cmp(&b.lines[4]))
    else {
        return f32::INFINITY;
    };
    let gap = staff.spacing();
    let mut glyphs: Vec<_> = score
        .source_text
        .iter()
        .filter(|g| g.baseline > staff.lines[4] + gap * 0.7)
        .collect();
    glyphs.sort_by(|a, b| a.baseline.total_cmp(&b.baseline));
    let mut start = 0;
    while start < glyphs.len() {
        let mut end = start + 1;
        while end < glyphs.len() && glyphs[end].baseline - glyphs[start].baseline < gap * 0.6 {
            end += 1;
        }
        let text = tokens(&glyphs[start..end], gap)
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if publication_row(&text) {
            return glyphs[start].baseline - gap * 0.6;
        }
        start = end;
    }
    f32::INFINITY
}

pub fn attach_text(score: &mut Score, text: Vec<TextGlyph>) {
    score.source_text = text;
    score.lyrics.clear();
    // Read part names left of the first system. Later systems use the same part ID.
    let mut names = std::collections::HashMap::new();
    for assignment in &score.staff_assignments {
        let staff = &score.staves[assignment.staff];
        let glyphs: Vec<_> = score
            .source_text
            .iter()
            .filter(|g| {
                g.bounds.right < staff.left - staff.spacing() * 0.3
                    && g.baseline > staff.lines[0] - staff.spacing()
                    && g.baseline < staff.lines[4] + staff.spacing()
            })
            .collect();
        let name = tokens(&glyphs, staff.spacing())
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() && name.chars().any(char::is_alphabetic) {
            names.entry(assignment.part).or_insert(name);
        }
    }
    for assignment in &mut score.staff_assignments {
        if let Some(name) = names.get(&assignment.part) {
            assignment.name = name.clone();
        }
    }
    let footer_top = footer_top(score);
    let mut consumed = std::collections::HashSet::new();
    for placement in [LyricPlacement::Below, LyricPlacement::Above] {
        for staff_index in 0..score.staves.len() {
            let staff = &score.staves[staff_index];
            let gap = staff.spacing();
            let next_top = score
                .staves
                .iter()
                .filter(|s| s.lines[0] > staff.lines[4])
                .map(|s| s.lines[0])
                .fold(f32::INFINITY, f32::min);
            let bottom = (staff.lines[4] + gap * 18.0).min(next_top - gap * 1.2);
            let mut glyphs: Vec<_> = score
                .source_text
                .iter()
                .enumerate()
                .filter(|(index, g)| {
                    !consumed.contains(index)
                        && g.baseline < footer_top
                        && (if placement == LyricPlacement::Below {
                            g.baseline > staff.lines[4] + gap * 0.7 && g.baseline < bottom
                        } else {
                            g.baseline < staff.lines[0] - gap * 0.7
                                && g.baseline > staff.lines[0] - gap * 3.5
                        })
                        && g.bounds.left >= staff.left - gap * 1.5
                        && g.bounds.right <= staff.right + gap
                })
                .collect();
            glyphs.sort_by(|a, b| a.1.baseline.total_cmp(&b.1.baseline));
            let mut rows: Vec<Vec<(usize, &TextGlyph)>> = Vec::new();
            for glyph in glyphs {
                if rows.last().is_some_and(|row| {
                    (row.iter().map(|(_, g)| g.baseline).sum::<f32>() / row.len() as f32
                        - glyph.1.baseline)
                        .abs()
                        < gap * 0.6
                }) {
                    rows.last_mut().unwrap().push(glyph);
                } else {
                    rows.push(vec![glyph]);
                }
            }
            let mut notes: Vec<_> = score
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.staff == staff_index && !n.grace)
                .collect();
            notes.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
            let mut onsets: Vec<(usize, &crate::model::Note)> = Vec::new();
            for note in notes {
                if let Some(last) = onsets.last_mut()
                    && (note.1.x - last.1.x).abs() < gap * 0.35
                {
                    if (placement == LyricPlacement::Above && note.1.y < last.1.y)
                        || (placement == LyricPlacement::Below && note.1.y > last.1.y)
                    {
                        *last = note;
                    }
                } else {
                    onsets.push(note);
                }
            }
            let notes = onsets;
            // Several left-aligned, numbered prose rows below the last system are
            // a standalone stanza block, not syllables under successive notes.
            let numbered: Vec<_> = rows
                .iter()
                .filter_map(|row| {
                    let glyphs: Vec<_> = row.iter().map(|(_, g)| *g).collect();
                    let values = tokens(&glyphs, gap);
                    let first_word = values
                        .iter()
                        .position(|t| t.text.chars().any(char::is_alphabetic))?;
                    let prefix: String = values[..first_word]
                        .iter()
                        .map(|t| t.text.as_str())
                        .collect();
                    let number = prefix.strip_suffix('.')?.parse::<u16>().ok()?;
                    let baseline =
                        row.iter().map(|(_, g)| g.baseline).sum::<f32>() / row.len() as f32;
                    Some((number, values.last()?.bounds.right, baseline))
                })
                .collect();
            if placement == LyricPlacement::Below
                && next_top.is_infinite()
                && numbered.len() >= 2
                && numbered[0].0 > 1
                && numbered
                    .iter()
                    .all(|(_, right, _)| *right < staff.left + (staff.right - staff.left) * 0.65)
            {
                let stanza_top = numbered[0].2;
                rows.retain(|row| {
                    row.iter().map(|(_, g)| g.baseline).sum::<f32>() / (row.len() as f32)
                        < stanza_top - gap * 0.6
                });
            }
            let mut next_verse = 1_u16;
            let mut used_verses = std::collections::HashSet::new();
            let mut previous_baseline = None;
            for row in rows {
                let baseline = row.iter().map(|(_, g)| g.baseline).sum::<f32>() / row.len() as f32;
                if placement == LyricPlacement::Below
                    && previous_baseline.map_or(baseline - staff.lines[4] > gap * 8.0, |prior| {
                        baseline - prior > gap * 3.2
                    })
                {
                    continue;
                }
                let glyphs: Vec<_> = row.iter().map(|(_, g)| *g).collect();
                let row_tokens = tokens(&glyphs, gap);
                let words: Vec<_> = row_tokens
                    .iter()
                    .filter(|t| t.text.chars().any(char::is_alphabetic))
                    .collect();
                let instructions = instruction_words(&words, gap);
                if words.is_empty()
                    || words.iter().zip(&instructions).all(|(t, instruction)| {
                        (t.text.chars().count() == 1
                            && t.text.chars().all(char::is_uppercase)
                            && !matches!(t.text.as_str(), "A" | "E" | "I" | "O" | "U"))
                            || *instruction
                    })
                {
                    continue;
                }
                // A printed number is a verse label, never a syllable or an extra note.
                let first_word = row_tokens
                    .iter()
                    .position(|t| t.text.chars().any(char::is_alphabetic))
                    .unwrap();
                let prefix: String = row_tokens[..first_word]
                    .iter()
                    .map(|t| t.text.trim())
                    .collect();
                let printed_verse = prefix
                    .strip_suffix('.')
                    .or_else(|| prefix.strip_suffix(')'))
                    .and_then(|number| number.parse::<u16>().ok())
                    .filter(|&number| number > 0);
                let verse = printed_verse.unwrap_or(next_verse);
                if used_verses.contains(&verse) {
                    continue;
                }
                let words: Vec<_> = words
                    .into_iter()
                    .zip(instructions)
                    .filter(|(t, instruction)| {
                        !instruction && !t.text.to_lowercase().starts_with("refrain:")
                    })
                    .map(|(token, _)| token)
                    .collect();
                // Align the complete row in order. Greedily taking the nearest
                // note can consume the final note before its syllable, notably
                // with closely spaced, left-aligned above-staff vowels.
                let columns = notes.len() + 1;
                let mut cost = vec![0.0; (words.len() + 1) * columns];
                let mut action = vec![0_u8; cost.len()];
                for i in 1..=words.len() {
                    cost[i * columns] = i as f32 * gap * 3.0;
                    for j in 1..=notes.len() {
                        let index = i * columns + j;
                        cost[index] = cost[index - 1];
                        let skip = cost[index - columns] + gap * 3.0;
                        if skip < cost[index] {
                            cost[index] = skip;
                            action[index] = 1;
                        }
                        let token = words[i - 1];
                        let distance = (notes[j - 1].1.x
                            - (token.bounds.left + token.bounds.right) * 0.5)
                            .abs();
                        let matched = cost[index - columns - 1] + distance;
                        if distance < gap * 3.0 && matched <= cost[index] {
                            cost[index] = matched;
                            action[index] = 2;
                        }
                    }
                }
                let mut assigned: Vec<(usize, &Token)> = Vec::new();
                let (mut i, mut j) = (words.len(), notes.len());
                while i > 0 && j > 0 {
                    match action[i * columns + j] {
                        2 => {
                            assigned.push((notes[j - 1].0, words[i - 1]));
                            i -= 1;
                            j -= 1;
                        }
                        1 => i -= 1,
                        _ => j -= 1,
                    }
                }
                assigned.reverse();
                if assigned.is_empty() {
                    continue;
                }
                used_verses.insert(verse);
                next_verse = verse.saturating_add(1);
                consumed.extend(row.iter().map(|(index, _)| *index));
                previous_baseline = Some(baseline);
                for (position, &(note, token)) in assigned.iter().enumerate() {
                    let before = assigned
                        .get(position.wrapping_sub(1))
                        .map_or(staff.left, |p| p.1.bounds.right);
                    let after = assigned
                        .get(position + 1)
                        .map_or(staff.right, |p| p.1.bounds.left);
                    let hyphen_before = row_tokens.iter().any(|t| {
                        t.text == "-"
                            && t.bounds.left >= before
                            && t.bounds.right <= token.bounds.left
                    });
                    let hyphen_after = row_tokens.iter().any(|t| {
                        t.text == "-"
                            && t.bounds.left >= token.bounds.right
                            && t.bounds.right <= after
                    });
                    let syllabic = match (hyphen_before, hyphen_after) {
                        (false, false) => Syllabic::Single,
                        (false, true) => Syllabic::Begin,
                        (true, true) => Syllabic::Middle,
                        (true, false) => Syllabic::End,
                    };
                    let extend = row_tokens.iter().any(|t| {
                        t.text.contains('_')
                            && t.bounds.left >= token.bounds.right
                            && t.bounds.right <= after
                    });
                    score.lyrics.push(Lyric {
                        placement,
                        note,
                        verse,
                        verse_label: (position == 0 && printed_verse.is_some())
                            .then(|| prefix.clone()),
                        text: token.text.clone(),
                        syllabic,
                        extend,
                        confirmed: false,
                        bounds: Some(token.bounds.clone()),
                    });
                }
            }
        }
    }
}

impl Score {
    pub fn remove_note(&mut self, index: usize) {
        // An attached span must never silently jump to a different note.
        self.spans.retain(|span| {
            ![&span.start_anchor, &span.end_anchor]
                .into_iter()
                .flatten()
                .any(|a| a.note == index)
        });
        for span in &mut self.spans {
            for anchor in [&mut span.start_anchor, &mut span.end_anchor]
                .into_iter()
                .flatten()
            {
                if anchor.note > index {
                    anchor.note -= 1;
                }
            }
        }
        self.notes.remove(index);
        self.lyrics.retain(|l| l.note != index);
        for lyric in &mut self.lyrics {
            if lyric.note > index {
                lyric.note -= 1;
            }
        }
    }

    /// Carry attributes by part identity, never from a neighboring simultaneous part.
    pub fn inherit_part_headers(&mut self, previous: &Score) {
        for assignment in self.staff_assignments.clone() {
            let prior = previous
                .staff_assignments
                .iter()
                .filter(|a| a.part == assignment.part)
                .max_by(|a, b| {
                    a.system.cmp(&b.system).then(
                        previous.staves[a.staff].lines[0]
                            .total_cmp(&previous.staves[b.staff].lines[0]),
                    )
                });
            if let Some(prior) = prior {
                if self
                    .staff_assignments
                    .iter()
                    .find(|a| a.staff == assignment.staff)
                    .is_some_and(|a| a.name.starts_with("Part "))
                {
                    self.staff_assignments
                        .iter_mut()
                        .find(|a| a.staff == assignment.staff)
                        .unwrap()
                        .name = prior.name.clone();
                }
                let source = &previous.staves[prior.staff].header;
                let mut target = self.staves[assignment.staff].header.clone();
                if target.time_signature.is_none() {
                    target.time_signature =
                        previous.time_signature_before(prior.staff, f32::INFINITY);
                }
                if target.key_signature.is_none() {
                    target.key_signature = previous
                        .key_changes
                        .iter()
                        .filter(|c| c.staff == prior.staff)
                        .max_by(|a, b| a.x.total_cmp(&b.x))
                        .map(|c| c.key_signature)
                        .or(source.key_signature);
                }
                if target.clef == Clef::Unknown {
                    target.clef = source.clef;
                }
                self.set_staff_header(assignment.staff, target);
            }
        }
        self.recompute_accidentals();
    }
}
