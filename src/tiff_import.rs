use std::io::{Read, Seek, SeekFrom};

use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
use image::GrayImage;
use tiff::{decoder::Decoder, tags::Tag};

/// Enumerate the main IFD chain without decompressing any image pixels.
pub(crate) fn page_offsets(reader: impl Read + Seek, max_pages: usize) -> Result<Vec<u64>, String> {
    let mut decoder = Decoder::new(reader).map_err(|error| {
        crate::i18n::format(
            "Cannot read TIFF: {error}",
            &[("error", format!("{}", error))],
        )
    })?;
    let mut offsets = Vec::new();
    loop {
        if offsets.len() >= max_pages {
            return Err(crate::i18n::format(
                "TIFF has more than {max_pages} pages.",
                &[("max_pages", format!("{}", max_pages))],
            ));
        }
        offsets.push(
            decoder
                .ifd_pointer()
                .ok_or(crate::i18n::tr("TIFF page has no image directory."))?
                .0,
        );
        if !decoder.more_images() {
            break;
        }
        decoder.next_image().map_err(|error| {
            crate::i18n::format(
                "Cannot read TIFF page {0}: {error}",
                &[
                    ("0", format!("{}", offsets.len() + 1)),
                    ("error", format!("{}", error)),
                ],
            )
        })?;
    }
    Ok(offsets)
}

/// Expose the selected IFD as the first page to the existing image/Fax3 decoders.
/// Only the header's directory pointer is overlaid; all strip offsets remain
/// relative to the original stream and the source file is never modified.
pub(crate) fn decode_page(mut reader: impl Read + Seek, offset: u64) -> Result<GrayImage, String> {
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut header = [0; 16];
    reader
        .read_exact(&mut header[..8])
        .map_err(|error| error.to_string())?;
    let little = &header[..2] == b"II";
    let big = &header[..2] == b"MM";
    let version = if little {
        u16::from_le_bytes([header[2], header[3]])
    } else {
        u16::from_be_bytes([header[2], header[3]])
    };
    let header_len = match (little || big, version) {
        (true, 42) => {
            let offset = u32::try_from(offset).map_err(|error| error.to_string())?;
            header[4..8].copy_from_slice(&if little {
                offset.to_le_bytes()
            } else {
                offset.to_be_bytes()
            });
            8
        }
        (true, 43) => {
            reader
                .read_exact(&mut header[8..16])
                .map_err(|error| error.to_string())?;
            header[8..16].copy_from_slice(&if little {
                offset.to_le_bytes()
            } else {
                offset.to_be_bytes()
            });
            16
        }
        _ => return Err(crate::i18n::tr("Invalid TIFF header.").into()),
    };
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let selected = PageReader {
        reader,
        header,
        header_len,
        position: 0,
    };
    crate::image_import::decode_image(image::ImageReader::with_format(
        std::io::BufReader::new(selected),
        image::ImageFormat::Tiff,
    ))
}

struct PageReader<R> {
    reader: R,
    header: [u8; 16],
    header_len: usize,
    position: u64,
}

impl<R: Read> Read for PageReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.reader.read(buffer)?;
        if self.position < self.header_len as u64 {
            let start = self.position as usize;
            let length = count.min(self.header_len - start);
            buffer[..length].copy_from_slice(&self.header[start..start + length]);
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Seek> Seek for PageReader<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.position = self.reader.seek(position)?;
        Ok(self.position)
    }
}

pub fn decode_fax3(reader: &mut (impl Read + Seek)) -> Result<Option<GrayImage>, String> {
    decode(reader).map_err(|error| {
        crate::i18n::format(
            "Could not load Fax3 TIFF: {error}",
            &[("error", error.to_string())],
        )
    })
}

