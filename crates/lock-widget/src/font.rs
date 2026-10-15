//! Minimal 5 × 7 bitmap font covering exactly the characters of the widget's two labels
//! (`Scanning`, `Not recognized`). No font file or font crate is used.

/// Glyph cell width in font pixels.
pub const GLYPH_W: u32 = 5;
/// Glyph cell height in font pixels.
pub const GLYPH_H: u32 = 7;
/// Gap between two cells in font pixels.
pub const GLYPH_GAP: u32 = 1;

/// Rows of `c` (bit 4 = leftmost column); `None` for a character outside the label set (drawn
/// as a blank cell).
#[must_use]
pub fn glyph(c: char) -> Option<[u8; 7]> {
    let rows = match c {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'a' => [0, 0, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111],
        'c' => [0, 0, 0b01110, 0b10000, 0b10000, 0b10001, 0b01110],
        'd' => [
            0b00001, 0b00001, 0b01101, 0b10011, 0b10001, 0b10001, 0b01111,
        ],
        'e' => [0, 0, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110],
        'g' => [0, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110],
        'i' => [0b00100, 0, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110],
        'n' => [0, 0, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001],
        'o' => [0, 0, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110],
        'r' => [0, 0, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000],
        't' => [
            0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110,
        ],
        'z' => [0, 0, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111],
        _ => return None,
    };
    Some(rows)
}

/// Width in pixels of `text` at integer `scale` (saturating).
#[must_use]
pub fn text_width(text: &str, scale: u32) -> u32 {
    let cells = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    if cells == 0 {
        return 0;
    }
    cells
        .saturating_mul(GLYPH_W.saturating_add(GLYPH_GAP))
        .saturating_sub(GLYPH_GAP)
        .saturating_mul(scale)
}
