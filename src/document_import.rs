//! Discover document pages without rasterizing them, then decode one page per job.
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use hayro::{RenderCache, RenderSettings, hayro_interpret::InterpreterSettings, hayro_syntax::Pdf};
use image::{GrayImage, Pixel};

use crate::{
    image_import::decode_image,
    image_limits::{MAX_PIXELS, MAX_SIDE, validate_dimensions},
};

pub const PDF_DPI: f32 = 300.0;
const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DOCUMENT_PAGES: usize = 1000;

struct Document {
    path: PathBuf,
    kind: DocumentKind,
    page_count: usize,
}

enum DocumentKind {
    Image,
    Tiff(Vec<u64>),
    Pdf(Pdf),
}

/// A cheap, thread-safe reference to one page of a source document.
#[derive(Clone)]
pub struct DocumentPage {
    document: Arc<Document>,
    index: usize,
}

impl DocumentPage {
    pub fn name(&self) -> String {
        let name = self
            .document
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        if self.document.page_count == 1 {
            name.into_owned()
        } else {
            crate::i18n::format(
                "{name} – Page {0} of {1}",
                &[
                    ("name", format!("{}", name)),
                    ("0", format!("{}", self.index + 1)),
                    ("1", format!("{}", self.document.page_count)),
                ],
            )
        }
    }

    pub fn path(&self) -> &Path {
        &self.document.path
    }

    pub fn render(&self) -> Result<GrayImage, String> {
        let result = match &self.document.kind {
            DocumentKind::Image => decode_image(
                image::ImageReader::open(&self.document.path).map_err(|error| {
                    crate::i18n::format(
                        "Cannot read image: {error}",
                        &[("error", format!("{}", error))],
                    )
                })?,
            ),
            DocumentKind::Tiff(offsets) => crate::tiff_import::decode_page(
                BufReader::new(File::open(&self.document.path).map_err(|error| error.to_string())?),
                offsets[self.index],
            ),
            DocumentKind::Pdf(pdf) => render_pdf_page(pdf, self.index),
        };
        result.map_err(|error| format!("{}: {error}", self.name()))
    }
}

/// Read only page metadata here; expensive image decoding happens in `render`.
pub fn open_document(path: &Path) -> Result<Vec<DocumentPage>, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|error| {
        crate::i18n::format(
            "Cannot read file: {error}",
            &[("error", format!("{}", error))],
        )
    })?);
    let mut header = [0; 1024];
    let length = reader
        .read(&mut header)
        .map_err(|error| error.to_string())?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    let (kind, page_count) = if header[..length].windows(5).any(|bytes| bytes == b"%PDF-")
        || extension.eq_ignore_ascii_case("pdf")
    {
        if reader
            .get_ref()
            .metadata()
            .map_err(|error| error.to_string())?
            .len()
            > MAX_PDF_BYTES
        {
            return Err(crate::i18n::tr("PDF is larger than 256 MiB.").into());
        }
        let mut bytes = Vec::new();
        reader
            .take(MAX_PDF_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_PDF_BYTES {
            return Err(crate::i18n::tr("PDF is larger than 256 MiB.").into());
        }
        let pdf = Pdf::new(bytes).map_err(|error| match error {
            hayro::hayro_syntax::LoadPdfError::Decryption(_) => crate::i18n::tr("PDF is password-protected or cannot be decrypted. Please import an unprotected copy.").to_string(),
            _ => crate::i18n::tr("PDF is damaged or unreadable.").to_string(),
        })?;
        let count = pdf.pages().len();
        (DocumentKind::Pdf(pdf), count)
    } else if header[..length].starts_with(b"II*\0")
        || header[..length].starts_with(b"MM\0*")
        || header[..length].starts_with(b"II+\0")
        || header[..length].starts_with(b"MM\0+")
        || extension.eq_ignore_ascii_case("tif")
        || extension.eq_ignore_ascii_case("tiff")
    {
        let offsets = crate::tiff_import::page_offsets(reader, MAX_DOCUMENT_PAGES)?;
        let count = offsets.len();
        (DocumentKind::Tiff(offsets), count)
    } else {
        (DocumentKind::Image, 1)
    };
    if page_count == 0 || page_count > MAX_DOCUMENT_PAGES {
        return Err(crate::i18n::format(
            "Document has {page_count} pages. Between 1 and {MAX_DOCUMENT_PAGES} pages per file are allowed.",
            &[
                ("page_count", format!("{}", page_count)),
                ("MAX_DOCUMENT_PAGES", format!("{}", MAX_DOCUMENT_PAGES)),
            ],
        ));
    }
    let document = Arc::new(Document {
        path: path.to_path_buf(),
        kind,
        page_count,
    });
    Ok((0..page_count)
        .map(|index| DocumentPage {
            document: document.clone(),
            index,
        })
        .collect())
}

