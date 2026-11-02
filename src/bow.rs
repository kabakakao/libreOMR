use crate::model::{NoteAnchor, Score, Span, SpanKind};

impl Span {
    pub fn is_bow(&self) -> bool {
        matches!(self.kind, SpanKind::Slur | SpanKind::Tie)
    }

    pub fn flip(&mut self) {
        if self.is_bow() {
            self.height = -self.height;
            if let Some(continuation) = &mut self.continuation {
                continuation.exit_y = 4.0 - continuation.exit_y;
                continuation.entry_y = 4.0 - continuation.entry_y;
                continuation.exit_height = -continuation.exit_height;
                continuation.entry_height = -continuation.entry_height;
            }
            for (point, anchor) in [
                (&mut self.start, &mut self.start_anchor),
                (&mut self.end, &mut self.end_anchor),
            ] {
                if let Some(anchor) = anchor {
                    point[1] -= 2.0 * anchor.offset[1];
                    anchor.offset[1] = -anchor.offset[1];
                }
            }
            self.confirmed = true;
        }
    }
}

impl Score {
    /// Chord tones at either endpoint share an onset and do not interrupt a tie.
    pub fn tie_adjacent(&self, first: usize, last: usize) -> bool {
        let (Some(a), Some(b)) = (self.notes.get(first), self.notes.get(last)) else {
            return false;
        };
        if a.voice != b.voice || a.pitch != b.pitch || !self.bow_note_order(first, last) {
            return false;
        }
        let rows = self.bow_staves(a.staff);
        let Some(left) = rows.iter().position(|&s| s == a.staff) else {
            return false;
        };
        let Some(right) = rows.iter().position(|&s| s == b.staff) else {
            return false;
        };
        let between = |staff: usize, x: f32, tolerance: bool| {
            let margin = if tolerance {
                self.staves[staff].spacing() * 0.35
            } else {
                0.0
            };
            rows.iter().position(|&s| s == staff).is_some_and(|i| {
                i >= left
                    && i <= right
                    && (i != left || x > a.x + margin)
                    && (i != right || x < b.x - margin)
            })
        };
        !self
            .notes
            .iter()
            .any(|n| n.voice == a.voice && between(n.staff, n.x, true))
            && !self
                .rests
                .iter()
                .any(|r| r.voice == a.voice && between(r.staff, r.x, false))
    }

    pub fn tie_compatible(&self, first: usize, last: usize) -> bool {
        if !self.tie_adjacent(first, last) {
            return false;
        }
        let a = &self.notes[first];
        let b = &self.notes[last];
        a.accidental == b.accidental
            || (!b.accidental_explicit
                && (a.staff != b.staff
                    || self
                        .barlines
                        .iter()
                        .any(|bar| bar.staff == a.staff && bar.x > a.x && bar.x <= b.x)))
    }

    pub(crate) fn carry_tie_accidentals(&mut self) {
        let mut rows: Vec<_> = (0..self.staves.len()).collect();
        rows.sort_by(|&a, &b| {
            let system = |staff| {
                self.staff_assignments
                    .iter()
                    .find(|a| a.staff == staff)
                    .map_or(0, |a| a.system)
            };
            system(a)
                .cmp(&system(b))
                .then(self.staves[a].lines[0].total_cmp(&self.staves[b].lines[0]))
        });
        let mut links: Vec<_> = self
            .spans
            .iter()
            .filter(|s| s.kind == SpanKind::Tie)
            .filter_map(|s| Some((s.start_anchor.as_ref()?.note, s.end_anchor.as_ref()?.note)))
            .filter(|&(a, b)| self.tie_compatible(a, b))
            .collect();
        links.sort_by(|&(a, _), &(b, _)| {
            rows.iter()
                .position(|&s| s == self.notes[a].staff)
                .cmp(&rows.iter().position(|&s| s == self.notes[b].staff))
                .then(self.notes[a].x.total_cmp(&self.notes[b].x))
        });
        for (first, last) in links {
            if !self.notes[last].accidental_explicit {
                self.notes[last].accidental = self.notes[first].accidental;
            }
        }
    }

