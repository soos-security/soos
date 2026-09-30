//! Shared test fixtures: synthetic camera frames, synthetic PAD presentations and minimal
//! hand-encoded ONNX models.
//!
//! This file is the library root of the dev-only workspace crate `soos-test-fixtures`
//! (`tests/fixtures/Cargo.toml`); new tests depend on it through `[dev-dependencies]`.
//! Three legacy test files still include it through `#[path]` (frozen by
//! `tests/invariants/src/fixtures_contract.rs`). Face embeddings are 512D and no embedding
//! fixture is provided here.

#![allow(
    dead_code,
    reason = "Legacy #[path] includes each compile only the fixtures they use; the soos-test-fixtures crate itself needs no allowance"
)]

pub mod synthetic {
    use soos_camera_v4l::{Frame, PixelFormat};

    /// Generates a synthetic RGB24 frame with a solid background and an optional colored rectangle.
    pub fn create_synthetic_rgb_frame(width: u32, height: u32, bg_color: [u8; 3]) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for _ in 0..pixel_count {
            data.extend_from_slice(&bg_color);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic YUYV frame (4:2:2).
    pub fn create_synthetic_yuyv_frame(width: u32, height: u32, y: u8, u: u8, v: u8) -> Frame {
        let num_pairs = (width as usize).saturating_mul(height as usize) / 2;
        let mut data = Vec::with_capacity(num_pairs.saturating_mul(4));
        for _ in 0..num_pairs {
            data.extend_from_slice(&[y, u, y, v]);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Yuyv, 1)
    }

    /// Generates a synthetic Grayscale frame.
    pub fn create_synthetic_grey_frame(width: u32, height: u32, value: u8) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let data = vec![value; pixel_count];
        Frame::new(data, width, height, 1_000_000, PixelFormat::Grey, 1)
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_is_multiple_of,
    reason = "Synthetic fixture pixel generation and coordinate math"
)]
pub mod pad {
    use soos_camera_v4l::{Frame, PixelFormat};

    /// Generates a synthetic genuine live face camera frame.
    /// Real skin tones, natural high-frequency texture gradient, without moire or paper border.
    pub fn create_live_face_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            // Natural skin tone spectrum with subtle organic gradient
            let r = 210u8.saturating_add(((i % 17) as u8) / 2);
            let g = 160u8.saturating_add(((i % 13) as u8) / 2);
            let b = 140u8.saturating_add(((i % 11) as u8) / 2);
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic printed photograph presentation attack frame.
    /// Flat reflectance, paper border artifacts, and low dynamic range.
    pub fn create_printed_photo_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            let x = (i % (width as usize)) as u32;
            let y = (i / (width as usize)) as u32;

            // White paper border around photo
            if x < 10 || x > width.saturating_sub(10) || y < 10 || y > height.saturating_sub(10) {
                data.push(255);
                data.push(255);
                data.push(255);
            } else {
                // Reduced contrast, paper texture noise
                let r = 180u8.saturating_sub(((i % 5) as u8) * 4);
                let g = 140u8.saturating_sub(((i % 5) as u8) * 3);
                let b = 120u8.saturating_sub(((i % 5) as u8) * 3);
                data.push(r);
                data.push(g);
                data.push(b);
            }
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }

    /// Generates a synthetic digital screen replay presentation attack frame.
    /// High-frequency LCD pixel grid, backlight glare, and moire pattern frequency.
    pub fn create_screen_replay_frame(width: u32, height: u32) -> Frame {
        let pixel_count = (width as usize).saturating_mul(height as usize);
        let mut data = Vec::with_capacity(pixel_count.saturating_mul(3));
        for i in 0..pixel_count {
            let x = (i % (width as usize)) as u32;
            let y = (i / (width as usize)) as u32;

            // Periodic subpixel RGB grid / moire interference pattern
            let moire = if (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13))) % 4 == 0 {
                40u8
            } else {
                0u8
            };
            let r = 160u8.saturating_add(moire);
            let g = 190u8.saturating_add(moire); // Slight blue/green screen backlight tint
            let b = 220u8.saturating_add(moire);
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }
}

pub mod onnx {
    //! Minimal hand-encoded ONNX models for wiring tests that need a real ORT session
    //! without any model file on disk.

    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn field_varint(field: u64, value: u64, out: &mut Vec<u8>) {
        varint(field << 3, out);
        varint(value, out);
    }

    fn field_bytes(field: u64, bytes: &[u8], out: &mut Vec<u8>) {
        varint((field << 3) | 2, out);
        varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    /// `ValueInfoProto` for a float tensor named `name` with shape `[1, 3]`.
    fn float_value_info(name: &str) -> Vec<u8> {
        let mut dims = Vec::new();
        for dim_value in [1u64, 3u64] {
            let mut dim = Vec::new();
            field_varint(1, dim_value, &mut dim); // Dimension.dim_value
            field_bytes(1, &dim, &mut dims); // TensorShapeProto.dim
        }
        let mut tensor_type = Vec::new();
        field_varint(1, 1, &mut tensor_type); // Tensor.elem_type = FLOAT
        field_bytes(2, &dims, &mut tensor_type); // Tensor.shape
        let mut type_proto = Vec::new();
        field_bytes(1, &tensor_type, &mut type_proto); // TypeProto.tensor_type
        let mut value_info = Vec::new();
        field_bytes(1, name.as_bytes(), &mut value_info); // ValueInfoProto.name
        field_bytes(2, &type_proto, &mut value_info); // ValueInfoProto.type
        value_info
    }

    /// Serialized `ModelProto` of a single-node `Identity` graph (`x[1,3] -> y[1,3]`).
    ///
    /// The bytes are deterministic and independent of any file under `/var/lib/soos`, so
    /// production factories that take an ORT session can be exercised in CI.
    pub fn minimal_identity_model() -> Vec<u8> {
        let mut node = Vec::new();
        field_bytes(1, b"x", &mut node); // NodeProto.input
        field_bytes(2, b"y", &mut node); // NodeProto.output
        field_bytes(4, b"Identity", &mut node); // NodeProto.op_type

        let mut graph = Vec::new();
        field_bytes(1, &node, &mut graph); // GraphProto.node
        field_bytes(2, b"soos_identity", &mut graph); // GraphProto.name
        field_bytes(11, &float_value_info("x"), &mut graph); // GraphProto.input
        field_bytes(12, &float_value_info("y"), &mut graph); // GraphProto.output

        let mut opset = Vec::new();
        field_varint(2, 13, &mut opset); // OperatorSetIdProto.version

        let mut model = Vec::new();
        field_varint(1, 7, &mut model); // ModelProto.ir_version
        field_bytes(7, &graph, &mut model); // ModelProto.graph
        field_bytes(8, &opset, &mut model); // ModelProto.opset_import
        model
    }
}
