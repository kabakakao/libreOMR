use serde::{Deserialize, Serialize};

// Integral even for double-dotted 64th-note triplets.
pub(crate) const TICKS_PER_QUARTER: u32 = 192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clef {
    #[default]
    Treble,
    Bass,
    Unknown,
}

impl Clef {
    pub const ALL: [Self; 3] = [Self::Unknown, Self::Treble, Self::Bass];

    pub fn label(self) -> &'static str {
        match self {
            Self::Treble => "Violinschluessel",
            Self::Bass => "Bassschluessel",
            Self::Unknown => "Unbekannt",
        }
    }

    pub fn bottom_pitch(self) -> i32 {
        match self {
            Self::Bass => 18,
            Self::Treble | Self::Unknown => 30,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl Duration {
    pub(crate) fn dotted_ticks(self, dots: u8) -> u32 {
        let base = self.ticks();
        match dots {
            1 => base * 3 / 2,
            2 => base * 7 / 4,
            _ => base,
        }
    }

    pub(crate) fn ticks(self) -> u32 {
        let divisor = match self {
            Self::Whole => 1,
            Self::Half => 2,
            Self::Quarter => 4,
            Self::Eighth => 8,
            Self::Sixteenth => 16,
            Self::ThirtySecond => 32,
            Self::SixtyFourth => 64,
        };
        TICKS_PER_QUARTER * 4 / divisor
    }

    pub const ALL: [Self; 7] = [
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
        Self::SixtyFourth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::SixtyFourth => "1/64",
        }
    }

    pub fn flag_count(self) -> usize {
        match self {
            Self::Whole | Self::Half | Self::Quarter => 0,
            Self::Eighth => 1,
            Self::Sixteenth => 2,
            Self::ThirtySecond => 3,
            Self::SixtyFourth => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Articulation {
    None,
    Staccato,
    Accent,
    Tenuto,
    Fermata,
    Marcato,
    Staccatissimo,
    Portato,
    AccentStaccato,
    TenutoAccent,
    MarcatoStaccato,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySignature {
    pub fifths: i8,
}

impl KeySignature {
    pub fn accidental_at(self, pitch: i32) -> i8 {
        let order = if self.fifths >= 0 {
            [3, 0, 4, 1, 5, 2, 6]
        } else {
            [6, 2, 5, 1, 4, 0, 3]
        };
        if order[..usize::from(self.fifths.unsigned_abs().min(7))].contains(&pitch.rem_euclid(7)) {
            self.fifths.signum()
        } else {
            0
        }
    }

    pub fn label(self) -> String {
        let names = [
            "Ces-Dur / as-Moll",
            "Ges-Dur / es-Moll",
            "Des-Dur / b-Moll",
            "As-Dur / f-Moll",
            "Es-Dur / c-Moll",
            "B-Dur / g-Moll",
            "F-Dur / d-Moll",
            "C-Dur / a-Moll",
            "G-Dur / e-Moll",
            "D-Dur / h-Moll",
            "A-Dur / fis-Moll",
            "E-Dur / cis-Moll",
            "H-Dur / gis-Moll",
            "Fis-Dur / dis-Moll",
            "Cis-Dur / ais-Moll",
        ];
        names[(self.fifths.clamp(-7, 7) + 7) as usize].into()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rest {
    pub x: f32,
    pub y: f32,
    pub staff: usize,
    pub duration: Duration,
    #[serde(default)]
    pub dots: u8,
    #[serde(default)]
    pub measures: Option<u8>,
    pub confirmed: bool,
    pub bounds: Option<SymbolBounds>,
}

impl Rest {
    pub(crate) fn ticks(&self, divisions: u128) -> u128 {
        u128::from(self.duration.dotted_ticks(self.dots)) * divisions
            / u128::from(TICKS_PER_QUARTER)
    }

    pub fn duration_label(&self) -> String {
        self.measures.map_or_else(
            || {
                format!(
                    "{}{}",
                    self.duration.label(),
                    ".".repeat(usize::from(self.dots))
                )
            },
            |count| format!("{count} Takte"),
        )
    }
}

impl Articulation {
    pub const ALL: [Self; 11] = [
        Self::None,
        Self::Staccato,
        Self::Accent,
        Self::Tenuto,
        Self::Fermata,
        Self::Marcato,
        Self::Staccatissimo,
        Self::Portato,
        Self::AccentStaccato,
        Self::TenutoAccent,
        Self::MarcatoStaccato,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Keine",
            Self::Staccato => "Staccato",
            Self::Accent => "Akzent",
            Self::Tenuto => "Tenuto",
            Self::Fermata => "Fermate",
            Self::Marcato => "Marcato (Huetchen)",
            Self::Staccatissimo => "Staccatissimo",
            Self::Portato => "Portato (Tenuto + Staccato)",
            Self::AccentStaccato => "Akzent + Staccato",
            Self::TenutoAccent => "Tenuto + Akzent",
            Self::MarcatoStaccato => "Marcato + Staccato",
        }
    }
}

impl Articulation {
    /// SMuFL above/below forms; shared by recognition and preview.
    pub fn glyph(self, below: bool) -> Option<char> {
        let code = match self {
            Self::None => return None,
            Self::Accent => 0xe4a0,
            Self::Staccato => 0xe4a2,
            Self::Tenuto => 0xe4a4,
            Self::Staccatissimo => 0xe4a6,
            Self::Marcato => 0xe4ac,
            Self::MarcatoStaccato => 0xe4ae,
            Self::AccentStaccato => 0xe4b0,
            Self::Portato => 0xe4b2,
            Self::TenutoAccent => 0xe4b4,
            Self::Fermata => 0xe4c0,
        };
        char::from_u32(code + u32::from(below))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DynamicKind {
    Pppp,
    Ppp,
    Pp,
    P,
    Mp,
    Mf,
    F,
    Ff,
    Fff,
    Ffff,
    Fp,
    Fz,
    Sf,
    Sfp,
    Sfpp,
    Sfz,
    Sfzp,
    Sffz,
    Rf,
    Rfz,
}

impl DynamicKind {
    pub const ALL: [Self; 20] = [
        Self::Pppp,
        Self::Ppp,
        Self::Pp,
        Self::P,
        Self::Mp,
        Self::Mf,
        Self::F,
        Self::Ff,
        Self::Fff,
        Self::Ffff,
        Self::Fp,
        Self::Fz,
        Self::Sf,
        Self::Sfp,
        Self::Sfpp,
        Self::Sfz,
        Self::Sfzp,
        Self::Sffz,
        Self::Rf,
        Self::Rfz,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pppp => "pppp",
            Self::Ppp => "ppp",
            Self::Pp => "pp",
            Self::P => "p",
            Self::Mp => "mp",
            Self::Mf => "mf",
            Self::F => "f",
            Self::Ff => "ff",
            Self::Fff => "fff",
            Self::Ffff => "ffff",
            Self::Fp => "fp",
            Self::Fz => "fz",
            Self::Sf => "sf",
            Self::Sfp => "sfp",
            Self::Sfpp => "sfpp",
            Self::Sfz => "sfz",
            Self::Sfzp => "sfzp",
            Self::Sffz => "sffz",
            Self::Rf => "rf",
            Self::Rfz => "rfz",
        }
    }

    pub fn glyph(self) -> char {
        char::from_u32(match self {
            Self::Pppp => 0xe529,
            Self::Ppp => 0xe52a,
            Self::Pp => 0xe52b,
            Self::P => 0xe520,
            Self::Mp => 0xe52c,
            Self::Mf => 0xe52d,
            Self::F => 0xe522,
            Self::Ff => 0xe52f,
            Self::Fff => 0xe530,
            Self::Ffff => 0xe531,
            Self::Fp => 0xe534,
            Self::Fz => 0xe535,
            Self::Sf => 0xe536,
            Self::Sfp => 0xe537,
            Self::Sfpp => 0xe538,
            Self::Sfz => 0xe539,
            Self::Sfzp => 0xe53a,
            Self::Sffz => 0xe53b,
            Self::Rf => 0xe53c,
            Self::Rfz => 0xe53d,
        })
        .unwrap()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dynamic {
    pub staff: usize,
    pub x: f32,
    pub y: f32,
    pub kind: DynamicKind,
    pub bounds: Option<SymbolBounds>,
    pub confirmed: bool,
}

impl Dynamic {
    pub fn symbol_bounds(&self, gap: f32) -> SymbolBounds {
        self.bounds.clone().unwrap_or(SymbolBounds {
            left: self.x - gap * 0.5,
            top: self.y - gap * 0.75,
            right: self.x + gap * (self.kind.label().len() as f32 * 0.8).max(0.5),
            bottom: self.y + gap * 0.75,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMarkKind {
    Segno,
    Coda,
    Ending,
}

impl RepeatMarkKind {
    pub const ALL: [Self; 3] = [Self::Segno, Self::Coda, Self::Ending];

    pub fn label(self) -> &'static str {
        match self {
            Self::Segno => "Segno",
            Self::Coda => "Coda",
            Self::Ending => "Voltenklammer",
        }
    }

    pub fn glyph(self) -> Option<char> {
        match self {
            Self::Segno => Some('\u{e047}'),
            Self::Coda => Some('\u{e048}'),
            Self::Ending => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepeatMark {
    pub staff: usize,
    pub x: f32,
    pub y: f32,
    pub kind: RepeatMarkKind,
    pub end_x: f32,
    /// Comma-separated passes; empty when the printed number is uncertain.
    pub numbers: String,
    pub closed: bool,
    pub bounds: Option<SymbolBounds>,
    pub confirmed: bool,
}

impl RepeatMark {
    pub fn valid_numbers(&self) -> bool {
        self.numbers.is_empty()
            || self.numbers.split(',').all(|number| {
                let number = number.trim();
                number.bytes().all(|byte| byte.is_ascii_digit())
                    && number.parse::<u8>().is_ok_and(|value| value > 0)
            })
    }

    pub fn symbol_bounds(&self, gap: f32) -> SymbolBounds {
        self.bounds.clone().unwrap_or_else(|| {
            if self.kind == RepeatMarkKind::Ending {
                SymbolBounds {
                    left: self.x,
                    top: self.y,
                    right: self.end_x,
                    bottom: self.y + gap * 1.4,
                }
            } else {
                SymbolBounds {
                    left: self.x - gap,
                    top: self.y - gap * 1.3,
                    right: self.x + gap,
                    bottom: self.y + gap * 1.3,
                }
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolBounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SymbolBounds {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    fn valid(&self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|value| value.is_finite())
            && self.left < self.right
            && self.top < self.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeSignature {
    Common,
    Cut,
    Meter { beats: u8, beat_unit: u8 },
}

impl TimeSignature {
    pub(crate) fn ticks(self) -> Option<u32> {
        if !self.valid() {
            return None;
        }
        Some(match self {
            Self::Common | Self::Cut => TICKS_PER_QUARTER * 4,
            Self::Meter { beats, beat_unit } => {
                TICKS_PER_QUARTER * 4 * u32::from(beats) / u32::from(beat_unit)
            }
        })
    }

    pub fn label(self) -> String {
        match self {
            Self::Common => "C (4/4)".into(),
            Self::Cut => "Alla breve (2/2)".into(),
            Self::Meter { beats, beat_unit } => format!("{beats}/{beat_unit}"),
        }
    }

    fn valid(self) -> bool {
        match self {
            Self::Meter { beats, beat_unit } => {
                (1..=32).contains(&beats) && [1, 2, 4, 8, 16, 32].contains(&beat_unit)
            }
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StaffHeader {
    pub clef: Clef,
    pub time_signature: Option<TimeSignature>,
    pub clef_bounds: Option<SymbolBounds>,
    pub time_bounds: Option<SymbolBounds>,
    #[serde(default)]
    pub key_signature: Option<KeySignature>,
    #[serde(default)]
    pub key_bounds: Vec<SymbolBounds>,
    pub confirmed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Staff {
    pub lines: [f32; 5],
    pub left: f32,
    pub right: f32,
    #[serde(default)]
    pub header: StaffHeader,
}

impl Staff {
    pub fn spacing(&self) -> f32 {
        (self.lines[4] - self.lines[0]) / 4.0
    }

    pub fn pitch_at(&self, y: f32) -> i32 {
        self.pitch_at_with_clef(y, self.header.clef)
    }

    pub fn pitch_at_with_clef(&self, y: f32, clef: Clef) -> i32 {
        clef.bottom_pitch() + ((self.lines[4] - y) / (self.spacing() / 2.0)).round() as i32
    }

    pub fn y_at_pitch(&self, pitch: i32) -> f32 {
        self.lines[4] - (pitch - self.header.clef.bottom_pitch()) as f32 * self.spacing() / 2.0
    }

    pub fn accidental_at(&self, pitch: i32) -> i8 {
        self.header
            .key_signature
            .map(|key| key.accidental_at(pitch))
            .unwrap_or(0)
    }
}

fn legacy_accidental_is_explicit() -> bool {
    true
}

/// `actual` notes occupy the time of `normal` notes of the same value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tuplet {
    pub actual: u8,
    pub normal: u8,
}

impl Default for Tuplet {
    fn default() -> Self {
        Self {
            actual: 3,
            normal: 2,
        }
    }
}

impl Tuplet {
    pub fn valid(self) -> bool {
        (2..=64).contains(&self.actual) && (1..=64).contains(&self.normal)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub x: f32,
    pub y: f32,
    pub staff: usize,
    pub pitch: i32,
    pub accidental: i8,
    #[serde(default = "legacy_accidental_is_explicit")]
    pub accidental_explicit: bool,
    pub duration: Duration,
    #[serde(default)]
    pub dots: u8,
    #[serde(default)]
    pub triplet: bool,
    #[serde(default)]
    pub tuplet: Option<Tuplet>,
    #[serde(default)]
    pub grace: bool,
    #[serde(default)]
    pub small: bool,
    pub articulation: Articulation,
    pub confirmed: bool,
}

impl Note {
    /// Read legacy triplets as 3:2; an explicit ratio takes precedence.
    pub fn tuplet_ratio(&self) -> Option<Tuplet> {
        self.tuplet.or_else(|| self.triplet.then(Tuplet::default))
    }

    pub(crate) fn ticks(&self, divisions: u128) -> u128 {
        if self.grace {
            return 0;
        }
        let dotted = u128::from(self.duration.dotted_ticks(self.dots)) * divisions
            / u128::from(TICKS_PER_QUARTER);
        self.tuplet_ratio()
            .filter(|ratio| ratio.valid())
            .map_or(dotted, |ratio| {
                dotted * u128::from(ratio.normal) / u128::from(ratio.actual)
            })
    }

    pub fn notation_scale(&self) -> f32 {
        if self.small { 0.7 } else { 1.0 }
    }

    pub fn duration_label(&self) -> String {
        let ratio = self.tuplet_ratio().map_or(String::new(), |ratio| {
            format!(" ({}:{})", ratio.actual, ratio.normal)
        });
        format!(
            "{}{}{}{}",
            self.duration.label(),
            ".".repeat(usize::from(self.dots)),
            ratio,
            if self.grace { " Vorschlag" } else { "" }
        )
    }

    pub fn refresh_accidental(&mut self, staff: &Staff) {
        if !self.accidental_explicit {
            self.accidental = staff.accidental_at(self.pitch);
        }
    }

    pub fn pitch_label(&self) -> String {
        let names = ["C", "D", "E", "F", "G", "A", "B"];
        let accidental = match self.accidental {
            -1 => "b",
            1 => "#",
            _ => "",
        };
        format!(
            "{}{}{}",
            names[self.pitch.rem_euclid(7) as usize],
            accidental,
            self.pitch.div_euclid(7)
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Score {
    pub staves: Vec<Staff>,
    pub notes: Vec<Note>,
    #[serde(default)]
    pub rests: Vec<Rest>,
    #[serde(default)]
    pub barlines: Vec<Barline>,
    #[serde(default)]
    pub spans: Vec<Span>,
    #[serde(default)]
    pub key_changes: Vec<KeyChange>,
    #[serde(default)]
    pub time_changes: Vec<TimeChange>,
    #[serde(default)]
    pub dynamics: Vec<Dynamic>,
    #[serde(default)]
    pub repeat_marks: Vec<RepeatMark>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeChange {
    pub staff: usize,
    pub x: f32,
    pub time_signature: TimeSignature,
    #[serde(default)]
    pub bounds: Vec<SymbolBounds>,
    pub confirmed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyChange {
    pub staff: usize,
    pub x: f32,
    pub key_signature: KeySignature,
    #[serde(default)]
    pub bounds: Vec<SymbolBounds>,
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanKind {
    Crescendo,
    Decrescendo,
    Slur,
    Tie,
}

impl SpanKind {
    pub const ALL: [Self; 4] = [Self::Crescendo, Self::Decrescendo, Self::Slur, Self::Tie];

    pub fn label(self) -> &'static str {
        match self {
            Self::Crescendo => "Crescendo",
            Self::Decrescendo => "Decrescendo",
            Self::Slur => "Bindebogen",
            Self::Tie => "Haltebogen",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Span {
    pub staff: usize,
    pub kind: SpanKind,
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub height: f32,
    pub confirmed: bool,
}

impl Span {
    pub fn curve_point(&self, fraction: f32) -> [f32; 2] {
        [
            self.start[0] + (self.end[0] - self.start[0]) * fraction,
            self.start[1]
                + (self.end[1] - self.start[1]) * fraction
                + 4.0 * self.height * fraction * (1.0 - fraction),
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Barline {
    pub staff: usize,
    pub x: f32,
    #[serde(default)]
    pub kind: BarlineKind,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarlineKind {
    #[default]
    Single,
    Double,
    Final,
    RepeatStart,
    RepeatEnd,
    RepeatBoth,
}

impl BarlineKind {
    pub const ALL: [Self; 6] = [
        Self::Single,
        Self::Double,
        Self::Final,
        Self::RepeatStart,
        Self::RepeatEnd,
        Self::RepeatBoth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "Einfach",
            Self::Double => "Doppelstrich",
            Self::Final => "Schlussstrich",
            Self::RepeatStart => "Wiederholung Anfang",
            Self::RepeatEnd => "Wiederholung Ende",
            Self::RepeatBoth => "Wiederholung beidseitig",
        }
    }
}

impl Score {
    pub fn time_signature_before(&self, staff_index: usize, x: f32) -> Option<TimeSignature> {
        self.time_signature_before_with_inherited(staff_index, x, None)
    }

    /// Resolve the meter using the preceding pages' final meter as a fallback.
    pub fn time_signature_before_with_inherited(
        &self,
        staff_index: usize,
        x: f32,
        inherited: Option<TimeSignature>,
    ) -> Option<TimeSignature> {
        self.staves.get(staff_index)?;
        let mut order: Vec<_> = (0..self.staves.len()).collect();
        order.sort_by(|&a, &b| self.staves[a].lines[0].total_cmp(&self.staves[b].lines[0]));
        let mut meter = inherited;
        for index in order {
            meter = self.staves[index].header.time_signature.or(meter);
            if let Some(change) = self
                .time_changes
                .iter()
                .filter(|change| change.staff == index && (index != staff_index || change.x < x))
                .max_by(|a, b| a.x.total_cmp(&b.x))
            {
                meter = Some(change.time_signature);
            }
            if index == staff_index {
                break;
            }
        }
        meter
    }

    /// Last explicit meter in reading order, including courtesy changes.
    pub fn ending_time_signature(&self) -> Option<TimeSignature> {
        let index = (0..self.staves.len())
            .max_by(|&a, &b| self.staves[a].lines[0].total_cmp(&self.staves[b].lines[0]))?;
        self.time_signature_before(index, f32::INFINITY)
    }

    pub fn recompute_accidentals(&mut self) {
        for (staff_index, staff) in self.staves.iter().enumerate() {
            let mut bars: Vec<_> = self
                .barlines
                .iter()
                .filter(|bar| bar.staff == staff_index)
                .map(|bar| bar.x)
                .collect();
            bars.sort_by(f32::total_cmp);
            let mut changes: Vec<_> = self
                .key_changes
                .iter()
                .filter(|change| change.staff == staff_index)
                .collect();
            changes.sort_by(|first, second| first.x.total_cmp(&second.x));
            let mut change_index = 0;
            let mut key = staff.header.key_signature;
            let mut indices: Vec<_> = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, note)| note.staff == staff_index)
                .map(|(index, _)| index)
                .collect();
            indices.sort_by(|&first, &second| {
                self.notes[first]
                    .x
                    .total_cmp(&self.notes[second].x)
                    .then_with(|| {
                        self.notes[second]
                            .accidental_explicit
                            .cmp(&self.notes[first].accidental_explicit)
                    })
            });
            let mut active = std::collections::BTreeMap::new();
            let mut bar_index = 0;
            for index in indices {
                let note = &mut self.notes[index];
                while bar_index < bars.len() && bars[bar_index] <= note.x {
                    active.clear();
                    bar_index += 1;
                }
                while change_index < changes.len() && changes[change_index].x <= note.x {
                    active.clear();
                    key = Some(changes[change_index].key_signature);
                    change_index += 1;
                }
                if note.accidental_explicit {
                    active.insert(note.pitch, note.accidental);
                } else {
                    let value = active.get(&note.pitch).copied().unwrap_or_else(|| {
                        key.map(|key| key.accidental_at(note.pitch)).unwrap_or(0)
                    });
                    if note.accidental != value {
                        note.accidental = value;
                        note.confirmed = false;
                    }
                }
            }
        }
    }

    pub fn set_staff_header(&mut self, staff_index: usize, header: StaffHeader) {
        let Some(staff) = self.staves.get_mut(staff_index) else {
            return;
        };
        let clef_changed = staff.header.clef != header.clef;
        let key_changed = staff.header.key_signature != header.key_signature;
        staff.header = header;
        if clef_changed || key_changed {
            for note in self
                .notes
                .iter_mut()
                .filter(|note| note.staff == staff_index)
            {
                let previous = (note.pitch, note.accidental);
                if clef_changed {
                    note.pitch = staff.pitch_at(note.y).clamp(0, 69);
                }
                if previous != (note.pitch, note.accidental) {
                    note.confirmed = false;
                }
            }
            self.recompute_accidentals();
        }
    }

    pub fn add_note(&mut self, x: f32, y: f32) -> Option<usize> {
        let (staff_index, staff) =
            self.staves
                .iter()
                .enumerate()
                .min_by(|(_, first), (_, second)| {
                    (first.lines[2] - y)
                        .abs()
                        .total_cmp(&(second.lines[2] - y).abs())
                })?;
        let pitch = staff.pitch_at(y).clamp(0, 69);
        self.notes.push(Note {
            x,
            y,
            staff: staff_index,
            pitch,
            accidental: staff.accidental_at(pitch),
            accidental_explicit: false,
            duration: Duration::Quarter,
            dots: 0,
            triplet: false,
            tuplet: None,
            grace: false,
            small: false,
            articulation: Articulation::None,
            confirmed: true,
        });
        self.recompute_accidentals();
        Some(self.notes.len() - 1)
    }

    pub fn add_rest(&mut self, x: f32, y: f32, duration: Duration) -> Option<usize> {
        let (staff_index, _) =
            self.staves
                .iter()
                .enumerate()
                .min_by(|(_, first), (_, second)| {
                    (first.lines[2] - y)
                        .abs()
                        .total_cmp(&(second.lines[2] - y).abs())
                })?;
        self.rests.push(Rest {
            x,
            y,
            staff: staff_index,
            duration,
            dots: 0,
            measures: None,
            confirmed: true,
            bounds: None,
        });
        Some(self.rests.len() - 1)
    }

    /// A common time unit keeps all supported n-tuplets integral, including
    /// double-dotted 64ths. u128 accommodates the LCM of every ratio up to 64.
    pub(crate) fn rhythm_divisions(&self) -> u128 {
        self.notes
            .iter()
            .filter(|note| !note.grace)
            .filter_map(|note| note.tuplet)
            .filter(|ratio| ratio.valid())
            .fold(1, |divisions, ratio| {
                let actual = u128::from(ratio.actual);
                let (mut a, mut b) = (divisions, actual);
                while b != 0 {
                    (a, b) = (b, a % b);
                }
                divisions / a * actual
            })
            * u128::from(TICKS_PER_QUARTER)
    }

    pub fn validate(&self) -> Result<(), String> {
        for mark in &self.repeat_marks {
            if !mark.x.is_finite()
                || !mark.y.is_finite()
                || !mark.end_x.is_finite()
                || !mark.valid_numbers()
                || self.staves.get(mark.staff).is_none_or(|staff| {
                    mark.x < staff.left
                        || mark.x > staff.right
                        || (mark.kind == RepeatMarkKind::Ending
                            && (mark.end_x <= mark.x || mark.end_x > staff.right))
                })
                || mark.bounds.as_ref().is_some_and(|bounds| !bounds.valid())
            {
                return Err("Ungueltiges Wiederholungszeichen im Projekt.".into());
            }
        }
        for dynamic in &self.dynamics {
            if !dynamic.x.is_finite()
                || !dynamic.y.is_finite()
                || self
                    .staves
                    .get(dynamic.staff)
                    .is_none_or(|staff| dynamic.x < staff.left || dynamic.x > staff.right)
                || dynamic
                    .bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.valid())
            {
                return Err("Ungueltige Dynamik im Projekt.".into());
            }
        }
        for change in &self.time_changes {
            if !change.x.is_finite()
                || self
                    .staves
                    .get(change.staff)
                    .is_none_or(|staff| change.x < staff.left || change.x > staff.right)
                || !change.time_signature.valid()
                || change.bounds.iter().any(|bounds| !bounds.valid())
            {
                return Err("Ungueltiger Taktwechsel im Projekt.".into());
            }
        }
        for change in &self.key_changes {
            if !change.x.is_finite()
                || self
                    .staves
                    .get(change.staff)
                    .is_none_or(|staff| change.x < staff.left || change.x > staff.right)
                || !(-7..=7).contains(&change.key_signature.fifths)
                || change.bounds.iter().any(|bounds| !bounds.valid())
            {
                return Err("Ungueltiger Tonartwechsel im Projekt.".into());
            }
        }
        for span in &self.spans {
            if span
                .start
                .iter()
                .chain(span.end.iter())
                .chain(std::iter::once(&span.height))
                .any(|value| !value.is_finite())
                || self
                    .staves
                    .get(span.staff)
                    .is_none_or(|staff| span.start[0] < staff.left || span.end[0] > staff.right)
                || span.start[0] >= span.end[0]
                || (matches!(span.kind, SpanKind::Crescendo | SpanKind::Decrescendo)
                    && span.height <= 0.0)
            {
                return Err("Ungueltige Gabel oder ungueltiger Bogen im Projekt.".into());
            }
        }
        for bar in &self.barlines {
            if !bar.x.is_finite()
                || self
                    .staves
                    .get(bar.staff)
                    .is_none_or(|staff| bar.x < staff.left || bar.x > staff.right)
            {
                return Err("Ungueltiger Taktstrich im Projekt.".into());
            }
        }
        for staff in &self.staves {
            if !staff.left.is_finite()
                || !staff.right.is_finite()
                || staff.left >= staff.right
                || staff.lines.iter().any(|line| !line.is_finite())
                || staff.lines.windows(2).any(|lines| lines[0] >= lines[1])
                || staff
                    .header
                    .clef_bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.valid())
                || staff
                    .header
                    .time_bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.valid())
                || staff
                    .header
                    .time_signature
                    .is_some_and(|meter| !meter.valid())
                || staff
                    .header
                    .key_signature
                    .is_some_and(|key| !(-7..=7).contains(&key.fifths))
                || staff.header.key_bounds.iter().any(|bounds| !bounds.valid())
            {
                return Err("Ungueltiges Notensystem im Projekt.".into());
            }
        }
        for note in &self.notes {
            if note.staff >= self.staves.len()
                || !note.x.is_finite()
                || !note.y.is_finite()
                || !(0..=69).contains(&note.pitch)
                || !(-1..=1).contains(&note.accidental)
                || note.dots > 2
                || note.tuplet.is_some_and(|ratio| !ratio.valid())
            {
                return Err("Ungueltige Note im Projekt.".into());
            }
        }
        for rest in &self.rests {
            if rest.staff >= self.staves.len()
                || !rest.x.is_finite()
                || !rest.y.is_finite()
                || rest.dots > 2
                || (rest.measures.is_some() && rest.dots != 0)
                || rest
                    .measures
                    .is_some_and(|count| !(2..=99).contains(&count))
                || rest.bounds.as_ref().is_some_and(|bounds| !bounds.valid())
            {
                return Err("Ungueltige Pause im Projekt.".into());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub image_name: String,
    pub image_png: Vec<u8>,
    pub score: Score,
}

/// A page in a version 2 project, in reading/export order.
#[derive(Serialize, Deserialize)]
pub struct ProjectPage {
    pub image_name: String,
    pub image_png: Vec<u8>,
    pub score: Score,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectDocument {
    pub version: u32,
    pub pages: Vec<ProjectPage>,
}

impl ProjectDocument {
    /// Accept legacy single-page files as well as multi-page documents.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Version {
            version: u32,
        }
        let version: Version =
            serde_json::from_slice(bytes).map_err(|error| format!("Projekt unlesbar: {error}"))?;
        let document = match version.version {
            1 => {
                let project: Project = serde_json::from_slice(bytes)
                    .map_err(|error| format!("Projekt unlesbar: {error}"))?;
                Self {
                    version: 2,
                    pages: vec![ProjectPage {
                        image_name: project.image_name,
                        image_png: project.image_png,
                        score: project.score,
                    }],
                }
            }
            2 => serde_json::from_slice(bytes)
                .map_err(|error| format!("Projekt unlesbar: {error}"))?,
            _ => return Err("Unbekannte Projektversion.".into()),
        };
        if document.pages.is_empty() {
            return Err("Projekt enthaelt keine Seiten.".into());
        }
        for (index, page) in document.pages.iter().enumerate() {
            page.score
                .validate()
                .map_err(|error| format!("Seite {}: {error}", index + 1))?;
        }
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn multipage_projects_accept_legacy_files_and_reject_invalid_documents() {
        let legacy = Project {
            version: 1,
            image_name: "legacy.png".into(),
            image_png: vec![1, 2, 3],
            score: Score::default(),
        };
        let converted = ProjectDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(converted.version, 2);
        assert_eq!(converted.pages.len(), 1);
        assert_eq!(converted.pages[0].image_name, "legacy.png");
        assert_eq!(converted.pages[0].image_png, vec![1, 2, 3]);
        let bytes = serde_json::to_vec(&converted).unwrap();
        assert_eq!(ProjectDocument::from_json(&bytes).unwrap().pages.len(), 1);
        assert!(ProjectDocument::from_json(br#"{"version":2,"pages":[]}"#).is_err());
        assert!(ProjectDocument::from_json(br#"{"version":99,"pages":[]}"#).is_err());
        let mut invalid = converted;
        invalid.pages[0].score.staves.push(Staff {
            lines: [1.0; 5],
            left: 0.0,
            right: 10.0,
            header: Default::default(),
        });
        let error = ProjectDocument::from_json(&serde_json::to_vec(&invalid).unwrap())
            .err()
            .unwrap();
        assert!(error.starts_with("Seite 1:"), "{error}");
    }

    #[test]
    fn tuplets_grace_and_small_notes_roundtrip_and_validate() {
        let mut score: Score = serde_json::from_str(r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[{"x":50,"y":55,"staff":0,"pitch":31,"accidental":0,"duration":"Quarter","articulation":"None","confirmed":true,"triplet":true}]}"#).unwrap();
        assert_eq!(score.notes[0].tuplet_ratio(), Some(Tuplet::default()));
        assert!(!score.notes[0].grace);
        assert!(!score.notes[0].small);
        score.notes[0].tuplet = Some(Tuplet {
            actual: 7,
            normal: 4,
        });
        score.notes[0].grace = true;
        score.notes[0].small = true;
        let restored: Score =
            serde_json::from_str(&serde_json::to_string(&score).unwrap()).unwrap();
        assert!(restored.validate().is_ok());
        let note = &restored.notes[0];
        assert_eq!(
            note.tuplet_ratio(),
            Some(Tuplet {
                actual: 7,
                normal: 4
            })
        );
        assert_eq!(note.duration_label(), "1/4 (7:4) Vorschlag");
        assert!(note.grace && note.small);
        for (actual, normal) in [(0, 4), (1, 4), (65, 4), (7, 0), (7, 65)] {
            score.notes[0].tuplet = Some(Tuplet { actual, normal });
            assert!(score.validate().is_err());
        }
    }

    use super::*;

    #[test]
    fn repeat_marks_roundtrip_and_reject_invalid_geometry_and_numbers() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[100,120,140,160,180],"left":20,"right":500}],"notes":[]}"#,
        )
        .unwrap();
        assert!(score.repeat_marks.is_empty());
        for kind in RepeatMarkKind::ALL {
            score.repeat_marks.push(RepeatMark {
                staff: 0,
                x: 100.0,
                y: 50.0,
                kind,
                end_x: 400.0,
                numbers: "1,2".into(),
                closed: true,
                bounds: None,
                confirmed: false,
            });
        }
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert!(restored.validate().is_ok());
        assert_eq!(restored.repeat_marks.len(), 3);
        let original = score.repeat_marks[2].clone();
        for (x, end_x, staff, numbers) in [
            (100.0, 90.0, 0, "1"),
            (100.0, 501.0, 0, "1"),
            (f32::NAN, 400.0, 0, "1"),
            (100.0, 400.0, 3, "1"),
            (100.0, 400.0, 0, "0"),
            (100.0, 400.0, 0, "1,"),
            (100.0, 400.0, 0, "1<"),
        ] {
            score.repeat_marks[2] = RepeatMark {
                x,
                end_x,
                staff,
                numbers: numbers.into(),
                ..original.clone()
            };
            assert!(score.validate().is_err());
        }
        score.repeat_marks[2] = RepeatMark {
            numbers: String::new(),
            ..original
        };
        assert!(score.validate().is_ok());
    }

    #[test]
    fn dynamics_roundtrip_validate_and_default_empty_in_legacy_projects() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":400}],"notes":[]}"#,
        )
        .unwrap();
        assert!(score.dynamics.is_empty());
        for (index, kind) in DynamicKind::ALL.into_iter().enumerate() {
            score.dynamics.push(Dynamic {
                staff: 0,
                x: 30.0 + index as f32 * 15.0,
                y: 90.0,
                kind,
                bounds: Some(SymbolBounds {
                    left: 20.0,
                    top: 75.0,
                    right: 50.0,
                    bottom: 105.0,
                }),
                confirmed: index % 2 == 0,
            });
        }
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert!(restored.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&score).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
        let original = score.dynamics[0].clone();
        for bad in [
            Dynamic {
                staff: 1,
                ..original.clone()
            },
            Dynamic {
                x: f32::NAN,
                ..original.clone()
            },
            Dynamic {
                y: f32::INFINITY,
                ..original.clone()
            },
            Dynamic {
                x: 401.0,
                ..original.clone()
            },
            Dynamic {
                bounds: Some(SymbolBounds {
                    left: 20.0,
                    top: 80.0,
                    right: 10.0,
                    bottom: 90.0,
                }),
                ..original.clone()
            },
        ] {
            score.dynamics[0] = bad;
            assert!(score.validate().is_err());
        }
        for kind in Articulation::ALL {
            let restored: Articulation =
                serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
            assert_eq!(restored, kind);
        }
    }

    #[test]
    fn time_changes_roundtrip_validate_and_default_empty_in_old_projects() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[]}"#,
        )
        .unwrap();
        assert!(score.time_changes.is_empty());
        let change = TimeChange {
            staff: 0,
            x: 100.0,
            time_signature: TimeSignature::Meter {
                beats: 7,
                beat_unit: 8,
            },
            bounds: vec![SymbolBounds {
                left: 100.0,
                top: 20.0,
                right: 120.0,
                bottom: 60.0,
            }],
            confirmed: false,
        };
        score.time_changes.push(change.clone());
        assert!(score.validate().is_ok());
        let restored: Score =
            serde_json::from_str(&serde_json::to_string(&score).unwrap()).unwrap();
        assert_eq!(
            restored.time_changes[0].time_signature,
            change.time_signature
        );
        assert_eq!(restored.time_changes[0].bounds, change.bounds);
        assert!(!restored.time_changes[0].confirmed);
        assert!(score.time_signature_before(0, 100.0).is_none());
        assert_eq!(
            score.time_signature_before(0, 101.0),
            Some(change.time_signature)
        );
        let mut later = restored.clone();
        let mut staff = later.staves[0].clone();
        staff.lines = staff.lines.map(|line| line + 100.0);
        later.staves.push(staff);
        assert_eq!(
            later.time_signature_before(1, 10.0),
            Some(change.time_signature)
        );
        later.staves[1].header.time_signature = Some(TimeSignature::Cut);
        assert_eq!(
            later.time_signature_before(1, 10.0),
            Some(TimeSignature::Cut)
        );
        for invalid in [
            TimeChange {
                staff: 1,
                ..change.clone()
            },
            TimeChange {
                x: f32::NAN,
                ..change.clone()
            },
            TimeChange {
                x: 201.0,
                ..change.clone()
            },
            TimeChange {
                time_signature: TimeSignature::Meter {
                    beats: 0,
                    beat_unit: 4,
                },
                ..change.clone()
            },
            TimeChange {
                time_signature: TimeSignature::Meter {
                    beats: 3,
                    beat_unit: 3,
                },
                ..change.clone()
            },
        ] {
            score.time_changes = vec![invalid];
            assert!(score.validate().is_err());
        }
    }

    #[test]
    fn barline_types_roundtrip_and_legacy_bars_default_to_single() {
        let legacy: Barline = serde_json::from_str(r#"{"staff":0,"x":100}"#).unwrap();
        assert_eq!(legacy.kind, BarlineKind::Single);
        assert!(!legacy.confirmed);
        for kind in BarlineKind::ALL {
            let bar = Barline {
                staff: 0,
                x: 100.0,
                kind,
                confirmed: true,
            };
            let restored: Barline =
                serde_json::from_slice(&serde_json::to_vec(&bar).unwrap()).unwrap();
            assert_eq!(restored.kind, kind);
            assert!(restored.confirmed);
        }
    }

    #[test]
    fn key_changes_reset_accidentals_and_survive_barlines_and_roundtrip() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[10,20,30,40,50],"left":0,"right":400}],"notes":[]}"#,
        )
        .unwrap();
        assert!(score.key_changes.is_empty());
        score.staves[0].header.key_signature = Some(KeySignature { fifths: -2 });
        for column in [50.0, 100.0, 150.0, 220.0, 260.0, 320.0] {
            score
                .add_note(column, score.staves[0].y_at_pitch(34))
                .unwrap();
        }
        score.notes[0].accidental = 1;
        score.notes[0].accidental_explicit = true;
        score.key_changes = vec![
            KeyChange {
                staff: 0,
                x: 120.0,
                key_signature: KeySignature { fifths: -4 },
                bounds: vec![],
                confirmed: false,
            },
            KeyChange {
                staff: 0,
                x: 250.0,
                key_signature: KeySignature { fifths: 0 },
                bounds: vec![],
                confirmed: true,
            },
        ];
        score.barlines.push(Barline {
            staff: 0,
            x: 200.0,
            kind: BarlineKind::Single,
            confirmed: false,
        });
        score.recompute_accidentals();
        assert_eq!(
            score
                .notes
                .iter()
                .map(|note| note.accidental)
                .collect::<Vec<_>>(),
            [1, 1, -1, -1, 0, 0]
        );
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert_eq!(restored.key_changes.len(), 2);
        assert!(restored.key_changes[1].confirmed);
        assert!(restored.validate().is_ok());
        score.key_changes[0].key_signature.fifths = 8;
        assert!(score.validate().is_err());
    }

    #[test]
    fn key_changes_are_position_local_and_staff_local() {
        let staff = Staff {
            lines: [10.0, 20.0, 30.0, 40.0, 50.0],
            left: 0.0,
            right: 400.0,
            header: StaffHeader {
                key_signature: Some(KeySignature { fifths: -2 }),
                ..Default::default()
            },
        };
        let mut score = Score {
            staves: vec![
                staff.clone(),
                Staff {
                    lines: staff.lines.map(|line| line + 100.0),
                    ..staff
                },
            ],
            ..Default::default()
        };
        score.key_changes = vec![
            KeyChange {
                staff: 0,
                x: 250.0,
                key_signature: KeySignature { fifths: 0 },
                bounds: vec![],
                confirmed: false,
            },
            KeyChange {
                staff: 0,
                x: 120.0,
                key_signature: KeySignature { fifths: 1 },
                bounds: vec![],
                confirmed: false,
            },
        ];
        for (x, pitch) in [
            (50.0, 34),
            (150.0, 31),
            (160.0, 38),
            (170.0, 31),
            (180.0, 31),
            (190.0, 38),
            (220.0, 31),
            (280.0, 31),
        ] {
            score
                .add_note(x, score.staves[0].y_at_pitch(pitch))
                .unwrap();
        }
        score.notes[3].accidental_explicit = true;
        score.notes[3].accidental = 0;
        score.barlines.push(Barline {
            staff: 0,
            x: 200.0,
            kind: BarlineKind::Single,
            confirmed: false,
        });
        score
            .add_note(320.0, score.staves[1].y_at_pitch(34))
            .unwrap();
        score.recompute_accidentals();
        assert_eq!(
            score
                .notes
                .iter()
                .map(|note| note.accidental)
                .collect::<Vec<_>>(),
            [-1, 1, 1, 0, 0, 1, 1, 0, -1]
        );
        score.key_changes[1].x = 155.0;
        score.recompute_accidentals();
        assert_eq!(score.notes[1].accidental, 0);
        assert_eq!(score.notes[2].accidental, 1);
        for invalid in [
            KeyChange {
                staff: 2,
                ..score.key_changes[0].clone()
            },
            KeyChange {
                x: f32::NAN,
                ..score.key_changes[0].clone()
            },
            KeyChange {
                x: 401.0,
                ..score.key_changes[0].clone()
            },
            KeyChange {
                bounds: vec![SymbolBounds {
                    left: 50.0,
                    right: 40.0,
                    top: 10.0,
                    bottom: 20.0,
                }],
                ..score.key_changes[0].clone()
            },
        ] {
            let mut broken = score.clone();
            broken.key_changes.push(invalid);
            assert!(broken.validate().is_err());
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    fn accidentals_apply_until_barline_and_only_in_the_same_octave() {
        let mut score = Score::default();
        score.staves.push(Staff {
            lines: [10.0, 20.0, 30.0, 40.0, 50.0],
            left: 0.0,
            right: 400.0,
            header: StaffHeader {
                clef: Clef::Treble,
                key_signature: Some(KeySignature { fifths: 1 }),
                ..Default::default()
            },
        });
        score.barlines.push(Barline {
            staff: 0,
            x: 200.0,
            kind: BarlineKind::Single,
            confirmed: false,
        });
        for x in [100.0, 150.0, 190.0, 220.0, 250.0, 280.0, 310.0] {
            let y = score.staves[0].y_at_pitch(31);
            score.add_note(x, y).unwrap();
        }
        score.notes[0].accidental_explicit = true;
        score.notes[0].accidental = 0;
        score.notes[2].pitch = 38;
        score.notes[4].accidental_explicit = true;
        score.notes[4].accidental = -1;
        score.notes[6].accidental_explicit = true;
        score.notes[6].accidental = 1;
        score.recompute_accidentals();
        assert_eq!(
            score
                .notes
                .iter()
                .map(|note| note.accidental)
                .collect::<Vec<_>>(),
            [0, 0, 1, 1, -1, -1, 1]
        );
        let mut edited = score.clone();
        edited.barlines[0].x = 130.0;
        edited.recompute_accidentals();
        assert_eq!(edited.notes[1].accidental, 1);
        edited.barlines.clear();
        edited.recompute_accidentals();
        assert_eq!(edited.notes[3].accidental, 0);
        edited.notes.remove(0);
        edited.recompute_accidentals();
        assert_eq!(edited.notes[0].accidental, 1);
        let mut other_staff = score.clone();
        other_staff.staves.push(other_staff.staves[0].clone());
        let mut note = other_staff.notes[1].clone();
        note.staff = 1;
        other_staff.notes.push(note);
        other_staff.recompute_accidentals();
        assert_eq!(other_staff.notes.last().unwrap().accidental, 1);
        score.notes.swap(0, 5);
        score.recompute_accidentals();
        assert_eq!(score.notes[0].accidental, -1);
        assert_eq!(score.notes[5].accidental, 0);
        let restored: Score =
            serde_json::from_str(&serde_json::to_string(&score).unwrap()).unwrap();
        assert_eq!(restored.barlines.len(), 1);
        assert_eq!(restored.barlines[0].x, 200.0);
        assert!(restored.validate().is_ok());
        score.barlines[0].staff = 10;
        assert!(score.validate().is_err());
    }

    #[test]
    fn key_signatures_alter_pitch_classes_in_every_octave() {
        let sharp = KeySignature { fifths: 1 };
        assert_eq!(sharp.accidental_at(24), 1);
        assert_eq!(sharp.accidental_at(31), 1);
        assert_eq!(sharp.accidental_at(30), 0);
        let flats = KeySignature { fifths: -2 };
        assert_eq!(flats.accidental_at(20), -1);
        assert_eq!(flats.accidental_at(23), -1);
        assert_eq!(flats.accidental_at(24), 0);
        for pitch in 0..70 {
            assert_eq!(KeySignature { fifths: 7 }.accidental_at(pitch), 1);
            assert_eq!(KeySignature { fifths: -7 }.accidental_at(pitch), -1);
        }
    }

    #[test]
    fn key_corrections_preserve_explicit_accidentals_and_rest_roundtrips() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[]}"#,
        )
        .unwrap();
        for _note in 0..4 {
            score.add_note(50.0, 55.0);
        }
        for (index, accidental) in [-1, 0, 1].into_iter().enumerate() {
            score.notes[index + 1].accidental_explicit = true;
            score.notes[index + 1].accidental = accidental;
        }
        let header = StaffHeader {
            key_signature: Some(KeySignature { fifths: 1 }),
            ..Default::default()
        };
        score.set_staff_header(0, header);
        assert_eq!(score.notes[0].pitch_label(), "F#4");
        assert_eq!(score.notes[1].pitch_label(), "Fb4");
        assert_eq!(score.notes[2].pitch_label(), "F4");
        assert_eq!(score.notes[3].pitch_label(), "F#4");
        score.notes[0].pitch = 32;
        score.notes[0].refresh_accidental(&score.staves[0]);
        assert_eq!(score.notes[0].pitch_label(), "G4");
        score.add_rest(70.0, 40.0, Duration::Sixteenth);
        let encoded = serde_json::to_vec(&score).unwrap();
        let restored: Score = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            restored.staves[0].header.key_signature,
            Some(KeySignature { fifths: 1 })
        );
        assert_eq!(restored.rests[0].duration, Duration::Sixteenth);
        assert!(restored.rests[0].confirmed);
        assert!(restored.notes[2].accidental_explicit);
        assert!(restored.validate().is_ok());
        score.staves[0].header.key_signature = Some(KeySignature { fifths: 8 });
        assert!(score.validate().is_err());
        score.staves[0].header.key_signature = None;
        score.rests[0].staff = 1;
        assert!(score.validate().is_err());
        assert!(
            Score::default()
                .add_rest(1.0, 1.0, Duration::Whole)
                .is_none()
        );
    }

    #[test]
    fn multimeasure_rests_roundtrip_and_legacy_rests_remain_ordinary() {
        let mut score: Score = serde_json::from_str(r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[],"rests":[{"x":70,"y":40,"staff":0,"duration":"Whole","confirmed":false,"bounds":null}]}"#).unwrap();
        assert_eq!(score.rests[0].measures, None);
        assert_eq!(score.rests[0].dots, 0);
        assert_eq!(score.rests[0].duration_label(), "1/1");
        for dots in [1, 2] {
            score.rests[0].dots = dots;
            let restored: Score =
                serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
            assert_eq!(restored.rests[0].dots, dots);
            assert_eq!(
                restored.rests[0].duration_label(),
                format!("1/1{}", ".".repeat(usize::from(dots)))
            );
            assert!(restored.validate().is_ok());
        }
        score.rests[0].dots = 3;
        assert!(score.validate().is_err());
        score.rests[0].measures = Some(8);
        score.rests[0].dots = 1;
        assert!(
            score.validate().is_err(),
            "multimeasure rests cannot be dotted"
        );
        score.rests[0].dots = 0;
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert_eq!(restored.rests[0].measures, Some(8));
        assert_eq!(restored.rests[0].duration_label(), "8 Takte");
        assert!(restored.validate().is_ok());
        for count in [0, 1, 100, 255] {
            score.rests[0].measures = Some(count);
            assert!(score.validate().is_err());
        }
    }

    #[test]
    fn legacy_accidentals_remain_explicit() {
        let mut score: Score = serde_json::from_str(r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[{"x":50,"y":55,"staff":0,"pitch":31,"accidental":0,"duration":"Quarter","articulation":"None","confirmed":true}]}"#).unwrap();
        assert!(score.rests.is_empty());
        assert!(score.notes[0].accidental_explicit);
        assert!(!score.notes[0].triplet);
        score.set_staff_header(
            0,
            StaffHeader {
                key_signature: Some(KeySignature { fifths: 1 }),
                ..Default::default()
            },
        );
        assert_eq!(score.notes[0].pitch_label(), "F4");
        assert!(score.notes[0].confirmed);
    }

    #[test]
    fn header_correction_recalculates_pitch_without_moving_notes() {
        let staff = Staff {
            lines: [20.0, 30.0, 40.0, 50.0, 60.0],
            left: 10.0,
            right: 200.0,
            header: Default::default(),
        };
        let mut score = Score {
            staves: vec![staff],
            notes: vec![],
            rests: vec![],
            barlines: vec![],
            spans: vec![],
            key_changes: vec![],
            time_changes: vec![],
            dynamics: vec![],
            repeat_marks: vec![],
        };
        score.add_note(50.0, 60.0);
        let header = StaffHeader {
            clef: Clef::Bass,
            time_signature: Some(TimeSignature::Common),
            confirmed: true,
            ..Default::default()
        };
        score.set_staff_header(0, header);
        assert_eq!(score.notes[0].pitch_label(), "G2");
        assert_eq!(score.notes[0].y, 60.0);
        assert_eq!(score.staves[0].y_at_pitch(score.notes[0].pitch), 60.0);
        assert!(!score.notes[0].confirmed);
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert_eq!(restored.staves[0].header, score.staves[0].header);
        assert!(restored.validate().is_ok());
    }

    #[test]
    fn spans_roundtrip_validate_and_default_empty_in_legacy_projects() {
        let mut score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[]}"#,
        )
        .unwrap();
        assert!(score.spans.is_empty());
        for kind in SpanKind::ALL {
            score.spans.push(Span {
                staff: 0,
                kind,
                start: [30.0, 80.0],
                end: [170.0, 82.0],
                height: 10.0,
                confirmed: true,
            });
        }
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert_eq!(
            restored
                .spans
                .iter()
                .map(|span| span.kind)
                .collect::<Vec<_>>(),
            SpanKind::ALL
        );
        assert!(restored.spans.iter().all(|span| span.confirmed));
        assert_eq!(restored.spans[0].curve_point(0.0), [30.0, 80.0]);
        assert_eq!(restored.spans[0].curve_point(0.5), [100.0, 91.0]);
        assert_eq!(restored.spans[0].curve_point(1.0), [170.0, 82.0]);
        assert!(restored.validate().is_ok());
        for bad in [
            Span {
                staff: 1,
                ..score.spans[0].clone()
            },
            Span {
                start: [180.0, 80.0],
                ..score.spans[0].clone()
            },
            Span {
                end: [210.0, 80.0],
                ..score.spans[0].clone()
            },
            Span {
                height: f32::NAN,
                ..score.spans[0].clone()
            },
            Span {
                height: -1.0,
                ..score.spans[0].clone()
            },
        ] {
            score.spans[0] = bad;
            assert!(score.validate().is_err());
        }
    }

    #[test]
    fn legacy_projects_default_to_the_original_treble_mapping() {
        let score: Score = serde_json::from_str(
            r#"{"staves":[{"lines":[20,30,40,50,60],"left":10,"right":200}],"notes":[]}"#,
        )
        .unwrap();
        assert_eq!(score.staves[0].header.clef, Clef::Treble);
        assert_eq!(score.staves[0].pitch_at(60.0), 30);
        assert!(score.staves[0].header.time_signature.is_none());
        assert!(score.barlines.is_empty());
    }

    #[test]
    fn bass_staff_maps_bottom_line_to_g2() {
        let staff = Staff {
            lines: [20.0, 30.0, 40.0, 50.0, 60.0],
            left: 10.0,
            right: 200.0,
            header: StaffHeader::default(),
        };
        assert_eq!(staff.pitch_at_with_clef(60.0, Clef::Bass), 18);
        assert_eq!(staff.pitch_at_with_clef(30.0, Clef::Bass), 24);
        assert_eq!(staff.pitch_at_with_clef(60.0, Clef::Treble), 30);
    }

    #[test]
    fn treble_staff_maps_lines_and_spaces() {
        let staff = Staff {
            lines: [20.0, 30.0, 40.0, 50.0, 60.0],
            left: 10.0,
            right: 200.0,
            header: StaffHeader::default(),
        };
        let mut score = Score {
            staves: vec![staff],
            notes: vec![],
            rests: vec![],
            barlines: vec![],
            spans: vec![],
            key_changes: vec![],
            time_changes: vec![],
            dynamics: vec![],
            repeat_marks: vec![],
        };
        score.add_note(40.0, 60.0);
        score.add_note(60.0, 55.0);
        score.add_note(80.0, 70.0);
        assert_eq!(score.notes[0].pitch_label(), "E4");
        assert_eq!(score.notes[1].pitch_label(), "F4");
        assert_eq!(score.notes[2].pitch_label(), "C4");
        assert!(score.validate().is_ok());
    }

    #[test]
    fn project_roundtrip_preserves_corrections() {
        let mut score = Score {
            staves: vec![Staff {
                lines: [20.0, 30.0, 40.0, 50.0, 60.0],
                left: 10.0,
                right: 200.0,
                header: StaffHeader::default(),
            }],
            notes: vec![],
            rests: vec![],
            barlines: vec![],
            spans: vec![],
            key_changes: vec![],
            time_changes: vec![],
            dynamics: vec![],
            repeat_marks: vec![],
        };
        score.add_note(40.0, 60.0);
        score.notes[0].articulation = Articulation::Staccato;
        score.notes[0].dots = 2;
        score.notes[0].triplet = true;
        let project = Project {
            version: 1,
            image_name: "scan.png".into(),
            image_png: vec![1, 2],
            score,
        };
        let json = serde_json::to_vec(&project).unwrap();
        let restored: Project = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored.score.notes[0].articulation, Articulation::Staccato);
        assert_eq!(restored.score.notes[0].dots, 2);
        assert!(restored.score.notes[0].triplet);
        assert_eq!(restored.score.notes[0].duration_label(), "1/4.. (3:2)");
        assert!(restored.score.notes[0].confirmed);
        assert!(restored.score.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_staff_references() {
        let mut score = Score::default();
        assert!(score.add_note(10.0, 10.0).is_none());
        score.staves.push(Staff {
            lines: [0.0; 5],
            left: 10.0,
            right: 20.0,
            header: StaffHeader::default(),
        });
        assert!(score.validate().is_err());
    }
}