    /// Ordered rows of the same part. Simultaneous staves must never be joined.
    pub fn bow_staves(&self, staff: usize) -> Vec<usize> {
        let assignment = self.staff_assignments.iter().find(|a| a.staff == staff);
        let mut rows: Vec<_> = (0..self.staves.len())
            .filter(|&index| match assignment {
                Some(a) => self
                    .staff_assignments
                    .iter()
                    .any(|b| b.staff == index && b.part == a.part),
                None => self.staff_assignments.is_empty() || index == staff,
            })
            .collect();
        rows.sort_by(|&a, &b| {
            let system = |staff| {
                self.staff_assignments
                    .iter()
                    .find(|a| a.staff == staff)
                    .map_or(0, |a| a.system)
            };
            system(a)
                .cmp(&system(b))
                .then(self.staves[a].lines[0].total_cmp(&self.staves[b].lines[0]))
        });
        rows
    }

    pub fn bow_note_order(&self, first: usize, last: usize) -> bool {
        let (Some(a), Some(b)) = (self.notes.get(first), self.notes.get(last)) else {
            return false;
        };
        if a.staff == b.staff {
            return a.x < b.x;
        }
        if let (Some(a), Some(b)) = (
            self.staff_assignments.iter().find(|s| s.staff == a.staff),
            self.staff_assignments.iter().find(|s| s.staff == b.staff),
        ) && a.system == b.system
        {
            return false;
        }
        let rows = self.bow_staves(a.staff);
        matches!((rows.iter().position(|&s| s == a.staff), rows.iter().position(|&s| s == b.staff)), (Some(a), Some(b)) if a < b)
    }

    pub fn resolved_span(&self, span: &Span) -> Span {
        let mut resolved = span.clone();
        for (point, anchor) in [
            (&mut resolved.start, &span.start_anchor),
            (&mut resolved.end, &span.end_anchor),
        ] {
            if let Some(anchor) = anchor
                && let Some(note) = self.notes.get(anchor.note)
            {
                *point = [note.x + anchor.offset[0], note.y + anchor.offset[1]];
            }
        }
        resolved
    }

    pub fn attach_span_note(&self, span: &mut Span, right: bool, note: usize) -> bool {
        if self.notes.get(note).is_none() {
            return false;
        }
        let other = if right {
            span.start_anchor.as_ref()
        } else {
            span.end_anchor.as_ref()
        };
        if let Some(other) = other
            && !if right {
                self.bow_note_order(other.note, note)
            } else {
                self.bow_note_order(note, other.note)
            }
        {
            return false;
        }
        let target = &self.notes[note];
        if other.is_none() && target.staff != span.staff {
            return false;
        }
        if other.is_none()
            && if right {
                target.x <= span.start[0]
            } else {
                target.x >= span.end[0]
            }
        {
            return false;
        }
        let old = if right {
            &span.end_anchor
        } else {
            &span.start_anchor
        };
        let new_note = old.as_ref().is_none_or(|a| a.note != note);
        let mut offset = old.as_ref().map_or(
            [
                0.0,
                if span.is_bow() {
                    span.height.signum() * self.staves[target.staff].spacing() * 0.65
                } else {
                    let source = &self.staves[span.staff];
                    let level = ((if right { span.end[1] } else { span.start[1] })
                        - source.lines[0])
                        / source.spacing();
                    self.staves[target.staff].lines[0] + level * self.staves[target.staff].spacing()
                        - target.y
                },
            ],
            |a| a.offset,
        );
        let staff = &self.staves[target.staff];
        let at = [
            (target.x + offset[0]).clamp(staff.left, staff.right),
            target.y + offset[1],
        ];
        offset[0] = at[0] - target.x;
        let resolved = self.resolved_span(span);
        let other_staff = other.map_or(span.staff, |a| self.notes[a.note].staff);
        let other_x = if right {
            resolved.start[0]
        } else {
            resolved.end[0]
        };
        if target.staff == other_staff
            && if right {
                at[0] <= other_x
            } else {
                at[0] >= other_x
            }
        {
            return false;
        }
        let anchor = NoteAnchor { note, offset };
        if new_note {
            span.continuation = None;
        }
        if right {
            span.end_anchor = Some(anchor);
            span.end = at;
        } else {
            span.start_anchor = Some(anchor);
            span.start = at;
            span.staff = target.staff;
        }
        span.confirmed = true;
        true
    }

