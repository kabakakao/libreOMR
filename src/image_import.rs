//! Shared image decoding for the desktop app and offline benchmarks.
use image::{DynamicImage, GrayImage, ImageFormat};

pub fn decode_image<Reader: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<Reader>,
) -> Result<GrayImage, String> {
    reader = reader
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    if reader.format() == Some(ImageFormat::Tiff) {
        let mut stream = reader.into_inner();
        let start = stream
            .stream_position()
            .map_err(|error| error.to_string())?;
        if let Some(image) = crate::tiff_import::decode_fax3(&mut stream)? {
            return Ok(image);
        }
        stream
            .seek(std::io::SeekFrom::Start(start))
            .map_err(|error| error.to_string())?;
        reader = image::ImageReader::with_format(stream, ImageFormat::Tiff);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::image_limits::MAX_SIDE);
    limits.max_image_height = Some(crate::image_limits::MAX_SIDE);
    limits.max_alloc = Some(crate::image_limits::MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|error| {
        crate::i18n::format(
            "Could not load image: {error}",
            &[("error", format!("{}", error))],
        )
    })?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    crate::image_limits::validate_dimensions(width, height)?;
    let image = DynamicImage::from_decoder(decoder).map_err(|error| {
        crate::i18n::format(
            "Could not load image: {error}",
            &[("error", format!("{}", error))],
        )
    })?;
    let mut opaque = image.to_rgba8();
    for pixel in opaque.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in 0..3 {
            pixel[channel] =
                ((u16::from(pixel[channel]) * alpha + 255 * (255 - alpha)) / 255) as u8;
        }
        pixel[3] = 255;
    }
    Ok(DynamicImage::ImageRgba8(opaque).to_luma8())
}
