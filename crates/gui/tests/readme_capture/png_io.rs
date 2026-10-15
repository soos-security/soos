//! Minimal PNG I/O for the README snapshots: RGB8, only IHDR/IDAT/IEND, no metadata.

use std::io::Cursor;

use super::raster::RgbaImage;

/// Encodes an opaque image as an RGB8 PNG (high compression, adaptive filter, no ancillary
/// chunk), deterministic for a given image.
///
/// # Panics
///
/// If the PNG encoder fails (test failure).
pub fn encode_png(image: &RgbaImage) -> Vec<u8> {
    let rgb: Vec<u8> = image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::High);
        encoder.set_filter(png::Filter::Adaptive);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&rgb).expect("PNG data");
        writer.finish().expect("PNG end");
    }
    out
}

/// Decodes an 8-bit RGB or RGBA PNG into an opaque RGBA image.
///
/// # Errors
///
/// Any decoding error or an unsupported colour type.
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG too large".to_string())?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("unsupported bit depth {:?}", info.bit_depth));
    }
    let data = buffer.get(..info.buffer_size()).unwrap_or(&[]);
    let pixels: Vec<u8> = match info.color_type {
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        png::ColorType::Rgba => data.to_vec(),
        other => return Err(format!("unsupported colour type {other:?}")),
    };
    Ok(RgbaImage {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Chunk types of a PNG, in order (for the metadata test).
pub fn png_chunk_types(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 8usize;
    while let Some(header) = bytes.get(offset..offset + 8) {
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        out.push(String::from_utf8_lossy(&header[4..8]).into_owned());
        offset += 12 + len;
    }
    out
}

/// Fraction of pixels whose largest channel difference exceeds `channel_tolerance`; `None`
/// when the sizes differ.
pub fn diff_fraction(a: &RgbaImage, b: &RgbaImage, channel_tolerance: u8) -> Option<f64> {
    if a.width != b.width || a.height != b.height || a.pixels.len() != b.pixels.len() {
        return None;
    }
    let total = a.pixels.len() / 4;
    if total == 0 {
        return Some(0.0);
    }
    let differing = a
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0.iter())
        .filter(|(pa, pb)| {
            pa.iter()
                .zip(pb.iter())
                .any(|(x, y)| x.abs_diff(*y) > channel_tolerance)
        })
        .count();
    Some(differing as f64 / total as f64)
}