fn pdf_render_settings(width: f32, height: f32) -> Result<RenderSettings, String> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(crate::i18n::tr("PDF page has invalid dimensions.").into());
    }
    let scale = f64::from(PDF_DPI / 72.0);
    let pixel_width = (f64::from(width) * scale).ceil();
    let pixel_height = (f64::from(height) * scale).ceil();
    let reduction = 1.0_f64
        .min(f64::from(MAX_SIDE) / pixel_width)
        .min(f64::from(MAX_SIDE) / pixel_height)
        .min((MAX_PIXELS as f64 / (pixel_width * pixel_height)).sqrt());
    // Round down to keep both the side and area budgets, even at their boundaries.
    let pixel_width = (pixel_width * reduction).floor() as u32;
    let pixel_height = (pixel_height * reduction).floor() as u32;
    validate_dimensions(pixel_width, pixel_height)?;
    // Scale the contents as well as the viewport: a smaller viewport alone crops
    // the page. Render directly at this size to avoid an oversized RGBA buffer.
    let scale = scale
        .min(f64::from(pixel_width) / f64::from(width))
        .min(f64::from(pixel_height) / f64::from(height)) as f32;
    Ok(RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: Some(pixel_width as u16),
        height: Some(pixel_height as u16),
        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
    })
}

