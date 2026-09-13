//! Frame structures and pixel formats for camera capture.

/// Supported pixel formats for camera capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// YUYV 4:2:2 (2 bytes per pixel)
    Yuyv,
    /// RGB 8:8:8 (3 bytes per pixel)
    Rgb24,
    /// Grayscale 8-bit (1 byte per pixel)
    Grey,
    /// Motion-JPEG compressed stream
    Mjpeg,
}

impl PixelFormat {
    /// Returns the fourcc identifier string for the format.
    pub const fn fourcc_str(self) -> &'static str {
        match self {
            Self::Yuyv => "YUYV",
            Self::Rgb24 => "RGB3",
            Self::Grey => "GREY",
            Self::Mjpeg => "MJPG",
        }
    }

    /// Calculates expected uncompressed byte size for a given resolution.
    /// Returns None for compressed formats like MJPEG.
    pub const fn expected_buffer_size(self, width: u32, height: u32) -> Option<usize> {
        let pixels = (width as usize).saturating_mul(height as usize);
        match self {
            Self::Yuyv => Some(pixels.saturating_mul(2)),
            Self::Rgb24 => Some(pixels.saturating_mul(3)),
            Self::Grey => Some(pixels),
            Self::Mjpeg => None,
        }
    }
}

/// A captured video frame with monotonic metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Raw pixel or compressed byte buffer.
    pub data: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Monotonic capture timestamp in nanoseconds.
    pub timestamp_mono_ns: u64,
    /// Pixel format of the buffer.
    pub format: PixelFormat,
    /// Sequential frame counter.
    pub sequence: u64,
}

impl Frame {
    /// Creates a new frame with the specified metadata.
    pub fn new(
        data: Vec<u8>,
        width: u32,
        height: u32,
        timestamp_mono_ns: u64,
        format: PixelFormat,
        sequence: u64,
    ) -> Self {
        Self {
            data,
            width,
            height,
            timestamp_mono_ns,
            format,
            sequence,
        }
    }

    /// Computes the age of the frame in milliseconds relative to a given monotonic timestamp.
    pub fn age_ms(&self, now_mono_ns: u64) -> u64 {
        let diff_ns = now_mono_ns.saturating_sub(self.timestamp_mono_ns);
        diff_ns.checked_div(1_000_000).unwrap_or(0)
    }

    /// Returns true if the frame age exceeds the specified maximum allowed age in milliseconds.
    pub fn is_stale(&self, now_mono_ns: u64, max_age_ms: u64) -> bool {
        self.age_ms(now_mono_ns) > max_age_ms
    }
}
