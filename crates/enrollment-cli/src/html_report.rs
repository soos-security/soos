#![allow(
    unknown_lints,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::same_item_push,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::chunks_exact_to_as_chunks,
    reason = "Diagnostic tool encoding logic requires integer arithmetic and bounds-checked indexing"
)]

pub fn encode_bmp(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let row_stride = ((width * 3 + 3) & !3) as usize;
    let file_size = 54 + (row_stride * height as usize) as u32;
    let mut bmp = Vec::with_capacity(file_size as usize);

    // BMP Header
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&54u32.to_le_bytes());

    // DIB Header (BITMAPINFOHEADER)
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    // Positive height for standard bottom-up bitmap (supported by all browsers)
    bmp.extend_from_slice(&height.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&24u16.to_le_bytes()); // bpp
    bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    bmp.extend_from_slice(&((row_stride * height as usize) as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    // Pixels (RGB to BGR), bottom-up (reverse y loop)
    let w3 = (width * 3) as usize;
    for y in (0..height as usize).rev() {
        let row_start = y * w3;
        let row_end = row_start + w3;
        let row = &rgb[row_start..row_end];
        for chunk in row.chunks_exact(3) {
            bmp.push(chunk[2]); // B
            bmp.push(chunk[1]); // G
            bmp.push(chunk[0]); // R
        }
        for _ in 0..(row_stride - w3) {
            bmp.push(0);
        }
    }
    bmp
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity((data.len() * 4 / 3) + 4);
    let mut chunks = data.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        result.push(BASE64_ALPHABET[((n >> 18) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((n >> 12) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((n >> 6) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[(n & 63) as usize] as char);
    }
    let rem = chunks.remainder();
    if rem.len() == 1 {
        let n = (rem[0] as u32) << 16;
        result.push(BASE64_ALPHABET[((n >> 18) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((n >> 12) & 63) as usize] as char);
        result.push('=');
        result.push('=');
    } else if rem.len() == 2 {
        let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
        result.push(BASE64_ALPHABET[((n >> 18) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((n >> 12) & 63) as usize] as char);
        result.push(BASE64_ALPHABET[((n >> 6) & 63) as usize] as char);
        result.push('=');
    }
    result
}

use soos_inference_ort::FaceDetection;

/// Renders the standalone HTML debug report.
///
/// `embedded_frame_base64` is the base64-encoded raw RGB24 frame. It is biometric
/// data: when `None` (the default for `debug-vision`) the report draws detection
/// geometry only on a blank canvas and explains how to opt in; when `Some`, the frame
/// is decoded on the canvas and the report carries a visible privacy warning.
pub fn generate_html_report(
    width: u32,
    height: u32,
    embedded_frame_base64: Option<&str>,
    detections: &[FaceDetection],
) -> String {
    let mut boxes_js = String::new();
    for (i, det) in detections.iter().enumerate() {
        let b = &det.box_;
        boxes_js.push_str(&format!(
            "ctx.strokeStyle = 'red'; ctx.lineWidth = 3; ctx.strokeRect({}, {}, {}, {});\n",
            b.x1,
            b.y1,
            b.x2 - b.x1,
            b.y2 - b.y1
        ));
        boxes_js.push_str(&format!(
            "ctx.fillStyle = 'red'; ctx.font = '16px Arial'; ctx.fillText('Face {} ({:.2})', {}, {});\n",
            i, det.score, b.x1, b.y1 - 5.0
        ));

        if let Some(lms) = &det.landmarks {
            let pts = [
                lms.left_eye,
                lms.right_eye,
                lms.nose,
                lms.mouth_left,
                lms.mouth_right,
            ];
            for pt in &pts {
                boxes_js.push_str(&format!(
                    "ctx.fillStyle = 'lime'; ctx.beginPath(); ctx.arc({}, {}, 3, 0, 2*Math.PI); ctx.fill();\n",
                    pt.x, pt.y
                ));
            }
        }
    }

    let (notice_html, frame_js) = match embedded_frame_base64 {
        Some(b64) => (
            "<p class=\"warning\"><strong>Privacy warning:</strong> this report embeds a raw camera frame \
             (biometric data). Keep it root-only and delete it after use.</p>"
                .to_string(),
            format!(
                r#"// Decode the raw RGB24 base64 frame
        const b64 = "{b64}";
        const bin = atob(b64);
        const imgData = ctx.createImageData({width}, {height});
        let j = 0;
        for (let i = 0; i < bin.length; i += 3) {{
            imgData.data[j++] = bin.charCodeAt(i);
            imgData.data[j++] = bin.charCodeAt(i+1);
            imgData.data[j++] = bin.charCodeAt(i+2);
            imgData.data[j++] = 255; // Alpha
        }}
        ctx.putImageData(imgData, 0, 0);"#
            ),
        ),
        None => (
            "<p class=\"notice\">Camera frame not embedded: only detection geometry is shown. \
             Re-run with <code>--embed-frame</code> to include the raw camera frame (biometric data).</p>"
                .to_string(),
            "// No frame embedded: geometry-only report\n        \
             ctx.fillStyle = '#000'; ctx.fillRect(0, 0, canvas.width, canvas.height);"
                .to_string(),
        ),
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>SOOS Vision Debugger</title>
    <style>
        body {{ font-family: sans-serif; background: #111; color: #eee; margin: 20px; }}
        h1 {{ margin-top: 0; }}
        .container {{ display: flex; flex-direction: column; align-items: flex-start; }}
        canvas {{ border: 2px solid #444; background: #000; margin-top: 10px; max-width: 100%; height: auto; }}
        .info {{ background: #222; padding: 15px; border-radius: 8px; margin-bottom: 20px; border: 1px solid #333; }}
        .warning {{ color: #ffb347; }}
        .notice {{ color: #9ecbff; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>SOOS Vision Debugger</h1>
        <div class="info">
            <p><strong>Detections:</strong> {detections_count}</p>
            <p><strong>Resolution:</strong> {width}x{height}</p>
            {notice_html}
        </div>
        <canvas id="canvas" width="{width}" height="{height}"></canvas>
    </div>
    <script>
        const canvas = document.getElementById('canvas');
        const ctx = canvas.getContext('2d');

        {frame_js}

        // Draw detections
        {boxes_js}
    </script>
</body>
</html>"#,
        detections_count = detections.len(),
    )
}
