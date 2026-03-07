//! Image compression utilities for LLM vision input.

use std::io::Cursor;

/// Default maximum edge length in pixels (Anthropic recommendation).
pub const DEFAULT_MAX_EDGE: u32 = 1568;
/// Default JPEG quality (0-100).
pub const DEFAULT_QUALITY: u8 = 85;

/// Compress an image: resize so neither edge exceeds `max_edge`, then encode
/// as JPEG.
///
/// Returns `(jpeg_bytes, "image/jpeg")`.
pub fn compress_image(
    bytes: &[u8],
    max_edge: u32,
    quality: u8,
) -> anyhow::Result<(Vec<u8>, String)> {
    let img = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;

    let (w, h) = (img.width(), img.height());
    let img = if w > max_edge || h > max_edge {
        let ratio = max_edge as f64 / w.max(h) as f64;
        let new_w = (w as f64 * ratio).round() as u32;
        let new_h = (h as f64 * ratio).round() as u32;
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    img.write_with_encoder(encoder)?;

    Ok((buf, "image/jpeg".to_owned()))
}
