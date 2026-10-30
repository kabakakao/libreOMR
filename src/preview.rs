use libreomr::i18n;
use std::{collections::HashMap, sync::OnceLock};

use ab_glyph::{Font, FontRef};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use libreomr::model::{
    Articulation, BarlineKind, Clef, Duration, RepeatMark, RepeatMarkKind, Score, Span, SpanKind,
    TimeSignature,
};
use libreomr::rhythm::MeasureValidity;

const GAP: f32 = 12.0;
pub(crate) fn paint_measure_validity(
    ui: &egui::Ui,
    measure: &MeasureValidity,
    rect: Rect,
    id: egui::Id,
) {
    if let Some(valid) = measure.is_valid() {
        let color = if valid {
            Color32::from_rgba_unmultiplied(0, 155, 85, 90)
        } else {
            Color32::from_rgba_unmultiplied(220, 45, 45, 90)
        };
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(rect.left_top(), Color32::TRANSPARENT);
        mesh.colored_vertex(rect.right_top(), color);
        mesh.colored_vertex(rect.right_bottom(), color);
        mesh.colored_vertex(rect.left_bottom(), Color32::TRANSPARENT);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(mesh));
    }
    ui.interact(rect, id, Sense::hover())
        .on_hover_text(measure.label());
}

pub(crate) fn paint_span(
    painter: &egui::Painter,
    span: &Span,
    map: impl Fn([f32; 2]) -> Pos2,
    color: Color32,
    thickness: f32,
) {
    if matches!(span.kind, SpanKind::Slur | SpanKind::Tie) {
        let points = (0..=48)
            .map(|sample| map(span.curve_point(sample as f32 / 48.0)))
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(thickness, color)));
    } else {
        for side in [-1.0, 1.0] {
            let start = span.wedge_point(0.0, side);
            let end = span.wedge_point(1.0, side);
            painter.line_segment([map(start), map(end)], Stroke::new(thickness, color));
        }
    }
}

pub(crate) fn paint_ending(
    painter: &egui::Painter,
    mark: &RepeatMark,
    gap: f32,
    map: impl Fn([f32; 2]) -> Pos2,
    color: Color32,
    scale: f32,
) {
    let start = map([mark.x, mark.y]);
    let end = map([mark.end_x, mark.y]);
    let hook = map([mark.x, mark.y + gap]) - start;
    let stroke = Stroke::new((1.2 * scale).max(0.8), color);
    painter.line_segment([start, end], stroke);
    painter.line_segment([start, start + hook], stroke);
    if mark.closed {
        painter.line_segment([end, end + hook], stroke);
    }
    painter.text(
        start + Vec2::new(4.0, 2.0) * scale,
        egui::Align2::LEFT_TOP,
        if mark.numbers.is_empty() {
            "?"
        } else {
            &mark.numbers
        },
        egui::FontId::proportional(13.0 * scale),
        color,
    );
}

fn music_font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(include_bytes!("../assets/Bravura.otf"))
            .expect("embedded Bravura font")
    })
}

// Bravura's outlines are shared with the recognition templates, but preview
// pixels use tiny-skia: ab_glyph_rasterizer corrupts some fractional-size CFF
// glyphs (notably flats). Keep the mask transparent and tint it when painting.
fn rasterize_music_glyph(character: char, scale: f32) -> (egui::ColorImage, Vec2) {
    let font = music_font();
    let id = font.glyph_id(character);
    let bounds = font
        .outline_glyph(id.with_scale(scale))
        .expect("SMuFL glyph")
        .px_bounds();
    let factor = scale / font.height_unscaled();
    let map = |point: ab_glyph::Point| {
        tiny_skia::Point::from_xy(
            point.x * factor - bounds.min.x,
            -point.y * factor - bounds.min.y,
        )
    };
    let mut path = tiny_skia::PathBuilder::new();
    let mut previous = None;
    for curve in font.outline(id).expect("SMuFL outline").curves {
        use ab_glyph::OutlineCurve;
        let start = match curve {
            OutlineCurve::Line(start, _)
            | OutlineCurve::Quad(start, _, _)
            | OutlineCurve::Cubic(start, _, _, _) => start,
        };
        if previous != Some(start) {
            if previous.is_some() {
                path.close();
            }
            let start = map(start);
            path.move_to(start.x, start.y);
        }
        previous = Some(match curve {
            OutlineCurve::Line(_, end) => {
                let point = map(end);
                path.line_to(point.x, point.y);
                end
            }
            OutlineCurve::Quad(_, control, end) => {
                let control = map(control);
                let point = map(end);
                path.quad_to(control.x, control.y, point.x, point.y);
                end
            }
            OutlineCurve::Cubic(_, first, second, end) => {
                let first = map(first);
                let second = map(second);
                let point = map(end);
                path.cubic_to(first.x, first.y, second.x, second.y, point.x, point.y);
                end
            }
        });
    }
    path.close();
    let path = path.finish().expect("SMuFL path");
    let size = [bounds.width() as usize, bounds.height() as usize];
    let mut pixels =
        tiny_skia::Pixmap::new(size[0] as u32, size[1] as u32).expect("SMuFL pixel bounds");
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    pixels.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    let image = egui::ColorImage::new(
        size,
        pixels
            .pixels()
            .iter()
            .map(|pixel| Color32::from_white_alpha(pixel.alpha()))
            .collect(),
    );
    (image, Vec2::new(bounds.min.x, bounds.min.y))
}

fn notation_scale() -> f32 {
    let font = music_font();
    let head = font
        .outline_glyph(font.glyph_id('\u{e0a4}').with_scale(160.0))
        .expect("preview notehead");
    160.0 * GAP * 0.85 / head.px_bounds().height()
}

fn notation_height(character: char) -> f32 {
    let font = music_font();
    let outline = font
        .outline_glyph(font.glyph_id(character).with_scale(160.0))
        .expect("preview SMuFL glyph");
    outline.px_bounds().height() * notation_scale() / 160.0
}

fn notation_width(character: char, height: f32) -> f32 {
    let font = music_font();
    let bounds = font
        .outline_glyph(font.glyph_id(character).with_scale(160.0))
        .expect("preview SMuFL glyph")
        .px_bounds();
    bounds.width() * height / bounds.height()
}

fn crisp_line(ui: &egui::Ui, mut from: Pos2, mut to: Pos2, thickness: f32, color: Color32) {
    let density = ui.ctx().pixels_per_point();
    let pixels = (thickness * density).round().max(1.0);
    let align =
        |coordinate: f32| ((coordinate * density - pixels * 0.5).round() + pixels * 0.5) / density;
    if from.x == to.x {
        from.x = align(from.x);
        to.x = from.x;
    }
    if from.y == to.y {
        from.y = align(from.y);
        to.y = from.y;
    }
    ui.painter()
        .line_segment([from, to], Stroke::new(pixels / density, color));
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Selection {
    Note(usize),
    Rest(usize),
    KeyChange(usize),
    TimeChange(usize),
    Dynamic(usize),
    RepeatMark(usize),
    Barline(usize),
    Span(usize),
}

pub(crate) struct Preview {
    glyphs: HashMap<(char, u32), Glyph>,
    pub(crate) selection: Vec<Selection>,
    pub(crate) fit_width: bool,
    scale: f32,
    anchor_drag: Option<AnchorDrag>,
    anchor_edit: Option<AnchorEdit>,
    document_layout: Option<ScoreLayout>,
    document_scale: f32,
    zoom: f32,
    last_follow: Option<FollowTarget>,
    last_view: Vec2,
    last_fit: bool,
    #[cfg(test)]
    canvas_view: Option<(egui::Id, Rect, Vec2, Vec2)>,
}

pub(crate) struct AnchorEdit {
    pub index: usize,
    pub right: bool,
    pub position: [f32; 2],
    pub started: bool,
    pub stopped: bool,
}

struct AnchorDrag {
    index: usize,
    right: bool,
    source: [f32; 2],
    screen: Pos2,
    source_per_y: f32,
    delta: Vec2,
    // Freeze the mapping while dragging: notation spacing differs from scan spacing.
    x_positions: Vec<(f32, f32)>,
    upper: f32,
    lower: f32,
    leftmost: f32,
    natural_width: f32,
    scale: f32,
}

impl AnchorDrag {
    fn position(&self, delta: Vec2) -> [f32; 2] {
        let source_x = |screen: f32| {
            let next = self.x_positions.partition_point(|&(x, _)| x < screen);
            let next = next.clamp(1, self.x_positions.len() - 1);
            let (left, start) = self.x_positions[next - 1];
            let (right, end) = self.x_positions[next];
            start + (end - start) * (screen - left) / (right - left)
        };
        [
            self.source[0] + source_x(self.screen.x + delta.x) - source_x(self.screen.x),
            self.source[1] + delta.y * self.source_per_y,
        ]
    }
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            glyphs: HashMap::new(),
            selection: vec![],
            fit_width: false,
            scale: 1.0,
            anchor_drag: None,
            anchor_edit: None,
            document_layout: None,
            document_scale: 1.0,
            zoom: 1.0,
            last_follow: None,
            last_view: Vec2::ZERO,
            last_fit: false,
            #[cfg(test)]
            canvas_view: None,
        }
    }
}

struct Glyph {
    texture: egui::TextureHandle,
    origin: Vec2,
}

#[derive(Clone, Copy)]
enum Item {
    Note(usize),
    Rest(usize),
    Bar(usize),
    KeyChange(usize),
    TimeChange(usize),
}

#[derive(Clone)]
struct RowLayout {
    staff: usize,
    items: Vec<(f32, Item)>,
    positioned: Vec<(f32, Item)>,
    measures: Vec<MeasureValidity>,
    time_signature: Option<TimeSignature>,
    key_count: u8,
    header_width: f32,
    upper: f32,
    lower: f32,
    lyric_top: f32,
    lyric_above: f32,
    measure_end: f32,
    staff_end: f32,
    leftmost: f32,
    natural_width: f32,
    axis: Option<SystemAxis>,
}

impl RowLayout {
    fn height(&self) -> f32 {
        (self.lower - self.upper) * GAP + 16.0
    }

    fn map_x(&self, source: f32, staff: &libreomr::model::Staff) -> f32 {
        if let Some(axis) = &self.axis {
            return axis.map_x(source);
        }
        if self.items.is_empty() {
            return self.header_width + (source - staff.left) / staff.spacing() * GAP;
        }
        for index in 1..self.items.len() {
            if source <= self.items[index].0 && self.items[index].0 > self.items[index - 1].0 {
                let fraction = (source - self.items[index - 1].0)
                    / (self.items[index].0 - self.items[index - 1].0);
                return self.positioned[index - 1].0
                    + (self.positioned[index].0 - self.positioned[index - 1].0) * fraction;
            }
        }
        let index = if source <= self.items[0].0 {
            0
        } else {
            self.items.len() - 1
        };
        self.positioned[index].0 + (source - self.items[index].0) / staff.spacing() * GAP
    }
}

#[derive(Clone)]
struct SystemAxis {
    points: Vec<(f32, f32)>,
    spacing: f32,
    header: f32,
}

impl SystemAxis {
    fn map_x(&self, source: f32) -> f32 {
        let next = self.points.partition_point(|&(x, _)| x < source);
        if next > 0 && next < self.points.len() {
            let (a, x) = self.points[next - 1];
            let (b, y) = self.points[next];
            return x + (y - x) * (source - a) / (b - a);
        }
        let &(x, y) = if next == 0 {
            self.points.first().unwrap()
        } else {
            self.points.last().unwrap()
        };
        y + (source - x) / self.spacing * GAP
    }
}

#[derive(Clone)]
struct SystemLayout {
    rows: Vec<(RowLayout, Rect)>,
    rect: Rect,
}

#[derive(Clone)]
struct ScoreLayout {
    systems: Vec<SystemLayout>,
    size: Vec2,
}

#[derive(Clone, Copy, PartialEq)]
struct FollowTarget {
    selected: Option<Selection>,
    staff: usize,
    position: [f32; 2],
}

fn key_change_symbols(score: &Score, index: usize) -> Vec<(char, i32)> {
    let change = &score.key_changes[index];
    let previous = score
        .key_changes
        .iter()
        .filter(|candidate| candidate.staff == change.staff && candidate.x < change.x)
        .max_by(|first, second| first.x.total_cmp(&second.x))
        .map(|candidate| candidate.key_signature)
        .or(score.staves[change.staff].header.key_signature)
        .unwrap_or_default();
    let order = |fifths: i8| {
        if fifths < 0 {
            [34, 37, 33, 36, 32, 35, 31]
        } else {
            [38, 35, 39, 36, 33, 37, 34]
        }
    };
    let next = change.key_signature;
    let keep = if previous.fifths.signum() == next.fifths.signum() {
        next.fifths.unsigned_abs() as usize
    } else {
        0
    };
    let mut symbols: Vec<_> = order(previous.fifths)
        .into_iter()
        .take(previous.fifths.unsigned_abs() as usize)
        .skip(keep)
        .map(|pitch| ('\u{e261}', pitch))
        .collect();
    symbols.extend(
        order(next.fifths)
            .into_iter()
            .take(next.fifths.unsigned_abs() as usize)
            .map(|pitch| {
                (
                    if next.fifths < 0 {
                        '\u{e260}'
                    } else {
                        '\u{e262}'
                    },
                    pitch,
                )
            }),
    );
    symbols
}

impl Item {
    fn width(self, score: &Score) -> f32 {
        match self {
            Self::Note(index) => 32.0 + f32::from(score.notes[index].dots) * 7.0,
            Self::Rest(index) if score.rests[index].measures.is_some() => 92.0,
            Self::Rest(index) => 32.0 + f32::from(score.rests[index].dots) * 7.0,
            Self::Bar(_) => 24.0,
            Self::KeyChange(index) => key_change_symbols(score, index).len() as f32 * 9.0 + 24.0,
            Self::TimeChange(index) => match score.time_changes[index].time_signature {
                TimeSignature::Meter { beats, beat_unit } => {
                    beats.to_string().len().max(beat_unit.to_string().len()) as f32 * 10.0 + 24.0
                }
                _ => 48.0,
            },
        }
    }
}

impl Preview {
    pub(crate) fn fitted() -> Self {
        Self {
            fit_width: true,
            ..Self::default()
        }
    }

    fn glyph(
        &mut self,
        ui: &egui::Ui,
        character: char,
        at: Pos2,
        height: f32,
        anchor: Vec2,
        color: Color32,
    ) -> Vec2 {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let pixel_height = (height * self.scale * pixels_per_point).round().max(1.0) as u32;
        let glyph = self
            .glyphs
            .entry((character, pixel_height))
            .or_insert_with(|| {
                let font = music_font();
                let reference = font
                    .outline_glyph(font.glyph_id(character).with_scale(160.0))
                    .expect("preview SMuFL glyph");
                let scale = if matches!(character, '\u{e0a2}'..='\u{e0a4}' | '\u{e240}'..='\u{e247}' | '\u{e260}'..='\u{e262}') {
                    notation_scale() * self.scale * pixels_per_point
                } else {
                    160.0 * pixel_height as f32 / reference.px_bounds().height()
                };
                let (image, origin) = rasterize_music_glyph(character, scale);
                let texture = ui.ctx().load_texture(
                    format!("preview-{character}-{pixel_height}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                Glyph {
                    texture,
                    origin,
                }
            });
        let size = glyph.texture.size_vec2() / pixels_per_point;
        let minimum = if matches!(character, '\u{e240}'..='\u{e247}') {
            at + glyph.origin / pixels_per_point
        } else {
            at - size * anchor
        };
        let minimum = Pos2::new(
            (minimum.x * pixels_per_point).round() / pixels_per_point,
            (minimum.y * pixels_per_point).round() / pixels_per_point,
        );
        let rect = Rect::from_min_size(minimum, size);
        ui.painter().image(
            glyph.texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            color,
        );
        size
    }

    #[cfg(test)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        staff_index: usize,
        selected: Option<Selection>,
    ) -> Option<Selection> {
        self.show_with_inherited(ui, score, staff_index, selected, None)
    }

    #[cfg(test)]
    pub(crate) fn show_with_inherited(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        staff_index: usize,
        selected: Option<Selection>,
        inherited: Option<TimeSignature>,
    ) -> Option<Selection> {
        self.anchor_edit = None;
        let clicked = self.show_score(ui, score, staff_index, selected, false, inherited);
        if !ui.input(|input| input.pointer.primary_down()) {
            self.anchor_drag = None;
        }
        clicked
    }

