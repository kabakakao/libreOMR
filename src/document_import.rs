//! Discover document pages without rasterizing them, then decode one page per job.
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use hayro::{RenderCache, RenderSettings, hayro_interpret::InterpreterSettings, hayro_syntax::Pdf};
use image::{GrayImage, Pixel};

use crate::{image_import::decode_image, image_limits::validate_dimensions};

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
            format!(
                "{name} – Seite {} von {}",
                self.index + 1,
                self.document.page_count
            )
        }
    }

    pub fn path(&self) -> &Path {
        &self.document.path
    }

    pub fn render(&self) -> Result<GrayImage, String> {
        let result = match &self.document.kind {
            DocumentKind::Image => decode_image(
                image::ImageReader::open(&self.document.path)
                    .map_err(|error| format!("Bild nicht lesbar: {error}"))?,
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
    let mut reader =
        BufReader::new(File::open(path).map_err(|error| format!("Datei nicht lesbar: {error}"))?);
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
            return Err("PDF ist groesser als 256 MiB.".into());
        }
        let mut bytes = Vec::new();
        reader
            .take(MAX_PDF_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_PDF_BYTES {
            return Err("PDF ist groesser als 256 MiB.".into());
        }
        let pdf = Pdf::new(bytes).map_err(|error| match error {
            hayro::hayro_syntax::LoadPdfError::Decryption(_) => "PDF ist passwortgeschuetzt oder kann nicht entschluesselt werden. Bitte eine ungeschuetzte Kopie importieren.".to_string(),
            _ => "PDF ist beschaedigt oder unlesbar.".to_string(),
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
        return Err(format!(
            "Dokument hat {page_count} Seiten. Erlaubt sind 1 bis {MAX_DOCUMENT_PAGES} Seiten pro Datei."
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

fn pdf_dimensions(width: f32, height: f32) -> Result<(u32, u32), String> {
    let scale = PDF_DPI / 72.0;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("PDF-Seite hat ungueltige Abmessungen.".into());
    }
    let width = (width * scale).ceil() as u32;
    let height = (height * scale).ceil() as u32;
    validate_dimensions(width, height)?;
    Ok((width, height))
}

fn render_pdf_page(pdf: &Pdf, index: usize) -> Result<GrayImage, String> {
    let page = &pdf.pages()[index];
    let (width, height) = page.render_dimensions();
    let (width, height) = pdf_dimensions(width, height)?;
    let pixmap = hayro::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings {
            x_scale: PDF_DPI / 72.0,
            y_scale: PDF_DPI / 72.0,
            width: Some(width as u16),
            height: Some(height as u16),
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let pixels = pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| image::Rgb([pixel[0], pixel[1], pixel[2]]).to_luma()[0])
        .collect();
    GrayImage::from_raw(width, height, pixels)
        .ok_or_else(|| "PDF-Seite konnte nicht gerastert werden.".into())
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
    fn document_import_invalid_files_and_excessive_pdf_dimensions_report_errors() {
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
            (1_000_000.0, 72.0),
            (72.0, 1_000_000.0),
            (1.0, 1.0),
        ] {
            assert!(pdf_dimensions(width, height).is_err());
        }
    }
}
