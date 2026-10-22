pub const MAX_SIDE: u32 = 10_000;
pub const MAX_PIXELS: u64 = 64_000_000;
pub const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;

pub fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width < 20
        || height < 20
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(format!(
            "Bild hat {width} x {height} Pixel. Erlaubt sind mindestens 20 x 20 Pixel, maximal {MAX_SIDE} Pixel pro Kante und maximal {} Megapixel.",
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_high_resolution_scans_within_budget() {
        assert!(validate_dimensions(4393, 6783).is_ok());
        assert!(validate_dimensions(8000, 8000).is_ok());
        assert!(validate_dimensions(10_000, 6400).is_ok());
    }

    #[test]
    fn rejects_invalid_and_excessive_dimensions() {
        for (width, height) in [
            (0, 20),
            (19, 100),
            (100, 19),
            (10_001, 20),
            (20, 10_001),
            (8001, 8000),
            (u32::MAX, u32::MAX),
        ] {
            let error = validate_dimensions(width, height).unwrap_err();
            assert!(error.contains(&format!("{width} x {height}")));
        }
    }
}