    pub(crate) fn zoom_by(&mut self, factor: f32) {
        let current = if self.fit_width {
            self.document_scale
        } else {
            self.zoom
        };
        self.zoom = (current * factor).clamp(0.15, 2.5);
        self.fit_width = false;
        self.last_follow = None;
    }

    fn score_layout(
        ui: &mut egui::Ui,
        score: &Score,
        inherited: Option<TimeSignature>,
    ) -> ScoreLayout {
        let mut order: Vec<_> = (0..score.staves.len()).collect();
        order.sort_by(|&a, &b| score.staves[a].lines[0].total_cmp(&score.staves[b].lines[0]));
        let system_of = |staff| {
            score
                .staff_assignments
                .iter()
                .find(|a| a.staff == staff)
                .map(|a| a.system)
        };
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for staff in order {
            if system_of(staff).is_some()
                && groups
                    .last()
                    .and_then(|g| g.last())
                    .is_some_and(|&prior| system_of(prior) == system_of(staff))
            {
                groups.last_mut().unwrap().push(staff);
            } else {
                groups.push(vec![staff]);
            }
        }
        let mut systems = Vec::new();
        let mut y = 8.0_f32;
        let mut width = 1.0_f32;
        let mut lyric_widths: HashMap<usize, f32> = HashMap::new();
        for lyric in &score.lyrics {
            let width = ui
                .painter()
                .layout_no_wrap(
                    lyric.text.clone(),
                    egui::FontId::proportional(13.0),
                    Color32::BLACK,
                )
                .size()
                .x
                + 14.0;
            lyric_widths
                .entry(lyric.note)
                .and_modify(|w| *w = w.max(width))
                .or_insert(width);
        }
        for group in groups {
            let mut rows: Vec<_> = group
                .into_iter()
                .filter_map(|s| Self::layout_row(ui, score, s, false, inherited))
                .collect();
            if rows.is_empty() {
                continue;
            }
            let spacing = rows
                .iter()
                .map(|r| score.staves[r.staff].spacing())
                .sum::<f32>()
                / rows.len() as f32;
            let header = rows
                .iter()
                .map(|r| r.positioned.first().map_or(r.header_width, |p| p.0))
                .fold(0.0, f32::max);
            let mut anchors = Vec::new();
            for row in &rows {
                for &(x, item) in &row.items {
                    let mut item_width = item.width(score);
                    if let Item::Note(note) = item {
                        item_width =
                            item_width.max(lyric_widths.get(&note).copied().unwrap_or(0.0));
                    }
                    anchors.push((x, item_width));
                }
                anchors.push((score.staves[row.staff].right, 24.0));
            }
            anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut columns: Vec<(f32, f32)> = Vec::new();
            for (x, w) in anchors {
                if let Some(last) = columns.last_mut()
                    && (last.0 - x).abs() < spacing * 0.35
                {
                    last.1 = last.1.max(w);
                } else {
                    columns.push((x, w));
                }
            }
            let mut points = Vec::new();
            let mut cursor = header;
            for (i, &(x, w)) in columns.iter().enumerate() {
                if i > 0 {
                    let minimum = (w + columns[i - 1].1) * 0.5;
                    cursor += ((x - columns[i - 1].0) / spacing * GAP)
                        .clamp(minimum, 180.0_f32.max(minimum));
                }
                points.push((x, cursor));
            }
            let axis = SystemAxis {
                points,
                spacing,
                header,
            };
            let leftmost = rows.iter().map(|r| r.leftmost).fold(0.0, f32::min);
            let staff_end = axis.points.last().unwrap().1 + GAP * 2.0;
            let natural_width = staff_end + 8.0 - leftmost;
            let start = y;
            let mut laid_out = Vec::new();
            for mut row in rows.drain(..) {
                for (position, item) in &mut row.positioned {
                    let source = match *item {
                        Item::Note(i) => score.notes[i].x,
                        Item::Rest(i) => score.rests[i].x,
                        Item::Bar(i) => score.barlines[i].x,
                        Item::KeyChange(i) => score.key_changes[i].x,
                        Item::TimeChange(i) => score.time_changes[i].x,
                    };
                    *position = axis.map_x(source);
                }
                row.header_width = axis.header;
                row.leftmost = leftmost;
                row.staff_end = staff_end;
                row.measure_end = axis.map_x(score.staves[row.staff].right);
                row.natural_width = natural_width;
                row.axis = Some(axis.clone());
                let rect =
                    Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(natural_width, row.height()));
                y = rect.bottom();
                laid_out.push((row, rect));
            }
            width = width.max(natural_width);
            systems.push(SystemLayout {
                rows: laid_out,
                rect: Rect::from_min_max(Pos2::new(0.0, start), Pos2::new(natural_width, y)),
            });
            y += 24.0;
        }
        ScoreLayout {
            systems,
            size: Vec2::new(width, y),
        }
    }

    fn follow_target(score: &Score, staff: usize, selected: Option<Selection>) -> FollowTarget {
        let (staff, position) = match selected {
            Some(Selection::Note(i)) => score.notes.get(i).map(|n| (n.staff, [n.x, n.y])),
            Some(Selection::Rest(i)) => score.rests.get(i).map(|n| (n.staff, [n.x, n.y])),
            Some(Selection::Dynamic(i)) => score.dynamics.get(i).map(|n| (n.staff, [n.x, n.y])),
            Some(Selection::RepeatMark(i)) => {
                score.repeat_marks.get(i).map(|n| (n.staff, [n.x, n.y]))
            }
            Some(Selection::Span(i)) => score
                .spans
                .get(i)
                .map(|n| (n.staff, score.resolved_span(n).start)),
            Some(Selection::Barline(i)) => score
                .barlines
                .get(i)
                .map(|n| (n.staff, [n.x, score.staves[n.staff].lines[2]])),
            Some(Selection::KeyChange(i)) => score
                .key_changes
                .get(i)
                .map(|n| (n.staff, [n.x, score.staves[n.staff].lines[2]])),
            Some(Selection::TimeChange(i)) => score
                .time_changes
                .get(i)
                .map(|n| (n.staff, [n.x, score.staves[n.staff].lines[2]])),
            None => None,
        }
        .unwrap_or_else(|| {
            (
                staff,
                score
                    .staves
                    .get(staff)
                    .map_or([0.0, 0.0], |s| [(s.left + s.right) * 0.5, s.lines[2]]),
            )
        });
        FollowTarget {
            selected,
            staff,
            position,
        }
    }

    pub(crate) fn show_document(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        staff: usize,
        selected: Option<Selection>,
        inherited: Option<TimeSignature>,
    ) -> Option<Selection> {
        if score.staves.is_empty() {
            ui.label(i18n::tr("No staff"));
            return None;
        }
        let dragging = self.anchor_drag.is_some();
        let layout = if dragging && let Some(layout) = &self.document_layout {
            layout.clone()
        } else {
            Self::score_layout(ui, score, inherited)
        };
        let target = Self::follow_target(score, staff, selected);
        let system = layout
            .systems
            .iter()
            .find(|s| s.rows.iter().any(|(r, _)| r.staff == target.staff))?;
        let viewport = ui.available_size();
        let scale = if self.fit_width {
            ((viewport.x - 16.0).max(1.0) / layout.size.x).clamp(0.03, 1.5)
        } else {
            self.zoom
        };
        let scale = if dragging { self.document_scale } else { scale };
        let follow = !dragging
            && (self.last_follow != Some(target)
                || self.last_view != viewport
                || self.last_fit != self.fit_width);
        self.document_scale = scale;
        self.anchor_edit = None;
        let mut clicked = None;
        let output = egui::ScrollArea::both()
            .id_salt("score_preview_canvas")
            .auto_shrink([false, false])
            .animated(false)
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: false,
                ..Default::default()
            })
            .show(ui, |ui| {
                let content = layout.size * scale;
                let (background, _) = ui.allocate_exact_size(
                    Vec2::new(content.x.max(ui.available_width()), content.y),
                    Sense::hover(),
                );
                ui.painter().rect_filled(background, 0.0, Color32::WHITE);
                let canvas = Rect::from_min_size(
                    background.min + Vec2::new((background.width() - content.x) * 0.5, 0.0),
                    content,
                );
                let screen = |rect: Rect| {
                    Rect::from_min_max(
                        canvas.min + rect.min.to_vec2() * scale,
                        canvas.min + rect.max.to_vec2() * scale,
                    )
                };
                if follow {
                    let focus = {
                        let (row, rect) = system
                            .rows
                            .iter()
                            .find(|(r, _)| r.staff == target.staff)
                            .unwrap();
                        let x = row.map_x(target.position[0], &score.staves[target.staff])
                            - row.leftmost;
                        let mut focus = Rect::from_min_size(
                            Pos2::new((x - 80.0).max(0.0), rect.top()),
                            Vec2::new(160.0, rect.height()),
                        );
                        let note_level = (target.position[1] - score.staves[target.staff].lines[0])
                            / score.staves[target.staff].spacing();
                        if focus.height() * scale > viewport.y {
                            let y = rect.top() + 8.0 + (note_level - row.upper) * GAP;
                            focus = Rect::from_center_size(
                                Pos2::new(x, y),
                                Vec2::new(160.0, (viewport.y / scale - 12.0).max(24.0)),
                            );
                        }
                        screen(focus)
                    };
                    ui.scroll_to_rect(focus, Some(egui::Align::Center));
                }
                for group in &layout.systems {
                    for (row, bounds) in &group.rows {
                        let rect = screen(*bounds);
                        if ui.is_rect_visible(rect) {
                            ui.push_id(("score_staff", row.staff), |ui| {
                                clicked = self
                                    .paint_row(ui, score, row, rect, scale, selected, false)
                                    .or(clicked);
                            });
                        }
                    }
                    if group.rows.len() > 1 {
                        let (first, first_rect) = &group.rows[0];
                        let (last, last_rect) = group.rows.last().unwrap();
                        let top = canvas.min
                            + Vec2::new(8.0, first_rect.top() + 8.0 - first.upper * GAP) * scale;
                        let bottom = canvas.min
                            + Vec2::new(8.0, last_rect.top() + 8.0 + (4.0 - last.upper) * GAP)
                                * scale;
                        ui.painter()
                            .line_segment([top, bottom], Stroke::new(scale, Color32::DARK_GRAY));
                        for p in [top, bottom] {
                            ui.painter().line_segment(
                                [p, p + Vec2::new(4.0 * scale, 0.0)],
                                Stroke::new(scale, Color32::DARK_GRAY),
                            );
                        }
                    }
                }
                if selected.is_some()
                    && let Some((row, _)) =
                        system.rows.iter().find(|(r, _)| r.staff == target.staff)
                {
                    let x =
                        row.map_x(target.position[0], &score.staves[target.staff]) - row.leftmost;
                    let band = Rect::from_min_max(
                        Pos2::new(x - 10.0, system.rect.top()),
                        Pos2::new(x + 10.0, system.rect.bottom()),
                    );
                    ui.painter().rect_filled(
                        screen(band),
                        0.0,
                        Color32::from_rgba_unmultiplied(0, 95, 180, 12),
                    );
                }
            });
        // Store viewport geometry, not content size: manual panning must not re-trigger following.
        #[cfg(test)]
        {
            self.canvas_view = Some((
                output.id,
                output.inner_rect,
                output.state.offset,
                output.content_size,
            ));
        }
        #[cfg(not(test))]
        let _ = output;
        if !dragging {
            self.last_follow = Some(target);
            self.last_view = viewport;
            self.last_fit = self.fit_width;
        }
        self.document_layout = Some(layout);
        if !ui.input(|i| i.pointer.primary_down()) {
            self.anchor_drag = None;
        }
        clicked
    }

    pub(crate) fn take_anchor_edit(&mut self) -> Option<AnchorEdit> {
        self.anchor_edit.take()
    }

    pub(crate) fn show_object(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        selected: Selection,
        max_height: f32,
    ) {
        let Some(isolated) = Self::isolate_object(score, selected) else {
            return;
        };
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), max_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                self.show_score(ui, &isolated, 0, None, true, None);
            },
        );
    }

    fn paint_time_signature(
        &mut self,
        ui: &egui::Ui,
        time: TimeSignature,
        at: Pos2,
        scale: f32,
        color: Color32,
    ) {
        match time {
            TimeSignature::Common | TimeSignature::Cut => {
                self.glyph(
                    ui,
                    if time == TimeSignature::Common {
                        '\u{e08a}'
                    } else {
                        '\u{e08b}'
                    },
                    at,
                    GAP * 2.0,
                    Vec2::new(0.0, 0.5),
                    color,
                );
            }
            TimeSignature::Meter { beats, beat_unit } => {
                for (number, level) in [(beats, 0.1), (beat_unit, 2.1)] {
                    let digits = number.to_string();
                    for (index, digit) in digits.chars().enumerate() {
                        self.glyph(
                            ui,
                            char::from_u32(0xe080 + digit.to_digit(10).unwrap()).unwrap(),
                            at + Vec2::new(index as f32 * 10.0, (level - 2.0) * GAP) * scale,
                            GAP * 1.8,
                            Vec2::ZERO,
                            color,
                        );
                    }
                }
            }
        }
    }

    fn isolate_object(score: &Score, selected: Selection) -> Option<Score> {
        let mut isolated = Score::default();
        let (staff_index, source_x) = match selected {
            Selection::Note(index) => {
                let mut note = score.notes.get(index)?.clone();
                let position = (note.staff, note.x);
                note.staff = 0;
                isolated.notes.push(note);
                position
            }
            Selection::Rest(index) => {
                let mut rest = score.rests.get(index)?.clone();
                let position = (rest.staff, rest.x);
                rest.staff = 0;
                isolated.rests.push(rest);
                position
            }
            Selection::KeyChange(index) => {
                let mut change = score.key_changes.get(index)?.clone();
                let position = (change.staff, change.x);
                change.staff = 0;
                isolated.key_changes.push(change);
                position
            }
            Selection::TimeChange(index) => {
                let mut change = score.time_changes.get(index)?.clone();
                let position = (change.staff, change.x);
                change.staff = 0;
                isolated.time_changes.push(change);
                position
            }
            Selection::Dynamic(index) => {
                let mut dynamic = score.dynamics.get(index)?.clone();
                let position = (dynamic.staff, dynamic.x);
                dynamic.staff = 0;
                isolated.dynamics.push(dynamic);
                position
            }
            Selection::RepeatMark(index) => {
                let mut mark = score.repeat_marks.get(index)?.clone();
                let position = (mark.staff, mark.x);
                mark.staff = 0;
                isolated.repeat_marks.push(mark);
                position
            }
            Selection::Barline(index) => {
                let mut barline = score.barlines.get(index)?.clone();
                let position = (barline.staff, barline.x);
                barline.staff = 0;
                isolated.barlines.push(barline);
                position
            }
            Selection::Span(index) => {
                let mut span = score
                    .span_segments(score.spans.get(index)?)
                    .into_iter()
                    .next()?;
                let position = (span.staff, span.start[0]);
                span.staff = 0;
                span.start_anchor = None;
                span.end_anchor = None;
                span.continuation = None;
                isolated.spans.push(span);
                position
            }
        };
        let mut staff = score.staves.get(staff_index)?.clone();
        staff.header.time_signature = score.time_signature_before(staff_index, source_x);
        if let Some(change) = score
            .key_changes
            .iter()
            .filter(|change| {
                change.staff == staff_index
                    && (change.x < source_x
                        || (change.x == source_x && !matches!(selected, Selection::KeyChange(_))))
            })
            .max_by(|first, second| first.x.total_cmp(&second.x))
        {
            staff.header.key_signature = Some(change.key_signature);
        }
        if let Some(change) = score
            .time_changes
            .iter()
            .filter(|change| {
                change.staff == staff_index
                    && (change.x < source_x
                        || (change.x == source_x && !matches!(selected, Selection::TimeChange(_))))
            })
            .max_by(|a, b| a.x.total_cmp(&b.x))
        {
            staff.header.time_signature = Some(change.time_signature);
        }
        if let Some(note) = isolated.notes.first_mut() {
            note.accidental_explicit |= note.accidental != staff.accidental_at(note.pitch);
        }
        staff.left = source_x;
        isolated.staves.push(staff);
        Some(isolated)
    }

    fn layout_row(
        ui: &mut egui::Ui,
        score: &Score,
        staff_index: usize,
        fit_object: bool,
        inherited: Option<TimeSignature>,
    ) -> Option<RowLayout> {
        let Some(staff) = score.staves.get(staff_index) else {
            ui.label(i18n::tr("No staff"));
            return None;
        };
        let drawable_spans = score.drawable_spans();
        let time_signature =
            score.time_signature_before_with_inherited(staff_index, staff.left, inherited);
        let measures = if fit_object {
            vec![]
        } else {
            score.measure_validities_with_inherited(staff_index, inherited)
        };
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
                    .barlines
                    .iter()
                    .enumerate()
                    .filter(|(_, bar)| bar.staff == staff_index)
                    .map(|(index, bar)| (bar.x, Item::Bar(index))),
            )
            .chain(
                score
                    .key_changes
                    .iter()
                    .enumerate()
                    .filter(|(_, change)| change.staff == staff_index)
                    .map(|(index, change)| (change.x, Item::KeyChange(index))),
            )
            .chain(
                score
                    .time_changes
                    .iter()
                    .enumerate()
                    .filter(|(_, change)| change.staff == staff_index)
                    .map(|(index, change)| (change.x, Item::TimeChange(index))),
            )
            .collect();
        let priority = |item| match item {
            Item::Bar(_) => 0,
            Item::KeyChange(_) | Item::TimeChange(_) => 1,
            _ => 2,
        };
        items.sort_by(|first, second| {
            first
                .0
                .total_cmp(&second.0)
                .then(priority(first.1).cmp(&priority(second.1)))
        });
        let key_count = staff
            .header
            .key_signature
            .map_or(0, |key| key.fifths.unsigned_abs());
        let header_width =
            90.0 + f32::from(key_count) * 9.0 + if time_signature.is_some() { 40.0 } else { 0.0 };
        let mut positioned = Vec::new();
        let mut cursor = header_width;
        if let Some((_, item @ (Item::KeyChange(_) | Item::TimeChange(_)))) = items.first() {
            cursor += item.width(score) * 0.5;
        }
        for (index, &(source_x, item)) in items.iter().enumerate() {
            if index > 0 {
                let (previous_x, previous_item) = items[index - 1];
                let chord = matches!((item, previous_item), (Item::Note(_), Item::Note(_)))
                    && (source_x - previous_x).abs() < staff.spacing() * 0.2;
                if !chord {
                    cursor += ((source_x - previous_x) / staff.spacing() * GAP).clamp(
                        (item.width(score) + previous_item.width(score)) / 2.0,
                        180.0,
                    );
                }
            }
            positioned.push((cursor, item));
        }
        let relative = |pitch| (staff.y_at_pitch(pitch) - staff.lines[0]) / staff.spacing();
        // Reserve space for the actual notation, including the side on which
        // stems and articulations are drawn, rather than padding every pitch
        // by a full stem length in both directions.
        let mut upper = match staff.header.clef {
            Clef::Unknown => -3.2_f32,
            Clef::Treble | Clef::Treble8vb => -1.0,
            Clef::Bass => -0.2,
        };
        let mut lower = if staff.header.clef == Clef::Bass {
            4.5_f32
        } else {
            5.0
        };
        if key_count > 0
            || items
                .iter()
                .any(|(_, item)| matches!(item, Item::KeyChange(_)))
        {
            upper = upper.min(-2.0);
        }
        for note in score.notes.iter().filter(|note| note.staff == staff_index) {
            let level = relative(note.pitch);
            let polyphonic = score
                .notes
                .iter()
                .any(|n| n.staff == note.staff && n.voice > 1)
                || score
                    .rests
                    .iter()
                    .any(|r| r.staff == note.staff && r.voice > 1);
            let down = if polyphonic {
                note.voice.is_multiple_of(2)
            } else {
                level <= 2.0
            };
            upper = upper.min(level - 0.6);
            lower = lower.max(level + 0.6);
            if note.duration != Duration::Whole {
                let stem = 3.3 + note.duration.flag_count().saturating_sub(2) as f32 * 0.5;
                let tip = level + if down { stem } else { -stem };
                upper = upper.min(tip - 0.1);
                lower = lower.max(tip + 0.1);
                if note.duration.flag_count() > 0 {
                    let character = char::from_u32(
                        0xe240 + (note.duration.flag_count() as u32 - 1) * 2 + u32::from(down),
                    )
                    .unwrap();
                    let bounds = music_font()
                        .outline_glyph(
                            music_font()
                                .glyph_id(character)
                                .with_scale(notation_scale()),
                        )
                        .unwrap()
                        .px_bounds();
                    upper = upper.min(tip + bounds.min.y / GAP);
                    lower = lower.max(tip + bounds.max.y / GAP);
                }
            }
            if note.accidental_explicit {
                let character = match note.accidental {
                    -1 => '\u{e260}',
                    1 => '\u{e262}',
                    _ => '\u{e261}',
                };
                let height = notation_height(character) / GAP;
                let anchor = if note.accidental == -1 { 0.78 } else { 0.5 };
                upper = upper.min(level - height * anchor);
                lower = lower.max(level + height * (1.0 - anchor));
            }
            if let Some(ratio) = note.tuplet_ratio() {
                let label = ui.painter().layout_no_wrap(
                    ratio.actual.to_string(),
                    egui::FontId::proportional(12.0),
                    Color32::BLACK,
                );
                upper = upper.min(level - 4.0 - (label.size().y - label.mesh_bounds.top()) / GAP);
            }
            if note.articulation == Articulation::Fermata {
                upper = upper.min(level - 2.7);
            } else if note.articulation != Articulation::None {
                let mark = level + if down { -1.2 } else { 1.2 };
                let half_height = note
                    .articulation
                    .glyph(!down)
                    .map_or(0.35, |character| notation_height(character) / (2.0 * GAP));
                upper = upper.min(mark - half_height);
                lower = lower.max(mark + half_height);
            }
        }
        if score
            .rests
            .iter()
            .any(|rest| rest.staff == staff_index && rest.measures.is_some())
        {
            upper = upper.min(-2.7);
        }
        for dynamic in score
            .dynamics
            .iter()
            .filter(|dynamic| dynamic.staff == staff_index)
        {
            let level = (dynamic.y - staff.lines[0]) / staff.spacing();
            let half_height = notation_height(dynamic.kind.glyph()) / (2.0 * GAP);
            upper = upper.min(level - half_height);
            lower = lower.max(level + half_height);
        }
        for mark in score
            .repeat_marks
            .iter()
            .filter(|mark| mark.staff == staff_index)
        {
            let level = (mark.y - staff.lines[0]) / staff.spacing();
            upper = upper.min(
                level
                    - if mark.kind == RepeatMarkKind::Ending {
                        0.1
                    } else {
                        1.25
                    },
            );
            lower = lower.max(level + 1.4);
        }
        for (_, span) in drawable_spans
            .iter()
            .filter(|(_, span)| span.staff == staff_index)
        {
            if matches!(span.kind, SpanKind::Slur | SpanKind::Tie) {
                for sample in 0..=48 {
                    let point = span.curve_point(sample as f32 / 48.0);
                    let level = (point[1] - staff.lines[0]) / staff.spacing();
                    upper = upper.min(level - 0.1);
                    lower = lower.max(level + 0.1);
                }
            } else {
                for t in [0.0, 1.0] {
                    upper = upper.min(
                        (span.wedge_point(t, -1.0)[1] - staff.lines[0]) / staff.spacing() - 0.1,
                    );
                    lower = lower.max(
                        (span.wedge_point(t, 1.0)[1] - staff.lines[0]) / staff.spacing() + 0.1,
                    );
                }
            }
        }
        let verse_count = score
            .lyrics
            .iter()
            .filter(|l| {
                score.notes[l.note].staff == staff_index
                    && l.placement == libreomr::model::LyricPlacement::Below
            })
            .map(|l| l.verse)
            .max()
            .unwrap_or(0);
        let lyric_top = lower + 2.0;
        if verse_count > 0 {
            lower = lyric_top + f32::from(verse_count) * 1.8;
        }
        let above_count = score
            .lyrics
            .iter()
            .filter(|l| {
                score.notes[l.note].staff == staff_index
                    && l.placement == libreomr::model::LyricPlacement::Above
            })
            .map(|l| l.verse)
            .max()
            .unwrap_or(0);
        let lyric_above = upper - 2.0 - f32::from(above_count.saturating_sub(1)) * 1.8;
        if above_count > 0 {
            upper = lyric_above - 0.9;
        }
        let map_x = |source: f32| {
            if items.is_empty() {
                return header_width + (source - staff.left) / staff.spacing() * GAP;
            }
            for index in 1..items.len() {
                if source <= items[index].0 && items[index].0 > items[index - 1].0 {
                    let fraction =
                        (source - items[index - 1].0) / (items[index].0 - items[index - 1].0);
                    return positioned[index - 1].0
                        + (positioned[index].0 - positioned[index - 1].0) * fraction;
                }
            }
            let index = if source <= items[0].0 {
                0
            } else {
                items.len() - 1
            };
            positioned[index].0 + (source - items[index].0) / staff.spacing() * GAP
        };
        let item_end = positioned
            .iter()
            .map(|&(x, item)| x + item.width(score) * 0.5)
            .fold(header_width, f32::max);
        let measure_end = item_end + GAP;
        let rightmost = drawable_spans
            .iter()
            .filter(|(_, span)| span.staff == staff_index)
            .map(|(_, span)| map_x(span.end[0]))
            .chain(
                score
                    .dynamics
                    .iter()
                    .filter(|dynamic| dynamic.staff == staff_index)
                    .map(|dynamic| {
                        let character = dynamic.kind.glyph();
                        map_x(dynamic.x)
                            + notation_width(character, notation_height(character)) * 0.5
                    }),
            )
            .chain(
                score
                    .repeat_marks
                    .iter()
                    .filter(|mark| mark.staff == staff_index)
                    .map(|mark| {
                        if let Some(character) = mark.kind.glyph() {
                            map_x(mark.x) + notation_width(character, GAP * 2.5) * 0.5
                        } else {
                            let label = ui.painter().layout_no_wrap(
                                if mark.numbers.is_empty() {
                                    "?".into()
                                } else {
                                    mark.numbers.clone()
                                },
                                egui::FontId::proportional(13.0),
                                Color32::BLACK,
                            );
                            map_x(mark.end_x).max(map_x(mark.x) + 4.0 + label.size().x)
                        }
                    }),
            )
            .fold(item_end, f32::max);
        let leftmost = drawable_spans
            .iter()
            .filter(|(_, span)| span.staff == staff_index)
            .map(|(_, span)| map_x(span.start[0]) - 12.0)
            .chain(
                score
                    .dynamics
                    .iter()
                    .filter(|dynamic| dynamic.staff == staff_index)
                    .map(|dynamic| {
                        let character = dynamic.kind.glyph();
                        map_x(dynamic.x)
                            - notation_width(character, notation_height(character)) * 0.5
                    }),
            )
            .chain(
                score
                    .repeat_marks
                    .iter()
                    .filter(|mark| mark.staff == staff_index)
                    .map(|mark| {
                        map_x(mark.x)
                            - mark
                                .kind
                                .glyph()
                                .map_or(1.0, |character| notation_width(character, GAP * 2.5) * 0.5)
                    }),
            )
            .fold(0.0_f32, f32::min);
        let staff_end = rightmost + GAP;
        let natural_width = (staff_end + 8.0 - leftmost).max(1.0);
        Some(RowLayout {
            staff: staff_index,
            items,
            positioned,
            measures,
            time_signature,
            key_count,
            header_width,
            upper,
            lower,
            lyric_top,
            lyric_above,
            measure_end,
            staff_end,
            leftmost,
            natural_width,
            axis: None,
        })
    }

    fn show_score(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        staff_index: usize,
        selected: Option<Selection>,
        fit_object: bool,
        inherited: Option<TimeSignature>,
    ) -> Option<Selection> {
        let mut layout = Self::layout_row(ui, score, staff_index, fit_object, inherited)?;
        let fit_width = self.fit_width || fit_object;
        self.scale = if fit_width {
            ((ui.available_width() - 12.0).max(1.0) / layout.natural_width).clamp(0.01, 1.0)
        } else {
            1.0
        };
        if fit_object {
            self.scale = self.scale.min(
                ((ui.available_height() - 16.0).max(1.0) / ((layout.lower - layout.upper) * GAP))
                    .clamp(0.01, 1.0),
            );
        }
        if !fit_object
            && let Some(drag) = &self.anchor_drag
            && score
                .spans
                .get(drag.index)
                .is_some_and(|span| span.staff == staff_index)
        {
            layout.upper = drag.upper;
            layout.lower = drag.lower;
            layout.leftmost = drag.leftmost;
            layout.natural_width = drag.natural_width;
            self.scale = drag.scale;
        }
        let scale = self.scale;
        egui::ScrollArea::new([!fit_width, !fit_object])
            .id_salt(("preview_scroll", staff_index, fit_width, fit_object))
            // Object previews use only the fitted notation height, even when
            // their parent offers more room. Document previews keep scrolling.
            .auto_shrink([false, fit_object])
            .show(ui, |ui| {
                let width = (layout.natural_width * scale).max(ui.available_width());
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(width, (layout.lower - layout.upper) * GAP * scale + 16.0),
                    Sense::hover(),
                );
                self.paint_row(ui, score, &layout, rect, scale, selected, fit_object)
            })
            .inner
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &mut self,
        ui: &mut egui::Ui,
        score: &Score,
        layout: &RowLayout,
        rect: Rect,
        scale: f32,
        selected: Option<Selection>,
        fit_object: bool,
    ) -> Option<Selection> {
        let RowLayout {
            staff: staff_index,
            items,
            positioned,
            measures,
            time_signature,
            key_count,
            header_width,
            upper,
            lower,
            lyric_top,
            lyric_above,
            measure_end,
            staff_end,
            leftmost,
            natural_width,
            ..
        } = layout.clone();
        let staff = &score.staves[staff_index];
        let drawable_spans = score.drawable_spans();
        let relative = |pitch| (staff.y_at_pitch(pitch) - staff.lines[0]) / staff.spacing();
        let map_x = |source| layout.map_x(source, staff);
        self.scale = scale;
        let mut clicked = None;
        ui.painter().rect_filled(rect, 0.0_f32, Color32::WHITE);
        let padding = if layout.axis.is_some() {
            8.0 * scale
        } else {
            8.0
        };
        let top = rect.top() + padding - upper * GAP * scale;
        let point = |x, level| {
            Pos2::new(
                rect.left() + (x - leftmost) * scale,
                top + level * GAP * scale,
            )
        };
        let line = |from, to, thickness| {
            crisp_line(ui, from, to, thickness * scale, Color32::BLACK);
        };
        for (index, measure) in measures.iter().enumerate() {
            let x = measure
                .barline
                .and_then(|barline| {
                    positioned.iter().find_map(|&(x, item)| {
                        matches!(item, Item::Bar(index) if index == barline).then_some(x)
                    })
                })
                .unwrap_or(measure_end);
            let measure_left = positioned
                .iter()
                .find_map(|&(x, item)| match item {
                    Item::Bar(index) if score.barlines[index].x == measure.left => Some(x),
                    _ => None,
                })
                .unwrap_or(header_width - 16.0);
            paint_measure_validity(
                ui,
                measure,
                Rect::from_min_max(
                    point((x - GAP * 1.8).max(measure_left), -0.3),
                    point(x, 4.3),
                ),
                ui.id().with(("measure_validity", index)),
            );
        }
        for level in 0..5 {
            line(
                point(12.0, level as f32),
                point(staff_end, level as f32),
                0.8_f32,
            );
        }
        let clef = staff.header.clef;
        if let Some(assignment) = score
            .staff_assignments
            .iter()
            .find(|a| a.staff == staff_index)
        {
            ui.painter().text(
                point(54.0, upper + 0.6),
                egui::Align2::LEFT_CENTER,
                &assignment.name,
                egui::FontId::proportional(12.0 * scale),
                Color32::DARK_GRAY,
            );
        }
        if clef == Clef::Unknown {
            ui.painter().text(
                point(24.0, -2.5),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(12.0 * scale),
                Color32::DARK_GRAY,
            );
        }
        self.glyph(
            ui,
            if clef == Clef::Bass {
                '\u{e062}'
            } else {
                '\u{e050}'
            },
            point(20.0, if clef == Clef::Bass { -0.1 } else { -1.0 }),
            GAP * if clef == Clef::Bass { 3.6 } else { 6.0 },
            Vec2::ZERO,
            Color32::BLACK,
        );
        if clef == Clef::Treble8vb {
            ui.painter().text(
                point(28.0, 5.7),
                egui::Align2::CENTER_CENTER,
                "8",
                egui::FontId::proportional(10.0 * scale),
                Color32::BLACK,
            );
        }
        if let Some(key) = staff.header.key_signature {
            let order = if key.fifths >= 0 {
                [38, 35, 39, 36, 33, 37, 34]
            } else {
                [34, 37, 33, 36, 32, 35, 31]
            };
            for (index, pitch) in order
                .iter()
                .take(usize::from(key.fifths.unsigned_abs().min(7)))
                .enumerate()
            {
                let pitch = pitch
                    - match clef {
                        Clef::Bass => 14,
                        Clef::Treble8vb => 7,
                        _ => 0,
                    };
                let character = if key.fifths >= 0 {
                    '\u{e262}'
                } else {
                    '\u{e260}'
                };
                self.glyph(
                    ui,
                    character,
                    point(54.0 + index as f32 * 9.0, relative(pitch)),
                    notation_height(character),
                    Vec2::new(0.0, if key.fifths >= 0 { 0.5 } else { 0.78 }),
                    Color32::BLACK,
                );
            }
        }
        let time_x = 58.0 + f32::from(key_count) * 9.0;
        if let Some(time) = time_signature {
            self.paint_time_signature(ui, time, point(time_x, 2.0), scale, Color32::BLACK);
        }

        for (index, mark) in score
            .repeat_marks
            .iter()
            .enumerate()
            .filter(|(_, mark)| mark.staff == staff_index)
        {
            let color = if selected == Some(Selection::RepeatMark(index))
                || self.selection.contains(&Selection::RepeatMark(index))
            {
                Color32::from_rgb(0, 95, 180)
            } else {
                Color32::BLACK
            };
            let map = |source: [f32; 2]| {
                point(
                    map_x(source[0]),
                    (source[1] - staff.lines[0]) / staff.spacing(),
                )
            };
            let at = map([mark.x, mark.y]);
            let rect = if let Some(character) = mark.kind.glyph() {
                let size = self.glyph(ui, character, at, GAP * 2.5, Vec2::splat(0.5), color);
                Rect::from_center_size(at, size + Vec2::splat(8.0 * scale))
            } else {
                paint_ending(ui.painter(), mark, staff.spacing(), map, color, scale);
                Rect::from_min_max(at, map([mark.end_x, mark.y + staff.spacing() * 1.4]))
                    .expand(4.0 * scale)
            };
            if ui
                .interact(rect, ui.id().with(("repeat_mark", index)), Sense::click())
                .on_hover_text(mark.kind.label())
                .clicked()
            {
                clicked = Some(Selection::RepeatMark(index));
            }
        }
        for (index, dynamic) in score
            .dynamics
            .iter()
            .enumerate()
            .filter(|(_, dynamic)| dynamic.staff == staff_index)
        {
            let color = if selected == Some(Selection::Dynamic(index))
                || self.selection.contains(&Selection::Dynamic(index))
            {
                Color32::from_rgb(0, 95, 180)
            } else {
                Color32::BLACK
            };
            let at = point(
                map_x(dynamic.x),
                (dynamic.y - staff.lines[0]) / staff.spacing(),
            );
            let size = self.glyph(
                ui,
                dynamic.kind.glyph(),
                at,
                notation_height(dynamic.kind.glyph()),
                Vec2::splat(0.5),
                color,
            );
            if ui
                .interact(
                    Rect::from_center_size(at, size + Vec2::splat(8.0 * scale)),
                    ui.id().with(("dynamic", index)),
                    Sense::click(),
                )
                .on_hover_text(i18n::format(
                    "Dynamic {0}",
                    &[("0", dynamic.kind.label().to_string())],
                ))
                .clicked()
            {
                clicked = Some(Selection::Dynamic(index));
            }
        }
        for (index, span) in drawable_spans
            .iter()
            .filter(|(_, span)| span.staff == staff_index)
            .map(|(i, s)| (*i, s))
        {
            let map = |source: [f32; 2]| {
                point(
                    map_x(source[0]),
                    (source[1] - staff.lines[0]) / staff.spacing(),
                )
            };
            paint_span(
                ui.painter(),
                span,
                |source| {
                    point(
                        map_x(source[0]),
                        (source[1] - staff.lines[0]) / staff.spacing(),
                    )
                },
                if selected == Some(Selection::Span(index))
                    || self.selection.contains(&Selection::Span(index))
                {
                    Color32::from_rgb(0, 95, 180)
                } else {
                    Color32::BLACK
                },
                1.3 * scale,
            );
            if !fit_object && let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
                let nearby = (0..=48).any(|sample| {
                    let fraction = sample as f32 / 48.0;
                    let source = if matches!(span.kind, SpanKind::Slur | SpanKind::Tie) {
                        span.curve_point(fraction)
                    } else {
                        [
                            span.start[0] + (span.end[0] - span.start[0]) * fraction,
                            span.start[1] + (span.end[1] - span.start[1]) * fraction,
                        ]
                    };
                    map(source).distance(pointer) <= 8.0
                        || (!span.is_bow()
                            && [-1.0, 1.0].iter().any(|&side| {
                                map(span.wedge_point(fraction, side)).distance(pointer) <= 8.0
                            }))
                });
                if nearby
                    && ui
                        .interact(
                            Rect::from_center_size(pointer, Vec2::splat(16.0)),
                            ui.id().with(("span", index)),
                            Sense::click(),
                        )
                        .on_hover_text(span.kind.label())
                        .clicked()
                {
                    clicked = Some(Selection::Span(index));
                }
            }
        }
        for &(x, item) in &positioned {
            match item {
                Item::Note(index) => {
                    let note = &score.notes[index];
                    let note_scale = note.notation_scale();
                    let level = relative(note.pitch);
                    let at = point(x, level);
                    let color = if selected == Some(Selection::Note(index))
                        || self.selection.contains(&Selection::Note(index))
                    {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::BLACK
                    };
                    for ledger in 1..=(-level).floor().max(0.0) as i32 {
                        line(
                            point(x - 11.0, -ledger as f32),
                            point(x + 11.0, -ledger as f32),
                            0.8_f32,
                        );
                    }
                    for ledger in 1..=(level - 4.0).floor().max(0.0) as i32 {
                        line(
                            point(x - 11.0, 4.0 + ledger as f32),
                            point(x + 11.0, 4.0 + ledger as f32),
                            0.8_f32,
                        );
                    }
                    if note.accidental_explicit {
                        let character = match note.accidental {
                            -1 => '\u{e260}',
                            1 => '\u{e262}',
                            _ => '\u{e261}',
                        };
                        self.glyph(
                            ui,
                            character,
                            at - Vec2::new(13.0 * scale * note_scale, 0.0),
                            notation_height(character) * note_scale,
                            Vec2::new(1.0, if note.accidental == -1 { 0.78 } else { 0.5 }),
                            color,
                        );
                    }
                    let character = match note.duration {
                        Duration::Whole => '\u{e0a2}',
                        Duration::Half => '\u{e0a3}',
                        _ => '\u{e0a4}',
                    };
                    let size = self.glyph(
                        ui,
                        character,
                        at,
                        notation_height(character) * note_scale,
                        Vec2::splat(0.5),
                        color,
                    );
                    let polyphonic = score
                        .notes
                        .iter()
                        .any(|n| n.staff == note.staff && n.voice > 1)
                        || score
                            .rests
                            .iter()
                            .any(|r| r.staff == note.staff && r.voice > 1);
                    let down = if polyphonic {
                        note.voice.is_multiple_of(2)
                    } else {
                        level <= 2.0
                    };
                    let stem_length = GAP
                        * note_scale
                        * (3.3 + note.duration.flag_count().saturating_sub(2) as f32 * 0.5);
                    let tip = at
                        + Vec2::new(
                            if down { -size.x * 0.45 } else { size.x * 0.45 },
                            if down {
                                stem_length * scale
                            } else {
                                -stem_length * scale
                            },
                        );
                    if note.duration != Duration::Whole {
                        crisp_line(
                            ui,
                            Pos2::new(tip.x, at.y),
                            tip,
                            GAP * 0.12 * scale * note_scale,
                            color,
                        );
                    }
                    if note.duration.flag_count() > 0 {
                        let character = char::from_u32(
                            0xe240 + (note.duration.flag_count() as u32 - 1) * 2 + u32::from(down),
                        )
                        .unwrap();
                        self.glyph(
                            ui,
                            character,
                            tip,
                            notation_height(character) * note_scale,
                            Vec2::ZERO,
                            color,
                        );
                    }
                    for dot in 0..note.dots.min(2) {
                        let dot_level = if (level - level.round()).abs() < 0.1 {
                            level - 0.5
                        } else {
                            level
                        };
                        ui.painter().circle_filled(
                            point(
                                x + size.x / (2.0 * scale) + 6.0 + f32::from(dot) * 6.0,
                                dot_level,
                            ),
                            1.7 * scale * note_scale,
                            color,
                        );
                    }
                    if let Some(ratio) = note.tuplet_ratio() {
                        ui.painter().text(
                            point(x, level - 4.0),
                            egui::Align2::CENTER_BOTTOM,
                            ratio.actual.to_string(),
                            egui::FontId::proportional(12.0 * scale),
                            color,
                        );
                    }
                    let mark = point(x, if down { level - 1.2 } else { level + 1.2 });
                    match note.articulation {
                        Articulation::None => {}
                        Articulation::Staccato => {
                            ui.painter().circle_filled(mark, 1.8 * scale, color);
                        }
                        Articulation::Tenuto => {
                            ui.painter().line_segment(
                                [
                                    mark - Vec2::new(4.0, 0.0) * scale,
                                    mark + Vec2::new(4.0, 0.0) * scale,
                                ],
                                Stroke::new(1.2 * scale, color),
                            );
                        }
                        Articulation::Accent => {
                            ui.painter().line_segment(
                                [
                                    mark - Vec2::new(4.0, 3.0) * scale,
                                    mark + Vec2::new(4.0, 0.0) * scale,
                                ],
                                Stroke::new(1.2 * scale, color),
                            );
                            ui.painter().line_segment(
                                [
                                    mark + Vec2::new(4.0, 0.0) * scale,
                                    mark + Vec2::new(-4.0, 3.0) * scale,
                                ],
                                Stroke::new(1.2 * scale, color),
                            );
                        }
                        Articulation::Marcato
                        | Articulation::Staccatissimo
                        | Articulation::Portato
                        | Articulation::AccentStaccato
                        | Articulation::TenutoAccent
                        | Articulation::MarcatoStaccato => {
                            let character = note.articulation.glyph(!down).unwrap();
                            self.glyph(
                                ui,
                                character,
                                mark,
                                notation_height(character) * note_scale,
                                Vec2::splat(0.5),
                                color,
                            );
                        }
                        Articulation::Fermata => {
                            self.glyph(
                                ui,
                                '\u{e4c0}',
                                point(x, level - 1.5),
                                GAP * 1.2,
                                Vec2::new(0.5, 1.0),
                                color,
                            );
                        }
                    }
                    if ui
                        .interact(
                            Rect::from_center_size(at, Vec2::splat(24.0 * scale)),
                            ui.id().with(("note", index)),
                            Sense::click(),
                        )
                        .on_hover_text(format!("{}  {}", note.pitch_label(), note.duration_label()))
                        .clicked()
                    {
                        clicked = Some(Selection::Note(index));
                    }
                }
                Item::Rest(index) => {
                    let rest = &score.rests[index];
                    let color = if selected == Some(Selection::Rest(index))
                        || self.selection.contains(&Selection::Rest(index))
                    {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::BLACK
                    };
                    if let Some(count) = rest.measures {
                        ui.painter().line_segment(
                            [point(x - 30.0, 2.0), point(x + 30.0, 2.0)],
                            Stroke::new(5.0 * scale, color),
                        );
                        for edge in [-30.0, 30.0] {
                            ui.painter().line_segment(
                                [point(x + edge, 1.0), point(x + edge, 3.0)],
                                Stroke::new(1.4 * scale, color),
                            );
                        }
                        ui.painter().text(
                            point(x, -0.6),
                            egui::Align2::CENTER_BOTTOM,
                            count.to_string(),
                            egui::FontId::proportional(20.0 * scale),
                            color,
                        );
                    } else {
                        let (character, height, level, anchor) = match rest.duration {
                            Duration::Whole => ('\u{e4e3}', 0.5, 1.0, 0.0),
                            Duration::Half => ('\u{e4e4}', 0.5, 2.0, 1.0),
                            Duration::Quarter => ('\u{e4e5}', 2.8, 2.0, 0.5),
                            Duration::Eighth => ('\u{e4e6}', 1.8, 2.0, 0.5),
                            Duration::Sixteenth => ('\u{e4e7}', 2.8, 2.0, 0.5),
                            Duration::ThirtySecond => ('\u{e4e8}', 3.8, 2.0, 0.5),
                            Duration::SixtyFourth => ('\u{e4e9}', 4.8, 2.0, 0.5),
                        };
                        let size = self.glyph(
                            ui,
                            character,
                            point(x, level),
                            GAP * height,
                            Vec2::new(0.5, anchor),
                            color,
                        );
                        for dot in 0..rest.dots.min(2) {
                            ui.painter().circle_filled(
                                point(x + size.x / (2.0 * scale) + 6.0 + f32::from(dot) * 6.0, 1.5),
                                1.7 * scale,
                                color,
                            );
                        }
                    }
                    if ui
                        .interact(
                            Rect::from_center_size(
                                point(x, 2.0),
                                Vec2::new(item.width(score), GAP * 3.0) * scale,
                            ),
                            ui.id().with(("rest", index)),
                            Sense::click(),
                        )
                        .on_hover_text(i18n::format(
                            "Rest {0}",
                            &[("0", rest.duration_label().to_string())],
                        ))
                        .clicked()
                    {
                        clicked = Some(Selection::Rest(index));
                    }
                }
                Item::TimeChange(index) => {
                    let color = if selected == Some(Selection::TimeChange(index))
                        || self.selection.contains(&Selection::TimeChange(index))
                    {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::BLACK
                    };
                    self.paint_time_signature(
                        ui,
                        score.time_changes[index].time_signature,
                        point(x - item.width(score) * 0.5 + 12.0, 2.0),
                        scale,
                        color,
                    );
                    if ui
                        .interact(
                            Rect::from_center_size(
                                point(x, 2.0),
                                Vec2::new(item.width(score), GAP * 5.0) * scale,
                            ),
                            ui.id().with(("time_change", index)),
                            Sense::click(),
                        )
                        .on_hover_text(i18n::format(
                            "Time change {0}",
                            &[(
                                "0",
                                score.time_changes[index].time_signature.label().to_string(),
                            )],
                        ))
                        .clicked()
                    {
                        clicked = Some(Selection::TimeChange(index));
                    }
                }
                Item::KeyChange(index) => {
                    let color = if selected == Some(Selection::KeyChange(index))
                        || self.selection.contains(&Selection::KeyChange(index))
                    {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::BLACK
                    };
                    for (ordinal, (character, pitch)) in
                        key_change_symbols(score, index).into_iter().enumerate()
                    {
                        self.glyph(
                            ui,
                            character,
                            point(
                                x - item.width(score) * 0.5 + 12.0 + ordinal as f32 * 9.0,
                                relative(
                                    pitch
                                        - match clef {
                                            Clef::Bass => 14,
                                            Clef::Treble8vb => 7,
                                            _ => 0,
                                        },
                                ),
                            ),
                            notation_height(character),
                            Vec2::new(0.0, if character == '\u{e260}' { 0.78 } else { 0.5 }),
                            color,
                        );
                    }
                    if ui
                        .interact(
                            Rect::from_center_size(
                                point(x, 2.0),
                                Vec2::new(item.width(score), GAP * 7.0) * scale,
                            ),
                            ui.id().with(("key_change", index)),
                            Sense::click(),
                        )
                        .on_hover_text(i18n::format(
                            "Key change {0}",
                            &[(
                                "0",
                                score.key_changes[index].key_signature.label().to_string(),
                            )],
                        ))
                        .clicked()
                    {
                        clicked = Some(Selection::KeyChange(index));
                    }
                }
                Item::Bar(index) => {
                    let kind = score.barlines[index].kind;
                    let strokes: &[(f32, f32)] = match kind {
                        BarlineKind::Single => &[(0.0, 1.0)],
                        BarlineKind::Double => &[(-3.0, 1.0), (3.0, 1.0)],
                        BarlineKind::Final | BarlineKind::RepeatEnd => &[(-4.0, 1.0), (3.0, 3.5)],
                        BarlineKind::RepeatStart => &[(-3.0, 3.5), (4.0, 1.0)],
                        BarlineKind::RepeatBoth => &[(-4.0, 1.0), (0.0, 3.5), (4.0, 1.0)],
                    };
                    for &(offset, thickness) in strokes {
                        line(point(x + offset, 0.0), point(x + offset, 4.0), thickness);
                    }
                    for side in [-1.0, 1.0] {
                        if kind == BarlineKind::RepeatBoth
                            || (side < 0.0 && kind == BarlineKind::RepeatEnd)
                            || (side > 0.0 && kind == BarlineKind::RepeatStart)
                        {
                            for level in [1.5, 2.5] {
                                ui.painter().circle_filled(
                                    point(x + side * 10.0, level),
                                    1.8 * scale,
                                    Color32::BLACK,
                                );
                            }
                        }
                    }
                }
            }
        }
        for lyric in score
            .lyrics
            .iter()
            .filter(|l| score.notes[l.note].staff == staff_index)
        {
            let Some(&(x, _)) = positioned
                .iter()
                .find(|(_, item)| matches!(item, Item::Note(index) if *index == lyric.note))
            else {
                continue;
            };
            let lyric_y = if lyric.placement == libreomr::model::LyricPlacement::Above {
                lyric_above
            } else {
                lyric_top
            } + f32::from(lyric.verse - 1) * 1.8;
            let label = ui.painter().text(
                point(x, lyric_y),
                egui::Align2::CENTER_CENTER,
                &lyric.text,
                egui::FontId::proportional(13.0 * scale),
                if selected == Some(Selection::Note(lyric.note))
                    || self.selection.contains(&Selection::Note(lyric.note))
                {
                    Color32::from_rgb(0, 95, 180)
                } else {
                    Color32::BLACK
                },
            );
            if let Some(number) = &lyric.verse_label {
                ui.painter().text(
                    Pos2::new(label.left() - 6.0 * scale, label.center().y),
                    egui::Align2::RIGHT_CENTER,
                    number,
                    egui::FontId::proportional(13.0 * scale),
                    Color32::BLACK,
                );
            }
            if ui
                .interact(
                    label.expand(2.0),
                    ui.id()
                        .with(("lyric", lyric.note, lyric.verse, lyric.placement)),
                    Sense::click(),
                )
                .clicked()
            {
                clicked = Some(Selection::Note(lyric.note));
            }
            let next = score
                .lyrics
                .iter()
                .filter(|other| {
                    other.verse == lyric.verse
                        && other.placement == lyric.placement
                        && score.notes[other.note].staff == staff_index
                        && score.notes[other.note].x > score.notes[lyric.note].x
                })
                .min_by(|a, b| score.notes[a.note].x.total_cmp(&score.notes[b.note].x));
            let end = next.map_or(x + 24.0, |next| map_x(score.notes[next.note].x));
            if matches!(
                lyric.syllabic,
                libreomr::model::Syllabic::Begin | libreomr::model::Syllabic::Middle
            ) {
                ui.painter().text(
                    point((x + end) * 0.5, lyric_y),
                    egui::Align2::CENTER_CENTER,
                    "-",
                    egui::FontId::proportional(13.0 * scale),
                    Color32::BLACK,
                );
            }
            if lyric.extend {
                let y = lyric_y + 0.6;
                line(point(x + 14.0, y), point(end - 8.0, y), 0.6);
            }
        }
        if !fit_object {
            for (index, span) in drawable_spans
                .iter()
                .filter(|(_, span)| span.staff == staff_index)
                .map(|(i, s)| (*i, s))
            {
                for (right, source) in [(false, span.start), (true, span.end)] {
                    let original = &score.spans[index];
                    let endpoint_staff = if right {
                        original
                            .end_anchor
                            .as_ref()
                            .map(|a| score.notes[a.note].staff)
                            .unwrap_or(original.staff)
                    } else {
                        original.staff
                    };
                    if endpoint_staff != staff_index {
                        continue;
                    }
                    let center = point(
                        map_x(source[0]),
                        (source[1] - staff.lines[0]) / staff.spacing(),
                    );
                    let active = selected == Some(Selection::Span(index))
                        || self.selection.contains(&Selection::Span(index));
                    let color = if active {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::GRAY
                    };
                    let radius = if active { 6.0 } else { 4.0 };
                    let anchor = if right {
                        &original.end_anchor
                    } else {
                        &original.start_anchor
                    };
                    if active && let Some(anchor) = anchor {
                        let note = &score.notes[anchor.note];
                        let attached =
                            point(map_x(note.x), (note.y - staff.lines[0]) / staff.spacing());
                        ui.painter()
                            .line_segment([center, attached], Stroke::new(1.0_f32, color));
                        ui.painter()
                            .circle_stroke(attached, 3.5, Stroke::new(1.0_f32, color));
                    }
                    if layout.axis.is_none()
                        || active
                        || ui.rect_contains_pointer(Rect::from_center_size(
                            center,
                            Vec2::splat(20.0),
                        ))
                    {
                        ui.painter().circle_filled(center, radius, Color32::WHITE);
                        ui.painter()
                            .circle_stroke(center, radius, Stroke::new(1.5_f32, color));
                    }
                    let response = ui
                        .interact(
                            Rect::from_center_size(center, Vec2::splat(20.0)),
                            ui.id().with(("span_anchor", index, right)),
                            Sense::click_and_drag(),
                        )
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(if right {
                            i18n::tr("Drag end anchor")
                        } else {
                            i18n::tr("Drag start anchor")
                        });
                    if response.clicked() {
                        clicked = Some(Selection::Span(index));
                    }
                    if response.drag_started() {
                        let mut x_positions = vec![];
                        let columns: Vec<_> = layout.axis.as_ref().map_or_else(
                            || items.iter().map(|item| item.0).collect(),
                            |axis| axis.points.iter().map(|point| point.0).collect(),
                        );
                        for x in std::iter::once(staff.left)
                            .chain(columns)
                            .chain(std::iter::once(staff.right))
                        {
                            let screen = point(map_x(x), 0.0).x;
                            if x_positions
                                .last()
                                .is_none_or(|&(previous, _)| screen > previous + 0.001)
                            {
                                x_positions.push((screen, x));
                            }
                        }
                        if x_positions.len() < 2 {
                            x_positions = vec![
                                (center.x - GAP * scale, source[0] - staff.spacing()),
                                (center.x + GAP * scale, source[0] + staff.spacing()),
                            ];
                        }
                        self.anchor_drag = Some(AnchorDrag {
                            index,
                            right,
                            source,
                            screen: center,
                            source_per_y: staff.spacing() / (GAP * scale),
                            delta: Vec2::ZERO,
                            x_positions,
                            upper,
                            lower,
                            leftmost,
                            natural_width,
                            scale,
                        });
                        clicked = Some(Selection::Span(index));
                    }
                    if (response.dragged() || response.drag_stopped())
                        && let Some(drag) = &mut self.anchor_drag
                        && drag.index == index
                        && drag.right == right
                    {
                        if response.dragged() {
                            drag.delta = response.drag_delta();
                        }
                        self.anchor_edit = Some(AnchorEdit {
                            index,
                            right,
                            position: drag.position(drag.delta),
                            started: response.drag_started(),
                            stopped: response.drag_stopped(),
                        });
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }
            }
        }
        if measures
            .last()
            .is_some_and(|measure| measure.barline.is_none() && measure.expected_ticks.is_some())
        {
            line(point(measure_end, 0.0), point(measure_end, 4.0), 1.0);
        }
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libreomr::model::{Barline, KeyChange, KeySignature, Staff, StaffHeader};

    fn choir_score() -> Score {
        use libreomr::model::{Lyric, StaffAssignment, Syllabic};
        let mut score = Score::default();
        for staff in 0..8 {
            let top = 30.0 + staff as f32 * 180.0;
            score.staves.push(Staff {
                lines: [top, top + 10.0, top + 20.0, top + 30.0, top + 40.0],
                left: 20.0,
                right: 1500.0,
                header: StaffHeader {
                    clef: if staff % 4 == 3 {
                        Clef::Bass
                    } else {
                        Clef::Treble
                    },
                    key_signature: Some(KeySignature {
                        fifths: if staff % 4 == 0 { 2 } else { 0 },
                    }),
                    ..Default::default()
                },
            });
            score.staff_assignments.push(StaffAssignment {
                staff,
                part: staff % 4,
                system: staff / 4,
                name: ["Soprano", "Alto", "Tenor", "Bass"][staff % 4].into(),
            });
            for x in [140.0, 360.0, 700.0, 1200.0] {
                if staff % 4 == 1 && x == 360.0 {
                    continue;
                }
                let index = score.add_note(x, top + 30.0).unwrap();
                for verse in 1..=4 {
                    score.lyrics.push(Lyric {
                        placement: Default::default(),
                        note: index,
                        verse,
                        verse_label: None,
                        text: format!("Lied{staff}-{verse}"),
                        syllabic: Syllabic::Single,
                        extend: false,
                        confirmed: false,
                        bounds: None,
                    });
                }
            }
            score.barlines.push(Barline {
                staff,
                x: 1000.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
        }
        score
    }

    fn document_frame(
        context: &egui::Context,
        preview: &mut Preview,
        score: &Score,
        staff: usize,
        selected: Option<Selection>,
        events: Vec<egui::Event>,
        size: Vec2,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview.show_document(ui, score, staff, selected, None);
                });
            },
        )
    }

    #[test]
    fn score_canvas_aligns_voices_and_fits_width_with_vertical_overflow() {
        let score = choir_score();
        let ctx = egui::Context::default();
        let mut preview = Preview::fitted();
        let mut output = None;
        for _ in 0..4 {
            output = Some(document_frame(
                &ctx,
                &mut preview,
                &score,
                0,
                None,
                vec![],
                Vec2::new(900.0, 360.0),
            ));
        }
        let layout = preview.document_layout.as_ref().unwrap();
        assert_eq!(layout.systems.len(), 2);
        let system = &layout.systems[0];
        assert_eq!(system.rows.len(), 4);
        let xs: Vec<_> = system
            .rows
            .iter()
            .map(|(r, _)| r.map_x(1000.0, &score.staves[r.staff]) - r.leftmost)
            .collect();
        assert!(xs.iter().all(|x| (x - xs[0]).abs() < 0.01));
        let (_, viewport, offset, content) = preview.canvas_view.unwrap();
        assert!(system.rect.height() * preview.document_scale > viewport.height());
        assert!(content.x <= viewport.width() + 1.0);
        assert!(content.y > viewport.height());
        assert!(offset.y < 10.0);
        let output = output.unwrap();
        assert!(
            output.shapes.iter().any(
                |s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Soprano")
                    && s.clip_rect.intersects(s.shape.visual_bounding_rect())
            ),
            "selected part is visible"
        );
    }

    #[test]
    fn score_canvas_follows_selection_but_preserves_manual_panning() {
        let score = choir_score();
        let ctx = egui::Context::default();
        let mut preview = Preview::default();
        let size = Vec2::new(600.0, 250.0);
        let note = score
            .notes
            .iter()
            .position(|n| n.staff == 7 && n.x == 1200.0)
            .unwrap();
        let selected = Some(Selection::Note(note));
        for _ in 0..4 {
            document_frame(&ctx, &mut preview, &score, 7, selected, vec![], size);
        }
        let (id, viewport, offset, _) = preview.canvas_view.unwrap();
        assert!(
            offset.x > 100.0 && offset.y > 500.0,
            "selected late Bass note is revealed: {offset:?}"
        );
        let layout = preview.document_layout.as_ref().unwrap();
        let (row, rect) = layout.systems[1]
            .rows
            .iter()
            .find(|(r, _)| r.staff == 7)
            .unwrap();
        let x = (row.map_x(1200.0, &score.staves[7]) - row.leftmost) * preview.document_scale
            - offset.x;
        assert!((0.0..=viewport.width()).contains(&x));
        let y = rect.center().y * preview.document_scale - offset.y;
        assert!((0.0..=viewport.height()).contains(&y));
        let mut state = egui::scroll_area::State::load(&ctx, id).unwrap();
        state.offset = Vec2::ZERO;
        state.store(&ctx, id);
        for _ in 0..3 {
            document_frame(&ctx, &mut preview, &score, 7, selected, vec![], size);
        }
        assert_eq!(
            preview.canvas_view.unwrap().2,
            Vec2::ZERO,
            "same selection must not pull back a manually panned view"
        );
        let first = Some(Selection::Note(0));
        for _ in 0..3 {
            document_frame(&ctx, &mut preview, &score, 0, first, vec![], size);
        }
        assert!(preview.canvas_view.unwrap().2.y < 100.0);
    }

    #[test]
    fn fitted_score_canvas_follows_selection_without_fitting_height() {
        let score = choir_score();
        let ctx = egui::Context::default();
        let mut preview = Preview::fitted();
        let mut wide_scale = None;
        for (staff, size) in [
            (0, Vec2::new(900.0, 360.0)),
            (3, Vec2::new(900.0, 180.0)),
            (7, Vec2::new(900.0, 360.0)),
            (7, Vec2::new(900.0, 85.0)),
            (7, Vec2::new(700.0, 180.0)),
        ] {
            let note = score.notes.iter().position(|n| n.staff == staff).unwrap();
            for _ in 0..4 {
                document_frame(
                    &ctx,
                    &mut preview,
                    &score,
                    staff,
                    Some(Selection::Note(note)),
                    vec![],
                    size,
                );
            }
            let system = &preview.document_layout.as_ref().unwrap().systems[staff / 4];
            let (row, bounds) = system.rows.iter().find(|(r, _)| r.staff == staff).unwrap();
            let (_, viewport, offset, content) = preview.canvas_view.unwrap();
            let note_level = (score.notes[note].y - score.staves[staff].lines[0])
                / score.staves[staff].spacing();
            let y = (bounds.top() + 8.0 + (note_level - row.upper) * GAP) * preview.document_scale
                - offset.y;
            assert!(
                (0.0..=viewport.height()).contains(&y),
                "selected note after selection/resize: {y}, {viewport:?}"
            );
            assert!(content.x <= viewport.width() + 1.0);
            assert!(offset.x < 1.0);
            if size.x == 900.0 {
                let previous = *wide_scale.get_or_insert(preview.document_scale);
                assert!(
                    (preview.document_scale - previous).abs() < 0.0001,
                    "height changes must not shrink the score"
                );
            }
        }
    }

    #[test]
    fn lyrics_render_and_select_their_note_in_octave_clef_preview() {
        use libreomr::model::{Lyric, Syllabic};
        let mut score = Score {
            staves: vec![Staff {
                lines: [20.0, 30.0, 40.0, 50.0, 60.0],
                left: 20.0,
                right: 300.0,
                header: StaffHeader {
                    clef: Clef::Treble8vb,
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        score.add_note(100.0, 60.0).unwrap();
        score.lyrics.push(Lyric {
            placement: Default::default(),
            note: 0,
            verse: 1,
            verse_label: Some("1.".into()),
            text: "für".into(),
            syllabic: Syllabic::Single,
            extend: false,
            confirmed: false,
            bounds: None,
        });
        let mut above = score.lyrics[0].clone();
        above.placement = libreomr::model::LyricPlacement::Above;
        above.text = "du".into();
        above.verse_label = None;
        score.lyrics.push(above);
        let context = egui::Context::default();
        let mut preview = Preview::default();
        let mut render = |events| {
            let mut selected = None;
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        selected = preview.show(ui, &score, 0, None);
                    });
                },
            );
            (output, selected)
        };
        for _ in 0..3 {
            render(Vec::new());
        }
        let (output, _) = render(Vec::new());
        assert!(
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "1."))
        );
        let point = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "für" => {
                    Some(s.shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("lyric is visible");
        let above = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "du" => {
                    let bounds = s.shape.visual_bounding_rect();
                    assert!(s.clip_rect.contains_rect(bounds));
                    Some(bounds)
                }
                _ => None,
            })
            .expect("above-staff lyric is visible without clipping");
        assert!(above.bottom() < point.y);
        assert!(
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="8"))
        );
        let (_, selected) = render(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]);
        assert_eq!(selected, Some(Selection::Note(0)));
    }

    fn object_score() -> Score {
        let staff = Staff {
            lines: [60.0, 72.0, 84.0, 96.0, 108.0],
            left: 20.0,
            right: 800.0,
            header: StaffHeader {
                clef: Clef::Bass,
                key_signature: Some(KeySignature { fifths: -2 }),
                time_signature: Some(TimeSignature::Meter {
                    beats: 12,
                    beat_unit: 8,
                }),
                ..Default::default()
            },
        };
        let mut score = Score {
            staves: vec![staff.clone(), staff],
            ..Default::default()
        };
        let index = score.add_note(160.0, 84.0).unwrap();
        let note = &mut score.notes[index];
        note.staff = 1;
        note.duration = Duration::Sixteenth;
        note.accidental = -1;
        note.accidental_explicit = true;
        note.dots = 2;
        note.triplet = true;
        note.articulation = Articulation::Fermata;
        score.add_note(650.0, 96.0);
        let rest = score.add_rest(220.0, 84.0, Duration::Quarter).unwrap();
        score.rests[rest].staff = 1;
        score.barlines.push(Barline {
            staff: 1,
            x: 500.0,
            kind: BarlineKind::Single,
            confirmed: false,
        });
        score.spans.push(Span {
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            staff: 1,
            kind: SpanKind::Slur,
            start: [120.0, 140.0],
            end: [300.0, 135.0],
            height: -18.0,
            confirmed: false,
        });
        for (x, fifths) in [(80.0, -4), (400.0, 1)] {
            score.key_changes.push(KeyChange {
                staff: 1,
                x,
                key_signature: KeySignature { fifths },
                bounds: vec![],
                confirmed: false,
            });
        }
        score
    }

    fn object_output(
        context: &egui::Context,
        preview: &mut Preview,
        score: &Score,
        selected: Selection,
        size: Vec2,
    ) -> egui::FullOutput {
        let mut output = None;
        for _pass in 0..3 {
            output = Some(context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        preview.show_object(ui, score, selected, 160.0);
                    });
                },
            ));
        }
        output.unwrap()
    }

    #[test]
    fn object_preview_allocates_only_the_fitted_notation_height() {
        let context = egui::Context::default();
        let score = object_score();
        let mut preview = Preview::fitted();
        for width in [184.0, 364.0] {
            let _ = context.run(egui::RawInput::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.set_max_width(width);
                    let before = ui.next_widget_position();
                    preview.show_object(ui, &score, Selection::Barline(0), 160.0);
                    let after = ui.next_widget_position();
                    assert!(
                        after.y - before.y < 144.0,
                        "the next widget must follow the notation without reserved blank space: {}",
                        after.y - before.y
                    );
                });
            });
        }
    }

    #[test]
    fn measure_veil_updates_color_fades_left_and_scales_with_preview() {
        let mut score = object_score();
        score.staves.truncate(1);
        score.staves[0].header.time_signature = Some(TimeSignature::Common);
        score.notes.clear();
        score.rests.clear();
        score.barlines.clear();
        score.key_changes.clear();
        score.spans.clear();
        score.add_note(160.0, 84.0);
        score.barlines.push(Barline {
            staff: 0,
            x: 400.0,
            kind: BarlineKind::Final,
            confirmed: false,
        });
        for width in [360.0, 1000.0] {
            let context = egui::Context::default();
            let mut preview = Preview::fitted();
            for (duration, meter, expected) in [
                (Duration::Whole, Some(TimeSignature::Common), Some(true)),
                (Duration::Quarter, Some(TimeSignature::Common), Some(false)),
                (
                    Duration::Quarter,
                    Some(TimeSignature::Meter {
                        beats: 1,
                        beat_unit: 4,
                    }),
                    Some(true),
                ),
                (Duration::Quarter, None, None),
            ] {
                score.notes[0].duration = duration;
                score.staves[0].header.time_signature = meter;
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 300.0))),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            preview.show(ui, &score, 0, None);
                        });
                    },
                );
                let veils: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh)
                            if mesh.vertices.len() == 4
                                && mesh.vertices[0].color == Color32::TRANSPARENT
                                && mesh.vertices[1].color.a() == 90 =>
                        {
                            Some((shape.clip_rect, mesh))
                        }
                        _ => None,
                    })
                    .collect();
                if let Some(valid) = expected {
                    assert_eq!(veils.len(), 1);
                    let (clip, mesh) = veils[0];
                    let color = if valid {
                        Color32::from_rgba_unmultiplied(0, 155, 85, 90)
                    } else {
                        Color32::from_rgba_unmultiplied(220, 45, 45, 90)
                    };
                    assert_eq!(mesh.vertices[1].color, color);
                    assert_eq!(mesh.vertices[2].color, color);
                    assert_eq!(mesh.vertices[3].color, Color32::TRANSPARENT);
                    assert!(mesh.vertices[0].pos.x < mesh.vertices[1].pos.x);
                    assert!((mesh.calc_bounds().width() - GAP * 1.8 * preview.scale).abs() < 0.01);
                    assert!(clip.contains_rect(mesh.calc_bounds()));
                } else {
                    assert!(veils.is_empty());
                }
            }
            let output = object_output(
                &context,
                &mut preview,
                &score,
                Selection::Note(0),
                Vec2::new(width, 300.0),
            );
            assert!(!output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|vertex| vertex.color.a() == 90))
            }));
        }
    }

    #[test]
    fn object_preview_isolates_every_type_and_retains_musical_context() {
        let mut score = object_score();
        let before = serde_json::to_value(&score).unwrap();
        for selected in [
            Selection::Note(0),
            Selection::Rest(0),
            Selection::Barline(0),
            Selection::Span(0),
            Selection::KeyChange(1),
        ] {
            let isolated = Preview::isolate_object(&score, selected).unwrap();
            assert_eq!(isolated.staves.len(), 1);
            assert_eq!(
                isolated.notes.len()
                    + isolated.rests.len()
                    + isolated.barlines.len()
                    + isolated.spans.len()
                    + isolated.key_changes.len(),
                1
            );
            assert_eq!(isolated.staves[0].header.clef, Clef::Bass);
            assert_eq!(
                isolated.staves[0].header.time_signature,
                score.staves[1].header.time_signature
            );
            assert_eq!(
                isolated.staves[0].header.key_signature.unwrap().fifths,
                if matches!(selected, Selection::Barline(_)) {
                    1
                } else {
                    -4
                }
            );
            let (original, actual) = match selected {
                Selection::Note(index) => (
                    serde_json::to_value(&score.notes[index]).unwrap(),
                    serde_json::to_value(&isolated.notes[0]).unwrap(),
                ),
                Selection::Rest(index) => (
                    serde_json::to_value(&score.rests[index]).unwrap(),
                    serde_json::to_value(&isolated.rests[0]).unwrap(),
                ),
                Selection::Barline(index) => (
                    serde_json::to_value(&score.barlines[index]).unwrap(),
                    serde_json::to_value(&isolated.barlines[0]).unwrap(),
                ),
                Selection::Span(index) => (
                    serde_json::to_value(&score.spans[index]).unwrap(),
                    serde_json::to_value(&isolated.spans[0]).unwrap(),
                ),
                Selection::KeyChange(index) => (
                    serde_json::to_value(&score.key_changes[index]).unwrap(),
                    serde_json::to_value(&isolated.key_changes[0]).unwrap(),
                ),
                Selection::Dynamic(index) => (
                    serde_json::to_value(&score.dynamics[index]).unwrap(),
                    serde_json::to_value(&isolated.dynamics[0]).unwrap(),
                ),
                Selection::RepeatMark(index) => (
                    serde_json::to_value(&score.repeat_marks[index]).unwrap(),
                    serde_json::to_value(&isolated.repeat_marks[0]).unwrap(),
                ),
                Selection::TimeChange(index) => (
                    serde_json::to_value(&score.time_changes[index]).unwrap(),
                    serde_json::to_value(&isolated.time_changes[0]).unwrap(),
                ),
            };
            let mut expected = original;
            expected["staff"] = 0.into();
            assert_eq!(actual, expected);
        }
        let isolated = Preview::isolate_object(&score, Selection::KeyChange(1)).unwrap();
        assert_eq!(
            key_change_symbols(&isolated, 0)
                .iter()
                .filter(|(character, _)| *character == '\u{e261}')
                .count(),
            4
        );
        assert_eq!(serde_json::to_value(&score).unwrap(), before);
        score.notes[0].accidental_explicit = false;
        score.notes[0].accidental = 0;
        assert!(
            Preview::isolate_object(&score, Selection::Note(0))
                .unwrap()
                .notes[0]
                .accidental_explicit
        );
        assert!(!score.notes[0].accidental_explicit);
        assert!(Preview::isolate_object(&score, Selection::Note(usize::MAX)).is_none());
    }

    #[test]
    fn object_preview_renders_live_settings_without_clipping_or_mutating() {
        let score = object_score();
        let before = serde_json::to_value(&score).unwrap();
        let mut variants = Vec::new();
        for duration in Duration::ALL {
            let mut changed = score.clone();
            changed.notes[0].duration = duration;
            if duration != score.notes[0].duration {
                variants.push((Selection::Note(0), changed));
            }
            let mut changed = score.clone();
            changed.rests[0].duration = duration;
            if duration != score.rests[0].duration {
                variants.push((Selection::Rest(0), changed));
            }
            for dots in [1, 2] {
                let mut changed = score.clone();
                changed.rests[0].duration = duration;
                changed.rests[0].dots = dots;
                variants.push((Selection::Rest(0), changed));
            }
        }
        for articulation in Articulation::ALL {
            let mut changed = score.clone();
            changed.notes[0].articulation = articulation;
            if articulation != score.notes[0].articulation {
                variants.push((Selection::Note(0), changed));
            }
        }
        for dots in 0..2 {
            let mut changed = score.clone();
            changed.notes[0].dots = dots;
            variants.push((Selection::Note(0), changed));
        }
        for accidental in [0, 1] {
            let mut changed = score.clone();
            changed.notes[0].accidental = accidental;
            variants.push((Selection::Note(0), changed));
        }
        let mut changed = score.clone();
        changed.notes[0].triplet = false;
        variants.push((Selection::Note(0), changed));
        let mut changed = score.clone();
        changed.notes[0].accidental_explicit = false;
        variants.push((Selection::Note(0), changed));
        let mut changed = score.clone();
        changed.notes[0].pitch += 7;
        variants.push((Selection::Note(0), changed));
        let mut changed = score.clone();
        changed.staves[1].header.clef = Clef::Treble;
        variants.push((Selection::Note(0), changed));
        for measures in [2, 99] {
            let mut changed = score.clone();
            changed.rests[0].measures = Some(measures);
            variants.push((Selection::Rest(0), changed));
        }
        for kind in BarlineKind::ALL.into_iter().skip(1) {
            let mut changed = score.clone();
            changed.barlines[0].kind = kind;
            variants.push((Selection::Barline(0), changed));
        }
        for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
            let mut changed = score.clone();
            changed.spans[0].kind = kind;
            changed.spans[0].height = 12.0;
            variants.push((Selection::Span(0), changed));
        }
        let mut changed = score.clone();
        changed.spans[0].height = 24.0;
        variants.push((Selection::Span(0), changed));
        let mut changed = score.clone();
        changed.spans[0].end = [400.0, 155.0];
        variants.push((Selection::Span(0), changed));
        for fifths in [-7, -4, 0, 7] {
            let mut changed = score.clone();
            changed.key_changes[1].key_signature.fifths = fifths;
            variants.push((Selection::KeyChange(1), changed));
        }
        let mut changed = score.clone();
        changed.notes[0].tuplet = Some(libreomr::model::Tuplet {
            actual: 7,
            normal: 4,
        });
        variants.push((Selection::Note(0), changed));
        let mut changed = score.clone();
        changed.notes[0].small = true;
        changed.notes[0].grace = true;
        variants.push((Selection::Note(0), changed));
        for width in [236.0, 356.0] {
            for density in [1.0, 2.0] {
                let context = egui::Context::default();
                context.set_pixels_per_point(density);
                let mut preview = Preview::default();
                let size = Vec2::new(width, 200.0);
                for (selected, changed) in &variants {
                    let original = object_output(&context, &mut preview, &score, *selected, size);
                    let output = object_output(&context, &mut preview, changed, *selected, size);
                    assert_ne!(
                        format!("{:?}", original.shapes),
                        format!("{:?}", output.shapes),
                        "setting must affect {selected:?}"
                    );
                    let mut glyphs = 0;
                    let mut circles = 0;
                    for shape in &output.shapes {
                        if let egui::Shape::Mesh(mesh) = &shape.shape {
                            glyphs += 1;
                            assert!(
                                shape.clip_rect.contains_rect(mesh.calc_bounds()),
                                "clipped {selected:?} at {width}/{density}: {:?}",
                                mesh.calc_bounds()
                            );
                        }
                        if let egui::Shape::Circle(circle) = &shape.shape {
                            circles += 1;
                            assert!(
                                shape.clip_rect.contains_rect(circle.visual_bounding_rect()),
                                "clipped dot at {width}/{density}"
                            );
                        }
                    }
                    assert!(glyphs > 0);
                    if let Selection::Rest(index) = selected {
                        assert_eq!(circles, usize::from(changed.rests[*index].dots));
                    }
                }
                assert!(
                    !preview.fit_width,
                    "object fit must not change the main preview setting"
                );
            }
        }
        assert_eq!(serde_json::to_value(&score).unwrap(), before);
    }

    #[test]
    fn dynamics_render_select_and_preserve_their_object_preview() {
        use libreomr::model::{Dynamic, DynamicKind};
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            for kind in DynamicKind::ALL {
                let mut score = object_score();
                score.dynamics.push(Dynamic {
                    staff: 1,
                    x: 400.0,
                    y: score.staves[1].lines[4] + 36.0,
                    kind,
                    bounds: None,
                    confirmed: false,
                });
                let before = serde_json::to_value(&score).unwrap();
                let isolated = Preview::isolate_object(&score, Selection::Dynamic(0)).unwrap();
                assert_eq!(isolated.dynamics.len(), 1);
                assert_eq!(isolated.dynamics[0].kind, kind);
                assert_eq!(isolated.dynamics[0].staff, 0);
                assert!(isolated.notes.is_empty());
                let context = egui::Context::default();
                let mut preview = Preview {
                    fit_width: true,
                    ..Preview::default()
                };
                let render = |preview: &mut Preview, events| {
                    let mut selected = None;
                    let output = context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |context| {
                            egui::CentralPanel::default().show(context, |ui| {
                                selected = preview.show(ui, &score, 1, None);
                            });
                        },
                    );
                    (output, selected)
                };
                for _ in 0..3 {
                    render(&mut preview, vec![]);
                }
                let (output, _) = render(&mut preview, vec![]);
                let glyph = preview
                    .glyphs
                    .iter()
                    .find(|((character, ..), _)| *character == kind.glyph())
                    .unwrap()
                    .1;
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.texture_id == glyph.texture.id() => {
                            assert!(shape.clip_rect.contains_rect(mesh.calc_bounds()));
                            Some(mesh.calc_bounds().center())
                        }
                        _ => None,
                    })
                    .unwrap();
                let mut selected = None;
                for pressed in [true, false] {
                    let (_, clicked) = render(
                        &mut preview,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    selected = clicked.or(selected);
                }
                assert_eq!(selected, Some(Selection::Dynamic(0)));
                assert_eq!(serde_json::to_value(&score).unwrap(), before);
            }
        }
    }

    #[test]
    fn repeat_marks_render_select_and_preserve_their_object_preview() {
        use libreomr::model::{RepeatMark, RepeatMarkKind};
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            for kind in RepeatMarkKind::ALL {
                let mut score = object_score();
                score.repeat_marks.push(RepeatMark {
                    staff: 1,
                    x: 400.0,
                    y: score.staves[1].lines[0] - 48.0,
                    kind,
                    end_x: 650.0,
                    numbers: "1,2".into(),
                    closed: true,
                    bounds: None,
                    confirmed: false,
                });
                let before = serde_json::to_value(&score).unwrap();
                let isolated = Preview::isolate_object(&score, Selection::RepeatMark(0)).unwrap();
                assert_eq!(isolated.repeat_marks.len(), 1);
                assert_eq!(isolated.repeat_marks[0].kind, kind);
                assert_eq!(isolated.repeat_marks[0].staff, 0);
                assert!(isolated.notes.is_empty());
                let context = egui::Context::default();
                let mut preview = Preview {
                    fit_width: true,
                    ..Preview::default()
                };
                let render = |preview: &mut Preview, events| {
                    let mut selected = None;
                    let output = context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |context| {
                            egui::CentralPanel::default().show(context, |ui| {
                                selected = preview.show(ui, &score, 1, None);
                            });
                        },
                    );
                    (output, selected)
                };
                for _ in 0..3 {
                    render(&mut preview, vec![]);
                }
                let (output, _) = render(&mut preview, vec![]);
                let position = if let Some(character) = kind.glyph() {
                    let glyph = preview
                        .glyphs
                        .iter()
                        .find(|((c, ..), _)| *c == character)
                        .unwrap()
                        .1;
                    output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Mesh(mesh) if mesh.texture_id == glyph.texture.id() => {
                                assert!(shape.clip_rect.contains_rect(mesh.calc_bounds()));
                                Some(mesh.calc_bounds().center())
                            }
                            _ => None,
                        })
                        .unwrap()
                } else {
                    output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::LineSegment { points, .. }
                                if points[0].y == points[1].y
                                    && points[1].x - points[0].x > 40.0 =>
                            {
                                Some((points[0] + points[1].to_vec2()) * 0.5)
                            }
                            _ => None,
                        })
                        .min_by(|a, b| a.y.total_cmp(&b.y))
                        .unwrap()
                };
                let mut selected = None;
                for pressed in [true, false] {
                    let (_, clicked) = render(
                        &mut preview,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    selected = clicked.or(selected);
                }
                assert_eq!(selected, Some(Selection::RepeatMark(0)));
                assert_eq!(serde_json::to_value(&score).unwrap(), before);
            }
        }
    }

    #[test]
    fn inline_time_changes_render_select_and_keep_the_correct_context() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let mut score = object_score();
            score.time_changes = vec![
                libreomr::model::TimeChange {
                    staff: 1,
                    x: 200.0,
                    time_signature: TimeSignature::Meter {
                        beats: 3,
                        beat_unit: 4,
                    },
                    bounds: vec![],
                    confirmed: true,
                },
                libreomr::model::TimeChange {
                    staff: 1,
                    x: 400.0,
                    time_signature: TimeSignature::Meter {
                        beats: 7,
                        beat_unit: 8,
                    },
                    bounds: vec![],
                    confirmed: false,
                },
            ];
            let isolated = Preview::isolate_object(&score, Selection::TimeChange(1)).unwrap();
            assert_eq!(isolated.time_changes.len(), 1);
            assert_eq!(
                isolated.staves[0].header.time_signature,
                Some(TimeSignature::Meter {
                    beats: 3,
                    beat_unit: 4
                })
            );
            assert_eq!(
                isolated.time_changes[0].time_signature,
                TimeSignature::Meter {
                    beats: 7,
                    beat_unit: 8
                }
            );
            let context = egui::Context::default();
            let mut preview = Preview::default();
            let render = |preview: &mut Preview, events| {
                let mut selected = None;
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            selected = preview.show(ui, &score, 1, None);
                        });
                    },
                );
                (output, selected)
            };
            for _ in 0..3 {
                render(&mut preview, vec![]);
            }
            let (output, _) = render(&mut preview, vec![]);
            let glyph = preview
                .glyphs
                .iter()
                .find(|((character, ..), _)| *character == '\u{e087}')
                .unwrap()
                .1;
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == glyph.texture.id() => {
                        Some(mesh.calc_bounds().center())
                    }
                    _ => None,
                })
                .unwrap();
            let mut selected = None;
            for pressed in [true, false] {
                let (_, clicked) = render(
                    &mut preview,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
                selected = clicked.or(selected);
            }
            assert_eq!(selected, Some(Selection::TimeChange(1)));
        }
    }

    #[test]
    fn inline_key_changes_render_cancellations_and_new_signatures() {
        for clef in [Clef::Treble, Clef::Bass] {
            for (previous, next, naturals, signs) in
                [(-4, -1, 3, 1), (2, -2, 2, 2), (-2, 0, 2, 0), (0, 3, 0, 3)]
            {
                let score = Score {
                    staves: vec![libreomr::model::Staff {
                        lines: [60.0, 72.0, 84.0, 96.0, 108.0],
                        left: 20.0,
                        right: 400.0,
                        header: libreomr::model::StaffHeader {
                            clef,
                            key_signature: Some(libreomr::model::KeySignature { fifths: previous }),
                            ..Default::default()
                        },
                    }],
                    key_changes: vec![libreomr::model::KeyChange {
                        staff: 0,
                        x: 120.0,
                        key_signature: libreomr::model::KeySignature { fifths: next },
                        bounds: vec![],
                        confirmed: false,
                    }],
                    ..Default::default()
                };
                let symbols = key_change_symbols(&score, 0);
                assert_eq!(
                    symbols
                        .iter()
                        .filter(|(character, _)| *character == '\u{e261}')
                        .count(),
                    naturals
                );
                assert_eq!(symbols.len(), naturals + signs);
                for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
                    let context = egui::Context::default();
                    let mut preview = Preview::default();
                    let output = context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |context| {
                            egui::CentralPanel::default().show(context, |ui| {
                                preview.show(ui, &score, 0, Some(Selection::KeyChange(0)));
                            });
                        },
                    );
                    assert!(!output.shapes.is_empty());
                    for (character, _) in &symbols {
                        assert!(preview.glyphs.keys().any(|(cached, _)| cached == character));
                    }
                    let character = if naturals > 0 { '\u{e261}' } else { '\u{e262}' };
                    let position = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Mesh(mesh)
                                if preview.glyphs.iter().any(|((cached, _), glyph)| {
                                    *cached == character && glyph.texture.id() == mesh.texture_id
                                }) && shape.clip_rect.contains_rect(mesh.calc_bounds()) =>
                            {
                                Some(mesh.calc_bounds().center())
                            }
                            _ => None,
                        })
                        .expect("visible key-change glyph");
                    let mut clicked = None;
                    for pressed in [true, false] {
                        let _ = context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                events: vec![
                                    egui::Event::PointerMoved(position),
                                    egui::Event::PointerButton {
                                        pos: position,
                                        button: egui::PointerButton::Primary,
                                        pressed,
                                        modifiers: Default::default(),
                                    },
                                ],
                                ..Default::default()
                            },
                            |context| {
                                egui::CentralPanel::default().show(context, |ui| {
                                    clicked = preview.show(ui, &score, 0, None);
                                });
                            },
                        );
                    }
                    assert_eq!(clicked, Some(Selection::KeyChange(0)));
                }
            }
        }
    }

    #[test]
    fn middle_line_and_higher_notes_have_downward_stems() {
        for clef in [Clef::Bass, Clef::Treble] {
            for step in [-1, 0, 1] {
                let mut score = Score {
                    staves: vec![libreomr::model::Staff {
                        lines: [60.0, 72.0, 84.0, 96.0, 108.0],
                        left: 20.0,
                        right: 400.0,
                        header: libreomr::model::StaffHeader {
                            clef,
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                };
                score.add_note(100.0, 84.0 - step as f32 * 6.0);
                if clef == Clef::Bass && step == 0 {
                    assert_eq!(score.notes[0].pitch_label(), "D3");
                }
                score.notes[0].duration = Duration::Eighth;
                let context = egui::Context::default();
                let mut preview = Preview::default();
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            preview.show(ui, &score, 0, None);
                        });
                    },
                );
                let stem = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::LineSegment { points, .. }
                            if points[0].x == points[1].x
                                && (points[1].y - points[0].y).abs() > 30.0 =>
                        {
                            Some(points)
                        }
                        _ => None,
                    })
                    .expect("D3 stem rendered");
                assert_eq!(
                    stem[1].y > stem[0].y,
                    step >= 0,
                    "{:?}, {}: {stem:?}",
                    clef,
                    score.notes[0].pitch_label()
                );
                let flag = if step >= 0 { '\u{e241}' } else { '\u{e240}' };
                assert!(
                    preview
                        .glyphs
                        .keys()
                        .any(|(character, ..)| *character == flag)
                );
            }
        }
    }

    #[test]
    fn fitted_accidentals_stay_transparent_at_problematic_sizes() {
        // The first staff of 80er_KULTtour_2_-_Bass2Cf.pdf uses em=27.269905.
        // The previous rasterizer produced a noisy rectangle at that size.
        for density in [1.0, 1.25, 1.5, 2.0] {
            for em in [
                8.0, 10.37, 12.2, 16.001, 20.3333, 27.269904, 27.269905, 27.269906, 27.37,
                31.41593, 40.904858, 54.53981, 72.1234,
            ] {
                for character in ['\u{e260}', '\u{e261}', '\u{e262}'] {
                    let font = music_font();
                    let scale =
                        em * density * font.height_unscaled() / font.units_per_em().unwrap();
                    let (image, _) = rasterize_music_glyph(character, scale);
                    // Tiny sharps/naturals may fill their narrow pixel bounds.
                    if character == '\u{e260}' || em * density >= 16.0 {
                        assert!(
                            image.pixels.iter().any(|pixel| pixel.a() == 0),
                            "opaque background for {character} at {em}/{density}"
                        );
                    }
                    assert!(image.pixels.iter().any(|pixel| pixel.a() > 0));
                    if em * density >= 20.0 {
                        assert!(
                            image.pixels.iter().any(|pixel| pixel.a() >= 200),
                            "missing ink for {character} at {em}/{density}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn accidentals_use_transparent_bravura_masks_in_both_themes() {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            for density in [1.0, 1.5, 2.0] {
                let context = egui::Context::default();
                context.set_theme(theme);
                context.set_pixels_per_point(density);
                let mut preview = Preview::default();
                let output = context.run(egui::RawInput::default(), |context| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(Color32::GRAY))
                        .show(context, |ui| {
                            for (index, character) in
                                ['\u{e260}', '\u{e261}', '\u{e262}', '\u{e261}']
                                    .into_iter()
                                    .enumerate()
                            {
                                preview.glyph(
                                    ui,
                                    character,
                                    Pos2::new(60.0 + index as f32 * 80.0, 70.0),
                                    notation_height(character),
                                    Vec2::splat(0.5),
                                    if index == 1 {
                                        Color32::from_rgb(0, 95, 180)
                                    } else {
                                        Color32::BLACK
                                    },
                                );
                            }
                        });
                });
                assert_eq!(
                    preview.glyphs.len(),
                    3,
                    "selection color shares the same alpha mask"
                );
                let meshes: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) => Some(mesh),
                        _ => None,
                    })
                    .collect();
                assert_eq!(meshes.len(), 4);
                for (index, mesh) in meshes.iter().enumerate() {
                    let expected = if index == 1 {
                        Color32::from_rgb(0, 95, 180)
                    } else {
                        Color32::BLACK
                    };
                    assert!(mesh.vertices.iter().all(|vertex| vertex.color == expected));
                }
                for glyph in preview.glyphs.values() {
                    let (_, delta) = output
                        .textures_delta
                        .set
                        .iter()
                        .find(|(id, _)| *id == glyph.texture.id())
                        .unwrap();
                    let egui::ImageData::Color(image) = &delta.image;
                    assert!(image.pixels.iter().any(|pixel| pixel.a() == 0));
                    assert!(image.pixels.iter().any(|pixel| pixel.a() > 128));
                }
                assert!(!output.shapes.iter().any(|shape|
                    matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == Color32::WHITE)),
                    "no backing rectangle behind the accidentals");
            }
        }
    }

    #[test]
    fn flags_and_accidentals_share_the_notehead_scale() {
        let characters = [
            '\u{e0a4}', '\u{e240}', '\u{e241}', '\u{e242}', '\u{e243}', '\u{e244}', '\u{e245}',
            '\u{e246}', '\u{e247}', '\u{e260}', '\u{e261}', '\u{e262}',
        ];
        for density in [1.0, 1.5, 2.0] {
            let context = egui::Context::default();
            context.set_pixels_per_point(density);
            let mut preview = Preview::default();
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1040.0, 520.0))),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        for (index, character) in characters.iter().enumerate() {
                            preview.glyph(
                                ui,
                                *character,
                                Pos2::new(60.0 + index as f32 * 70.0, 100.0),
                                notation_height(*character),
                                Vec2::ZERO,
                                Color32::BLACK,
                            );
                        }
                    });
                },
            );
            for character in characters {
                let native = music_font()
                    .outline_glyph(
                        music_font()
                            .glyph_id(character)
                            .with_scale(notation_scale() * density),
                    )
                    .unwrap()
                    .px_bounds();
                let key = (
                    character,
                    (notation_height(character) * density).round() as u32,
                );
                let bounds = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh)
                            if preview
                                .glyphs
                                .get(&key)
                                .is_some_and(|glyph| mesh.texture_id == glyph.texture.id()) =>
                        {
                            Some(mesh.calc_bounds())
                        }
                        _ => None,
                    })
                    .unwrap();
                let expected = Vec2::new(native.width(), native.height());
                assert!(
                    (bounds.height() * density - expected.y).abs() < 0.001,
                    "{character}, density {density}"
                );
                assert!(
                    (bounds.width() * density - expected.x).abs() < 0.001,
                    "{character}, density {density}"
                );
            }
        }
        for character in ['\u{e240}', '\u{e241}', '\u{e242}', '\u{e243}'] {
            assert!(notation_height(character) < GAP * 3.2);
        }
        for character in ['\u{e260}', '\u{e261}', '\u{e262}'] {
            assert!(notation_height(character) < GAP * 2.5);
        }
    }

    #[test]
    fn flags_touch_the_stem_and_use_native_pixel_resolution() {
        for density in [1.0, 1.5, 2.0] {
            for character in '\u{e240}'..='\u{e247}' {
                let context = egui::Context::default();
                context.set_pixels_per_point(density);
                let mut preview = Preview::default();
                let tip = Pos2::new(180.25, 100.25);
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 260.0))),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            preview.glyph(
                                ui,
                                character,
                                tip,
                                notation_height(character),
                                Vec2::ZERO,
                                Color32::BLACK,
                            );
                            crisp_line(
                                ui,
                                tip,
                                tip + Vec2::new(0.0, -30.0),
                                GAP * 0.12,
                                Color32::BLACK,
                            );
                        });
                    },
                );
                let glyph = preview.glyphs.values().next().unwrap();
                let mesh = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.texture_id == glyph.texture.id() => {
                            Some(mesh)
                        }
                        _ => None,
                    })
                    .unwrap();
                let bounds = mesh.calc_bounds();
                let image = output
                    .textures_delta
                    .set
                    .iter()
                    .find_map(|(id, delta)| {
                        if *id != glyph.texture.id() {
                            return None;
                        }
                        let egui::ImageData::Color(image) = &delta.image;
                        Some(image)
                    })
                    .unwrap();
                assert!((bounds.width() * density - image.width() as f32).abs() < 0.001);
                assert!((bounds.height() * density - image.height() as f32).abs() < 0.001);
                assert!(
                    (bounds.left() * density - (bounds.left() * density).round()).abs() < 0.001
                );
                assert!((bounds.top() * density - (bounds.top() * density).round()).abs() < 0.001);
                let connected = image.pixels.iter().enumerate().any(|(index, pixel)| {
                    let column = (index % image.width()) as f32 + 0.5;
                    let row = (index / image.width()) as f32 + 0.5;
                    let point = bounds.min + Vec2::new(column, row) / density;
                    pixel.a() >= 100
                        && (point.x - tip.x).abs() <= 1.5 / density
                        && (point.y - tip.y).abs() <= 2.5 / density
                });
                assert!(
                    connected,
                    "flag {character} at density {density}: {bounds:?}, origin {:?}",
                    glyph.origin
                );
                for shape in &output.shapes {
                    if let egui::Shape::LineSegment { points, stroke } = &shape.shape {
                        let pixels = stroke.width * density;
                        assert!((pixels - pixels.round()).abs() < 0.001);
                        assert!(
                            (points[0].x * density
                                - pixels * 0.5
                                - (points[0].x * density - pixels * 0.5).round())
                            .abs()
                                < 0.001
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn preview_draws_only_current_staff_spans_and_updates_curvature() {
        let mut score = libreomr::recognition::recognize(&libreomr::recognition::demo_image());
        score.spans.clear();
        let staff = &score.staves[0];
        for (index, kind) in SpanKind::ALL.into_iter().enumerate() {
            score.spans.push(Span {
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                staff: 0,
                kind,
                start: [
                    score.notes[0].x,
                    staff.lines[4] + staff.spacing() * (2.0 + index as f32),
                ],
                end: [
                    score.notes[2].x,
                    staff.lines[4] + staff.spacing() * (2.0 + index as f32),
                ],
                height: staff.spacing() * 0.6,
                confirmed: false,
            });
        }
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut preview = Preview::default();
            let render = |preview: &mut Preview, score: &Score, staff| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            preview.show(ui, score, staff, None);
                        });
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut preview, &score, 0);
            }
            let output = render(&mut preview, &score, 0);
            let curves = |output: &egui::FullOutput| {
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Path(path) if path.points.len() == 49 => {
                            Some(path.points.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let before = curves(&output);
            assert_eq!(before.len(), 2);
            assert_eq!(output.shapes.iter().filter(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if stroke.width == 1.3)).count(), 4);
            score.spans[2].height *= -1.0;
            assert_ne!(curves(&render(&mut preview, &score, 0)), before);
            assert!(curves(&render(&mut preview, &score, 1)).is_empty());
        }
    }

    #[test]
    fn preview_uses_compact_top_spacing_for_notes_curves_and_repeat_marks() {
        let mut score = object_score();
        score.staves.truncate(1);
        score.staves[0].header = StaffHeader::default();
        score.notes.clear();
        score.rests.clear();
        score.key_changes.clear();
        score.barlines.clear();
        score.spans.clear();
        score.add_note(160.0, 84.0);
        let mut variants = vec![score.clone()];
        for level in [-6.0, 0.0, 2.0, 4.0, 10.0] {
            for duration in Duration::ALL {
                let mut changed = score.clone();
                changed.notes[0].pitch = changed.staves[0].pitch_at(60.0 + level * GAP);
                changed.notes[0].duration = duration;
                changed.notes[0].dots = 2;
                changed.notes[0].accidental_explicit = true;
                changed.notes[0].accidental = -1;
                variants.push(changed);
            }
        }
        for articulation in Articulation::ALL {
            let mut changed = score.clone();
            changed.notes[0].articulation = articulation;
            changed.notes[0].triplet = true;
            variants.push(changed);
        }
        for kind in SpanKind::ALL {
            let mut changed = score.clone();
            changed.spans.push(Span {
                start_anchor: None,
                end_anchor: None,
                continuation: None,
                wedge_openings: None,
                staff: 0,
                kind,
                start: [160.0, 48.0],
                end: [300.0, 48.0],
                // Wedges require a positive half-opening; bows may curve upward.
                height: if matches!(kind, SpanKind::Slur | SpanKind::Tie) {
                    -48.0
                } else {
                    48.0
                },
                confirmed: false,
            });
            assert!(changed.validate().is_ok(), "invalid span fixture: {kind:?}");
            variants.push(changed);
        }
        for kind in RepeatMarkKind::ALL {
            let mut changed = score.clone();
            changed.repeat_marks.push(RepeatMark {
                staff: 0,
                kind,
                x: 160.0,
                y: 0.0,
                end_x: 300.0,
                numbers: "1,2".into(),
                closed: true,
                bounds: None,
                confirmed: false,
            });
            variants.push(changed);
        }
        for density in [1.0, 1.5, 2.0] {
            for score in &variants {
                let context = egui::Context::default();
                context.set_pixels_per_point(density);
                let mut preview = Preview::default();
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(1000.0, 300.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            preview.show(ui, score, 0, None);
                        });
                    },
                );
                for shape in &output.shapes {
                    let bounds = shape.shape.visual_bounding_rect();
                    if !matches!(shape.shape, egui::Shape::Rect(_) | egui::Shape::Noop) {
                        assert!(
                            shape.clip_rect.contains_rect(bounds),
                            "notation clipped at density {density}: {bounds:?} / {:?}",
                            shape.clip_rect
                        );
                    }
                }
                let top = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        (!matches!(shape.shape, egui::Shape::Rect(_))).then_some(
                            (shape.shape.visual_bounding_rect().top() - shape.clip_rect.top())
                                .max(0.0),
                        )
                    })
                    .reduce(f32::min)
                    .unwrap();
                assert!(
                    top <= 12.0,
                    "unused top margin: {top} at density {density}, notes: {:?}",
                    score.notes
                );
            }
        }
    }

    #[test]
    fn staff_lines_end_after_the_last_object_and_shrink_after_deletion() {
        use libreomr::model::{Dynamic, DynamicKind, TimeChange};
        let mut score = object_score();
        score.staves.truncate(1);
        score.staves[0].header = StaffHeader::default();
        score.notes.clear();
        score.rests.clear();
        score.key_changes.clear();
        score.barlines.clear();
        score.spans.clear();
        score.add_note(160.0, 84.0);
        let mut variants = Vec::new();
        let mut changed = score.clone();
        changed.add_note(650.0, 96.0);
        changed.notes[1].dots = 2;
        variants.push(changed);
        let mut changed = score.clone();
        let rest = changed.add_rest(650.0, 84.0, Duration::Whole).unwrap();
        changed.rests[rest].measures = Some(99);
        variants.push(changed);
        let mut changed = score.clone();
        changed.barlines.push(Barline {
            staff: 0,
            x: 650.0,
            kind: BarlineKind::RepeatBoth,
            confirmed: false,
        });
        variants.push(changed);
        let mut changed = score.clone();
        changed.key_changes.push(KeyChange {
            staff: 0,
            x: 650.0,
            key_signature: KeySignature { fifths: 7 },
            bounds: vec![],
            confirmed: false,
        });
        variants.push(changed);
        let mut changed = score.clone();
        changed.time_changes.push(TimeChange {
            staff: 0,
            x: 650.0,
            time_signature: TimeSignature::Meter {
                beats: 12,
                beat_unit: 16,
            },
            bounds: vec![],
            confirmed: false,
        });
        variants.push(changed);
        let mut changed = score.clone();
        changed.dynamics.push(Dynamic {
            staff: 0,
            x: 650.0,
            y: 120.0,
            kind: DynamicKind::Ffff,
            bounds: None,
            confirmed: false,
        });
        variants.push(changed);
        let mut changed = score.clone();
        changed.spans.push(Span {
            start_anchor: None,
            end_anchor: None,
            continuation: None,
            wedge_openings: None,
            staff: 0,
            kind: SpanKind::Slur,
            start: [400.0, 48.0],
            end: [800.0, 48.0],
            height: -12.0,
            confirmed: false,
        });
        variants.push(changed);
        for kind in RepeatMarkKind::ALL {
            let mut changed = score.clone();
            changed.repeat_marks.push(RepeatMark {
                staff: 0,
                x: 650.0,
                y: 24.0,
                kind,
                end_x: 800.0,
                numbers: "1,2".into(),
                closed: true,
                bounds: None,
                confirmed: false,
            });
            variants.push(changed);
        }
        for fit_width in [false, true] {
            for density in [1.0, 2.0] {
                let context = egui::Context::default();
                context.set_pixels_per_point(density);
                let mut preview = Preview {
                    fit_width,
                    ..Default::default()
                };
                let mut render = |score: &Score, width| {
                    let before = serde_json::to_value(score).unwrap();
                    let mut output = None;
                    for _ in 0..3 {
                        output = Some(context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(
                                    Pos2::ZERO,
                                    Vec2::new(width, 300.0),
                                )),
                                ..Default::default()
                            },
                            |context| {
                                egui::CentralPanel::default().show(context, |ui| {
                                    preview.show(ui, score, 0, None);
                                });
                            },
                        ));
                    }
                    assert_eq!(serde_json::to_value(score).unwrap(), before);
                    let mut ends = Vec::new();
                    let mut content_right = 0.0_f32;
                    for shape in &output.unwrap().shapes {
                        if let egui::Shape::LineSegment { points, stroke } = &shape.shape
                            && points[0].y == points[1].y
                            && (points[1].x - points[0].x).abs() > 80.0
                            && (stroke.width - (0.8 * density).round().max(1.0) / density).abs()
                                < 0.01
                        {
                            ends.push(points[1].x);
                        } else if !matches!(shape.shape, egui::Shape::Rect(_)) {
                            content_right =
                                content_right.max(shape.shape.visual_bounding_rect().right());
                        }
                    }
                    assert_eq!(ends.len(), 5);
                    assert!(ends.iter().all(|end| (*end - ends[0]).abs() < 0.01));
                    let padding = ends[0] - content_right;
                    assert!((-1.0..=32.0).contains(&padding), "end padding: {padding}");
                    ends[0]
                };
                let short_end = render(&score, 1000.0);
                for changed in &variants {
                    let end = render(changed, 1000.0);
                    assert!(end > short_end + GAP);
                    assert!(
                        (render(changed, 1400.0) - end).abs() < 1.0,
                        "window width must not extend the staff"
                    );
                    assert!(
                        (render(&score, 1000.0) - short_end).abs() < 1.0,
                        "deletion must shorten the staff again"
                    );
                }
            }
        }
    }

    #[test]
    fn fitted_preview_keeps_the_complete_row_visible_and_last_note_clickable() {
        let mut score = libreomr::recognition::recognize(&libreomr::recognition::demo_image());
        for (ordinal, note) in score
            .notes
            .iter_mut()
            .filter(|note| note.staff == 0)
            .enumerate()
        {
            note.duration = Duration::Quarter;
            note.x = 200.0 + ordinal as f32 * 300.0;
        }
        score.staves[0].right = 4000.0;
        let expected = score
            .notes
            .iter()
            .rposition(|note| note.staff == 0)
            .unwrap();
        for size in [Vec2::new(400.0, 180.0), Vec2::new(900.0, 240.0)] {
            for density in [1.0, 2.0] {
                let context = egui::Context::default();
                context.set_pixels_per_point(density);
                let mut preview = Preview::fitted();
                let render = |preview: &mut Preview, events| {
                    let mut selected = None;
                    let output = context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |context| {
                            egui::CentralPanel::default().show(context, |ui| {
                                selected = preview.show(ui, &score, 0, None);
                            });
                        },
                    );
                    (output, selected)
                };
                for _pass in 0..3 {
                    render(&mut preview, vec![]);
                }
                let (output, _) = render(&mut preview, vec![]);
                assert!(preview.scale < 1.0);
                let heads: Vec<_> = preview
                    .glyphs
                    .iter()
                    .filter(|((character, ..), _)| *character == '\u{e0a4}')
                    .map(|(_, glyph)| glyph.texture.id())
                    .collect();
                let mut last = None;
                for shape in &output.shapes {
                    if let egui::Shape::Mesh(mesh) = &shape.shape {
                        let bounds = mesh.calc_bounds();
                        assert!(
                            shape.clip_rect.contains_rect(bounds),
                            "{size:?}, {density}: {bounds:?} clipped by {:?}",
                            shape.clip_rect
                        );
                        if heads.contains(&mesh.texture_id)
                            && last.is_none_or(|point: Pos2| bounds.center().x > point.x)
                        {
                            last = Some(bounds.center());
                        }
                    }
                }
                let position = last.expect("last notehead visible without horizontal scrolling");
                let mut selected = None;
                for pressed in [true, false] {
                    let (_, clicked) = render(
                        &mut preview,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                    selected = clicked.or(selected);
                }
                assert_eq!(selected, Some(Selection::Note(expected)));
                preview.fit_width = false;
                render(&mut preview, vec![]);
                assert_eq!(preview.scale, 1.0);
                preview.fit_width = true;
                let (output, _) = render(&mut preview, vec![]);
                assert!(preview.scale < 1.0);
                assert!(
                    output
                        .shapes
                        .iter()
                        .any(|shape| matches!(&shape.shape, egui::Shape::Mesh(_)))
                );
            }
        }
    }

    #[test]
    fn renders_preview_symbols_and_selects_notes_and_rests() {
        let mut score = libreomr::recognition::recognize(&libreomr::recognition::demo_image());
        for (index, note) in score.notes.iter_mut().enumerate() {
            note.duration = Duration::ALL[index % Duration::ALL.len()];
            note.articulation = Articulation::ALL[index % Articulation::ALL.len()];
            note.accidental_explicit = true;
            note.accidental = [-1, 0, 1][index % 3];
            note.dots = (index % 3) as u8;
            note.triplet = index % 2 == 0;
        }
        for (index, duration) in Duration::ALL.into_iter().enumerate() {
            score.add_rest(
                1000.0 + index as f32 * 100.0,
                score.staves[0].lines[2],
                duration,
            );
        }
        for (index, kind) in BarlineKind::ALL.into_iter().enumerate() {
            score.barlines.push(libreomr::model::Barline {
                staff: 0,
                x: 1500.0 + index as f32 * 100.0,
                kind,
                confirmed: false,
            });
        }
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut preview = Preview::default();
            let render = |preview: &mut Preview, staff, events| {
                let mut selected = None;
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            selected = preview.show(ui, &score, staff, None);
                        });
                    },
                );
                (output, selected)
            };
            let (first, _) = render(&mut preview, 0, Vec::new());
            assert!(
                first
                    .textures_delta
                    .set
                    .iter()
                    .any(|(_, delta)| match &delta.image {
                        egui::ImageData::Color(image) =>
                            image.pixels.iter().any(|pixel| pixel.a() > 0),
                    })
            );
            for _pass in 0..3 {
                render(&mut preview, 0, Vec::new());
            }
            let (output, _) = render(&mut preview, 0, Vec::new());
            let texture = preview
                .glyphs
                .iter()
                .find(|((character, ..), _)| *character == '\u{e0a2}')
                .unwrap()
                .1
                .texture
                .id();
            let at = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        let bounds = mesh.calc_bounds();
                        shape
                            .clip_rect
                            .contains_rect(bounds)
                            .then_some(bounds.center())
                    }
                    _ => None,
                })
                .expect("first whole note is visible");
            let mut clicked = None;
            for pressed in [true, false] {
                let (_, selection) = render(
                    &mut preview,
                    0,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
                clicked = selection.or(clicked);
            }
            assert_eq!(clicked, Some(Selection::Note(0)));
            render(&mut preview, 1, Vec::new());
            for character in [
                '\u{e062}', '\u{e050}', '\u{e0a2}', '\u{e0a3}', '\u{e0a4}', '\u{e260}', '\u{e261}',
                '\u{e262}', '\u{e4c0}',
            ] {
                assert!(
                    preview
                        .glyphs
                        .keys()
                        .any(|(cached, ..)| *cached == character),
                    "{character}"
                );
            }
            let mut rest_score = score.clone();
            rest_score.notes.clear();
            rest_score.barlines.clear();
            rest_score.rests.truncate(1);
            rest_score.rests[0].measures = Some(8);
            let rest_context = egui::Context::default();
            let render_rest = |preview: &mut Preview, events| {
                let mut selection = None;
                let output = rest_context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            selection = preview.show(ui, &rest_score, 0, None);
                        });
                    },
                );
                (output, selection)
            };
            let mut rest_preview = Preview::default();
            for _pass in 0..3 {
                render_rest(&mut rest_preview, Vec::new());
            }
            let (output, _) = render_rest(&mut rest_preview, Vec::new());
            let at = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::LineSegment { points, stroke } if stroke.width == 5.0 => {
                        Some(points[0].lerp(points[1], 0.5))
                    }
                    _ => None,
                })
                .expect("multimeasure rest is rendered");
            let mut clicked = None;
            for pressed in [true, false] {
                let (_, selection) = render_rest(
                    &mut rest_preview,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
                clicked = selection.or(clicked);
            }
            assert_eq!(clicked, Some(Selection::Rest(0)));
        }
    }
}