    pub(crate) fn span_entry_x(&self, staff: usize) -> f32 {
        let staff = &self.staves[staff];
        [&staff.header.clef_bounds, &staff.header.time_bounds]
            .into_iter()
            .flatten()
            .chain(staff.header.key_bounds.iter())
            .map(|b| b.right + staff.spacing() * 0.5)
            .fold(staff.left, f32::max)
            .min(staff.right - staff.spacing() * 0.25)
    }

    /// Split a musical span into drawable fragments at system breaks.
    pub fn span_segments(&self, span: &Span) -> Vec<Span> {
        let resolved = self.resolved_span(span);
        let end_staff = span
            .end_anchor
            .as_ref()
            .and_then(|a| self.notes.get(a.note))
            .map_or(span.staff, |n| n.staff);
        if end_staff == span.staff {
            return vec![resolved];
        }
        let rows = self.bow_staves(span.staff);
        let (Some(first), Some(last)) = (
            rows.iter().position(|&s| s == span.staff),
            rows.iter().position(|&s| s == end_staff),
        ) else {
            return vec![];
        };
        if first >= last {
            return vec![];
        }
        if !span.is_bow() {
            let start_staff = &self.staves[span.staff];
            let final_staff = &self.staves[end_staff];
            let start_level = (resolved.start[1] - start_staff.lines[0]) / start_staff.spacing();
            let end_level = (resolved.end[1] - final_staff.lines[0]) / final_staff.spacing();
            let mut pieces: Vec<_> = rows[first..=last]
                .iter()
                .map(|&index| {
                    let staff = &self.staves[index];
                    let mut piece = resolved.clone();
                    piece.staff = index;
                    piece.start[0] = if index == span.staff {
                        resolved.start[0]
                    } else {
                        self.span_entry_x(index)
                    };
                    piece.end[0] = if index == end_staff {
                        resolved.end[0]
                    } else {
                        staff.right
                    };
                    piece.start_anchor = if index == span.staff {
                        span.start_anchor.clone()
                    } else {
                        None
                    };
                    piece.end_anchor = if index == end_staff {
                        span.end_anchor.clone()
                    } else {
                        None
                    };
                    piece.height = span.height * staff.spacing() / start_staff.spacing();
                    piece
                })
                .collect();
            if pieces.len() == 2
                && let Some(c) = &span.continuation
            {
                pieces[0].end[0] = start_staff.right + c.exit_x * start_staff.spacing();
                pieces[1].start[0] = final_staff.left + c.entry_x * final_staff.spacing();
            }
            let total: f32 = pieces.iter().map(|s| s.end[0] - s.start[0]).sum();
            if total <= 0.0 {
                return vec![];
            }
            let mut distance = 0.0;
            let [opening_start, opening_end] =
                span.wedge_openings
                    .unwrap_or(if span.kind == SpanKind::Crescendo {
                        [0.0, 1.0]
                    } else {
                        [1.0, 0.0]
                    });
            for piece in &mut pieces {
                let staff = &self.staves[piece.staff];
                let start = distance / total;
                distance += piece.end[0] - piece.start[0];
                let end = distance / total;
                piece.start[1] = staff.lines[0]
                    + staff.spacing() * (start_level + (end_level - start_level) * start);
                piece.end[1] = staff.lines[0]
                    + staff.spacing() * (start_level + (end_level - start_level) * end);
                piece.wedge_openings = Some([
                    opening_start + (opening_end - opening_start) * start,
                    opening_start + (opening_end - opening_start) * end,
                ]);
            }
            if pieces.len() == 2
                && let Some(c) = &span.continuation
            {
                pieces[0].end[1] = start_staff.lines[0] + c.exit_y * start_staff.spacing();
                pieces[1].start[1] = final_staff.lines[0] + c.entry_y * final_staff.spacing();
                pieces[0].wedge_openings.as_mut().unwrap()[1] =
                    c.exit_height * start_staff.spacing() / pieces[0].height;
                pieces[1].wedge_openings.as_mut().unwrap()[0] =
                    c.entry_height * final_staff.spacing() / pieces[1].height;
            }
            return pieces;
        }
        rows[first..=last]
            .iter()
            .map(|&index| {
                let staff = &self.staves[index];
                let gap = staff.spacing();
                let first_staff = &self.staves[span.staff];
                let last_staff = &self.staves[end_staff];
                let level = ((resolved.start[1] - first_staff.lines[0]) / first_staff.spacing()
                    + (resolved.end[1] - last_staff.lines[0]) / last_staff.spacing())
                    * 0.5
                    + span.height.signum();
                let edge_y = staff.lines[0] + gap * level;
                let mut segment = resolved.clone();
                segment.staff = index;
                if index != span.staff {
                    segment.start = [staff.left, edge_y];
                    segment.start_anchor = None;
                }
                if index != end_staff {
                    segment.end = [staff.right, edge_y];
                    segment.end_anchor = None;
                }
                segment.height = span.height.signum()
                    * span
                        .height
                        .abs()
                        .min((segment.end[0] - segment.start[0]).max(0.0) * 0.16);
                if last == first + 1
                    && let Some(c) = &span.continuation
                {
                    if index == span.staff {
                        segment.end[0] = staff.right + c.exit_x * gap;
                        segment.end[1] = staff.lines[0] + c.exit_y * gap;
                        segment.height = c.exit_height * gap;
                    } else {
                        segment.start[0] = staff.left + c.entry_x * gap;
                        segment.start[1] = staff.lines[0] + c.entry_y * gap;
                        segment.height = c.entry_height * gap;
                    }
                }
                segment
            })
            .collect()
    }

