//! Extract positioned Unicode with the same page transform as PDF rasterization.
use crate::model::{SymbolBounds, TextGlyph};
use hayro::hayro_interpret::{self as pdf, font::Glyph, hayro_cmap::BfString, util::TransformExt};
use hayro::vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape};

#[derive(Default)]
struct TextDevice {
    glyphs: Vec<TextGlyph>,
}

impl<'a> pdf::Device<'a> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<pdf::SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: pdf::BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &pdf::Paint<'a>, _: &pdf::PathDrawMode) {}
    fn push_clip_path(&mut self, _: &pdf::ClipPath) {}
    fn pop_clip_path(&mut self) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<pdf::SoftMask<'a>>, _: pdf::BlendMode) {
    }
    fn pop_transparency_group(&mut self) {}
    fn draw_image(&mut self, _: pdf::Image<'a, '_>, _: Affine) {}
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _: &pdf::Paint<'a>,
        mode: &pdf::GlyphDrawMode,
    ) {
        if matches!(mode, pdf::GlyphDrawMode::Invisible) {
            return;
        }
        let Glyph::Outline(outline) = glyph else {
            return;
        };
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => return,
        };
        if text
            .chars()
            .any(|c| c.is_control() || ('\u{e000}'..='\u{f8ff}').contains(&c))
        {
            return;
        }
        let matrix = transform * glyph_transform;
        let baseline = (matrix * Point::ORIGIN).y as f32;
        let bounds = (matrix * outline.outline()).bounding_box();
        // Spaces have no contour; keep them to preserve explicit word boundaries.
        let bounds = if bounds.is_zero_area() && text.chars().all(char::is_whitespace) {
            let origin = matrix * Point::ORIGIN;
            let advance = matrix * Point::new(outline.advance_width().unwrap_or(250.0) as f64, 0.0);
            Rect::new(
                origin.x,
                baseline as f64 - 1.0,
                advance.x.max(origin.x + 0.1),
                baseline as f64,
            )
        } else {
            bounds
        };
        let bounds = SymbolBounds {
            left: bounds.x0 as f32,
            top: bounds.y0 as f32,
            right: bounds.x1 as f32,
            bottom: bounds.y1 as f32,
        };
        if bounds.valid() && baseline.is_finite() && self.glyphs.len() < 200_000 {
            self.glyphs.push(TextGlyph {
                text,
                baseline,
                bounds,
            });
        }
    }
}

pub(crate) fn extract(page: &pdf::hayro_syntax::page::Page<'_>) -> Vec<TextGlyph> {
    let scale = f64::from(crate::document_import::PDF_DPI / 72.0);
    let transform = Affine::scale(scale) * page.initial_transform(true).to_kurbo();
    let (width, height) = page.render_dimensions();
    let cache = pdf::InterpreterCache::new();
    let mut context = pdf::Context::new(
        transform,
        Rect::new(
            0.0,
            0.0,
            f64::from(width) * scale,
            f64::from(height) * scale,
        ),
        &cache,
        page.xref(),
        pdf::InterpreterSettings::default(),
    );
    let mut device = TextDevice::default();
    pdf::interpret_page(page, &mut context, &mut device);
    device.glyphs
}
