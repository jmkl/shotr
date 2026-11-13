//! Pixel buffer + WebP encoding.
use webp::{Encoder, WebPMemory};

/// Top-down BGRA, alpha forced to 255.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// Lossy WebP, quality 90 (fast; set to lossless later if you prefer exact text).
/// Swaps B/R in place (the encoder wants RGBA), so call this AFTER the clipboard
/// copy: `img` is RGBA afterwards. WebP lossy max dimension is 16383.
pub fn encode_webp(img: &mut Image) -> Option<WebPMemory> {
    if img.w > 16383 || img.h > 16383 {
        return None;
    }
    for px in img.bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let mem = Encoder::from_rgba(&img.bgra, img.w, img.h).encode(90.0);
    if mem.len() == 0 {
        None
    } else {
        Some(mem)
    }
}