    pub fn drawable_spans(&self) -> Vec<(usize, Span)> {
        self.spans
            .iter()
            .enumerate()
            .flat_map(|(index, span)| {
                self.span_segments(span)
                    .into_iter()
                    .map(move |s| (index, s))
            })
            .collect()
    }

    pub(crate) fn valid_span_anchors(&self, span: &Span) -> bool {
        if span.staff >= self.staves.len() {
            return false;
        }
        if let Some(c) = &span.continuation
            && (span.start_anchor.is_none()
                || span.end_anchor.is_none()
                || [
                    c.exit_x,
                    c.entry_x,
                    c.exit_y,
                    c.entry_y,
                    c.exit_height,
                    c.entry_height,
                ]
                .iter()
                .any(|v| !v.is_finite())
                || (!span.is_bow() && (c.exit_height < 0.0 || c.entry_height < 0.0)))
        {
            return false;
        }
        for anchor in [&span.start_anchor, &span.end_anchor].into_iter().flatten() {
            if self.notes.get(anchor.note).is_none()
                || self
                    .notes
                    .get(anchor.note)
                    .is_some_and(|n| n.staff >= self.staves.len())
                || anchor.offset.iter().any(|v| !v.is_finite())
            {
                return false;
            }
        }
        if let Some(openings) = span.wedge_openings
            && (span.is_bow()
                || openings
                    .iter()
                    .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x)))
        {
            return false;
        }
        if span
            .start_anchor
            .as_ref()
            .is_some_and(|a| self.notes[a.note].staff != span.staff)
        {
            return false;
        }
        if span.continuation.is_some() {
            let end_staff = self.notes[span.end_anchor.as_ref().unwrap().note].staff;
            let rows = self.bow_staves(span.staff);
            if !rows.windows(2).any(|pair| pair == [span.staff, end_staff]) {
                return false;
            }
            if !span.is_bow()
                && let Some(c) = &span.continuation
            {
                let maximum = span.height / self.staves[span.staff].spacing() + 0.0001;
                if c.exit_height > maximum || c.entry_height > maximum {
                    return false;
                }
            }
        }
        match (&span.start_anchor, &span.end_anchor) {
            (Some(a), Some(b)) => self.bow_note_order(a.note, b.note),
            (_, Some(b)) => self.notes[b.note].staff == span.staff,
            _ => true,
        }
    }
}