fn decode(reader: &mut (impl Read + Seek)) -> Result<Option<GrayImage>, String> {
    let mut decoder = Decoder::new(reader).map_err(|error| error.to_string())?;
    if decoder
        .find_tag_unsigned::<u32>(Tag::Compression)
        .map_err(|error| error.to_string())?
        .unwrap_or(1)
        != 3
    {
        return Ok(None);
    }
    let (width, height) = decoder.dimensions().map_err(|error| error.to_string())?;
    crate::image_limits::validate_dimensions(width, height)?;
    let mut tag = |key, default| {
        decoder
            .find_tag_unsigned::<u32>(key)
            .map(|value| value.unwrap_or(default))
            .map_err(|error| error.to_string())
    };
    let bits = tag(Tag::BitsPerSample, 1)?;
    let samples = tag(Tag::SamplesPerPixel, 1)?;
    let photometric = tag(Tag::PhotometricInterpretation, 0)?;
    let fill_order = tag(Tag::FillOrder, 1)?;
    let options = tag(Tag::Unknown(292), 0)?;
    let orientation = tag(Tag::Orientation, 1)?;
    let rows_per_strip = tag(Tag::RowsPerStrip, height)?.min(height);
    if bits != 1 || samples != 1 || photometric > 1 || !(1..=2).contains(&fill_order) {
        return Err(crate::i18n::tr(
            "Fax3 requires a black-and-white image with one bit per pixel.",
        )
        .into());
    }
    if options & !5 != 0 {
        return Err(crate::i18n::tr("Fax3 with uncompressed regions is not yet supported.").into());
    }
    if orientation != 1 {
        return Err(crate::i18n::tr(
            "This TIFF orientation is not yet supported for Fax3. Please save the image upright.",
        )
        .into());
    }
    if rows_per_strip == 0 {
        return Err(crate::i18n::tr("Invalid strip height.").into());
    }
    let offsets = decoder
        .get_tag_u64_vec(Tag::StripOffsets)
        .map_err(|error| error.to_string())?;
    let counts = decoder
        .get_tag_u64_vec(Tag::StripByteCounts)
        .map_err(|error| error.to_string())?;
    if offsets.len() != height.div_ceil(rows_per_strip) as usize
        || offsets.len() != counts.len()
        || counts
            .iter()
            .any(|&size| size == 0 || size > 128 * 1024 * 1024)
        || counts
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count))
            .is_none_or(|total| total > 256 * 1024 * 1024)
    {
        return Err(crate::i18n::tr("Invalid or oversized TIFF image strips.").into());
    }
    let mut image = GrayImage::new(width, height);
    for (index, (&offset, &count)) in offsets.iter().zip(&counts).enumerate() {
        let mut compressed = vec![0; count as usize];
        decoder
            .inner()
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        decoder
            .inner()
            .read_exact(&mut compressed)
            .map_err(|error| error.to_string())?;
        if fill_order == 2 {
            for byte in &mut compressed {
                *byte = byte.reverse_bits();
            }
        }
        let first_row = index as u32 * rows_per_strip;
        let rows = rows_per_strip.min(height - first_row);
        let settings = DecodeSettings {
            columns: width,
            rows,
            end_of_block: false,
            end_of_line: true,
            rows_are_byte_aligned: false,
            encoding: if options & 1 != 0 {
                EncodingMode::Group3_2D { k: 0 }
            } else {
                EncodingMode::Group3_1D
            },
            invert_black: photometric == 1,
        };
        let start = (first_row * width) as usize;
        let end = ((first_row + rows) * width) as usize;
        let mut output = Pixels {
            data: &mut image.as_mut()[start..end],
            width: width as usize,
            row: 0,
            column: 0,
            invalid: false,
        };
        hayro_ccitt::decode(&compressed, &mut output, &mut DecoderContext::new(settings)).map_err(
            |error| {
                crate::i18n::format(
                    "Damaged image strip {0}: {error}",
                    &[
                        ("0", format!("{}", index + 1)),
                        ("error", format!("{}", error)),
                    ],
                )
            },
        )?;
        if output.invalid || output.row != rows as usize {
            return Err(crate::i18n::format(
                "Image strip {0} is incomplete.",
                &[("0", format!("{}", index + 1))],
            ));
        }
    }
    Ok(Some(image))
}

struct Pixels<'a> {
    data: &'a mut [u8],
    width: usize,
    row: usize,
    column: usize,
    invalid: bool,
}

impl hayro_ccitt::Decoder for Pixels<'_> {
    fn push_pixels(&mut self, white: bool, count: u32) {
        let end = self.column + count as usize;
        if end > self.width || self.row >= self.data.len() / self.width {
            self.invalid = true;
            return;
        }
        let start = self.row * self.width;
        self.data[start + self.column..start + end].fill(if white { 255 } else { 0 });
        self.column = end;
    }

    fn next_line(&mut self) {
        self.invalid |= self.column != self.width;
        self.column = 0;
        self.row += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn big_endian_pages() -> Vec<u8> {
        let directory_size = 2 + 9 * 12 + 4;
        let pixels_start = 8 + 2 * directory_size;
        let mut bytes = b"MM\0*\0\0\0\x08".to_vec();
        for (width, offset, next) in [
            (30_u32, pixels_start, 8 + directory_size),
            (40, pixels_start + 600, 0),
        ] {
            let tags = [
                (256_u16, 4_u16, width),
                (257, 4, 20),
                (258, 3, 8),
                (259, 3, 1),
                (262, 3, 1),
                (273, 4, offset),
                (277, 3, 1),
                (278, 4, 20),
                (279, 4, width * 20),
            ];
            bytes.extend(9_u16.to_be_bytes());
            for (tag, kind, value) in tags {
                bytes.extend(tag.to_be_bytes());
                bytes.extend(kind.to_be_bytes());
                bytes.extend(1_u32.to_be_bytes());
                if kind == 3 {
                    bytes.extend((value as u16).to_be_bytes());
                    bytes.extend([0, 0]);
                } else {
                    bytes.extend(value.to_be_bytes());
                }
            }
            bytes.extend(next.to_be_bytes());
        }
        bytes.extend(vec![17; 600]);
        bytes.extend(vec![200; 800]);
        bytes
    }

    #[test]
    fn document_import_big_endian_tiff_selects_each_page_without_modifying_the_source() {
        let bytes = big_endian_pages();
        let offsets = page_offsets(Cursor::new(&bytes), 1000).unwrap();
        assert_eq!(offsets.len(), 2);
        for (index, (width, value)) in [(30, 17), (40, 200)].into_iter().enumerate() {
            let decoded = decode_page(Cursor::new(&bytes), offsets[index]).unwrap();
            assert_eq!(decoded.dimensions(), (width, 20));
            assert!(decoded.pixels().all(|pixel| pixel[0] == value));
        }
        assert_eq!(bytes, big_endian_pages());
    }

    #[test]
    fn document_import_tiff_metadata_rejects_cycles_truncation_and_excessive_page_counts() {
        let bytes = big_endian_pages();
        assert!(
            page_offsets(Cursor::new(&bytes), 1)
                .unwrap_err()
                .contains("mehr als 1")
        );
        let mut cycle = bytes.clone();
        cycle[8 + 2 + 9 * 12..8 + 2 + 9 * 12 + 4].copy_from_slice(&8_u32.to_be_bytes());
        assert!(page_offsets(Cursor::new(cycle), 1000).is_err());
        assert!(page_offsets(Cursor::new(&bytes[..130]), 1000).is_err());
        assert!(decode_page(Cursor::new(b"invalid header"), 8).is_err());
    }
}