fn render_pdf_page(pdf: &Pdf, index: usize) -> Result<GrayImage, String> {
    let page = &pdf.pages()[index];
    let (width, height) = page.render_dimensions();
    let settings = pdf_render_settings(width, height)?;
    let pixmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &settings,
    );
    let pixels = pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| image::Rgb([pixel[0], pixel[1], pixel[2]]).to_luma()[0])
        .collect();
    GrayImage::from_raw(u32::from(pixmap.width()), u32::from(pixmap.height()), pixels)
        .ok_or_else(|| crate::i18n::tr("Could not rasterize PDF page.").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tiff::encoder::{TiffEncoder, colortype};

    fn fixture(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn document_import_pdf_renders_each_page_at_300_dpi_with_crop_rotation_and_white_background() {
        fn assert_thread_safe<T: Send + Sync>() {}
        assert_thread_safe::<DocumentPage>();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("score.PDF");
        fixture(&path, include_bytes!("../tests/fixtures/two-pages.pdf"));
        let pages = open_document(&path).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].name(), "score.PDF – Seite 1 von 2");
        assert_eq!(pages[1].name(), "score.PDF – Seite 2 von 2");
        assert!(Arc::ptr_eq(&pages[0].document, &pages[1].document));
        let first = pages[0].render().unwrap();
        let second = pages[1].render().unwrap();
        assert_eq!(first.dimensions(), (300, 400));
        assert_eq!(second.dimensions(), (250, 300));
        assert_eq!(first.get_pixel(10, 10)[0], 0);
        assert_eq!(first.get_pixel(290, 10)[0], 255);
        assert!((127..=128).contains(&second.get_pixel(100, 100)[0]));
        // Document bytes and parsed resources stay valid after the source is closed/removed.
        std::fs::remove_file(path).unwrap();
        assert_eq!(pages[0].render().unwrap(), first);
    }

    #[test]
    fn document_import_tiff_preserves_page_order_dimensions_and_transparency() {
        let mut data = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new(&mut data).unwrap();
            encoder
                .write_image::<colortype::Gray8>(30, 40, &vec![10; 30 * 40])
                .unwrap();
            encoder = encoder.with_compression(tiff::encoder::Compression::Packbits);
            encoder
                .write_image::<colortype::RGBA8>(50, 60, &vec![0; 50 * 60 * 4])
                .unwrap();
            encoder = encoder.with_compression(tiff::encoder::Compression::Lzw);
            encoder
                .write_image::<colortype::Gray16>(20, 30, &vec![u16::MAX; 20 * 30])
                .unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scan.TIFF");
        fixture(&path, data.get_ref());
        let pages = open_document(&path).unwrap();
        assert_eq!(pages.len(), 3);
        for (index, (size, value)) in [((30, 40), 10), ((50, 60), 255), ((20, 30), 255)]
            .into_iter()
            .enumerate()
        {
            let image = pages[index].render().unwrap();
            assert_eq!(image.dimensions(), size);
            assert!(image.pixels().all(|pixel| pixel[0] == value));
            assert!(
                pages[index]
                    .name()
                    .ends_with(&format!("Seite {} von 3", index + 1))
            );
        }
        assert_eq!(std::fs::read(path).unwrap(), *data.get_ref());
    }

    #[test]
    fn document_import_bigtiff_and_plain_images_keep_all_pages() {
        let mut data = Cursor::new(Vec::new());
        {
            let mut encoder = TiffEncoder::new_big(&mut data).unwrap();
            encoder
                .write_image::<colortype::Gray8>(20, 25, &vec![42; 20 * 25])
                .unwrap();
            encoder
                .write_image::<colortype::Gray8>(25, 30, &vec![210; 25 * 30])
                .unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scan.tif");
        fixture(&path, data.get_ref());
        let pages = open_document(&path).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].render().unwrap().get_pixel(10, 10)[0], 42);
        assert_eq!(pages[1].render().unwrap().dimensions(), (25, 30));
        assert_eq!(pages[1].render().unwrap().get_pixel(10, 10)[0], 210);
        let png = directory.path().join("single.png");
        GrayImage::from_pixel(30, 40, image::Luma([123]))
            .save(&png)
            .unwrap();
        let pages = open_document(&png).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].name(), "single.png");
        assert_eq!(pages[0].path(), png);
        assert_eq!(pages[0].render().unwrap().get_pixel(0, 0)[0], 123);
    }

    #[test]
    fn document_import_pdf_reduces_resolution_to_fit_side_and_area_budgets() {
        for (width, height, expected) in [
            (14472.0, 21150.0, (6617, 9671)),
            (21150.0, 14472.0, (9671, 6617)),
            (12000.0, 6000.0, (10000, 5000)),
            (6000.0, 12000.0, (5000, 10000)),
            (9000.0, 9000.0, (8000, 8000)),
            (10000.0, 6400.0, (10000, 6400)),
        ] {
            let settings =
                pdf_render_settings(width / (PDF_DPI / 72.0), height / (PDF_DPI / 72.0)).unwrap();
            let size = (
                u32::from(settings.width.unwrap()),
                u32::from(settings.height.unwrap()),
            );
            assert_eq!(size, expected);
            validate_dimensions(size.0, size.1).unwrap();
            assert_eq!(settings.x_scale, settings.y_scale);
            assert!(settings.x_scale <= PDF_DPI / 72.0);
        }
        // Large finite PDF coordinates must not overflow before being scaled.
        let settings = pdf_render_settings(f32::MAX, f32::MAX).unwrap();
        assert_eq!(settings.width, settings.height);
        assert!((7999..=8000).contains(&settings.width.unwrap()));
    }

    #[test]
    fn document_import_oversized_pdf_renders_both_edges_without_cropping() {
        let pdf = Pdf::new(include_bytes!("../tests/fixtures/wide-page.pdf").to_vec()).unwrap();
        let image = render_pdf_page(&pdf, 0).unwrap();
        assert_eq!(image.dimensions(), (10000, 166));
        for y in [5, 160] {
            assert_eq!(image.get_pixel(10, y)[0], 0);
            assert_eq!(image.get_pixel(4900, y)[0], 0);
            assert_eq!(image.get_pixel(6000, y)[0], 255);
            assert_eq!(image.get_pixel(9900, y)[0], 0);
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_LARGE_PDF pointing to a local oversized PDF"]
    fn document_import_large_external_pdf_renders_all_pages() {
        let path = PathBuf::from(std::env::var_os("LIBREOMR_LARGE_PDF").unwrap());
        let pages = open_document(&path).unwrap();
        for page in pages {
            let image = page.render().unwrap();
            validate_dimensions(image.width(), image.height()).unwrap();
            assert!(image.pixels().any(|pixel| pixel[0] < 128));
            println!("{}: {} x {}", page.name(), image.width(), image.height());
        }
    }

    #[test]
    fn document_import_invalid_files_and_pdf_dimensions_report_errors() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["bad.pdf", "bad.tif"] {
            let path = directory.path().join(name);
            fixture(&path, b"broken");
            assert!(open_document(&path).is_err());
        }
        let path = directory.path().join("pdf-without-extension");
        fixture(&path, include_bytes!("../tests/fixtures/two-pages.pdf"));
        assert_eq!(open_document(&path).unwrap().len(), 2);
        for (width, height) in [
            (f32::NAN, 72.0),
            (0.0, 72.0),
            (f32::INFINITY, 72.0),
            (72.0, f32::NAN),
            (72.0, -1.0),
            (1_000_000.0, 72.0),
            (72.0, 1_000_000.0),
            (1.0, 1.0),
        ] {
            assert!(pdf_render_settings(width, height).is_err());
        }
    }
}
