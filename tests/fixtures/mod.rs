//! Shared test fixtures: synthetic camera frames, synthetic PAD presentations and minimal
//! hand-encoded ONNX models.
//!
//! This file is the library root of the dev-only workspace crate `soos-test-fixtures`
//! (`tests/fixtures/Cargo.toml`); every test depends on it through `[dev-dependencies]`
//! (no `#[path]` include remains, enforced by `tests/invariants/src/fixtures_contract.rs`).
//! Face embeddings have the dimension of the shipped model (`EMBEDDING_DIMENSION` of
//! `soos-inference-ort`) and no embedding fixture is provided here.

#![allow(
    dead_code,
    reason = "Legacy #[path] includes each compile only the fixtures they use; the soos-test-fixtures crate itself needs no allowance"
)]

pub mod synthetic {
    use soos_frame::{Frame, PixelFormat};

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
    use soos_frame::{Frame, PixelFormat};

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

#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "Placeholder illustration geometry on bounded frame sizes (16..=4096)"
)]
pub mod readme {
    //! Rendered placeholder face for the README screenshots (GitHub #381, spec
    //! `AI/architect_spec_readme_capture.md` §3.3): a flat illustration drawn analytically, never
    //! a photo and never a real face.

    use soos_frame::{Frame, PixelFormat};

    /// Smallest and largest accepted frame side.
    pub const MIN_SIDE: u32 = 16;
    /// Largest accepted frame side.
    pub const MAX_SIDE: u32 = 4096;

    const BACKGROUND: [u8; 3] = [237, 241, 255]; // brand pale #EDF1FF
    const SHIRT: [u8; 3] = [0, 71, 187]; // brand blue #0047BB
    const SKIN: [u8; 3] = [232, 196, 170];
    const SKIN_SHADE: [u8; 3] = [214, 172, 146];
    const HAIR: [u8; 3] = [16, 24, 32]; // brand ink #101820
    const MOUTH: [u8; 3] = [229, 155, 220]; // brand pink #E59BDC

    fn in_ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
        let dx = (x - cx) / rx;
        let dy = (y - cy) / ry;
        dx * dx + dy * dy <= 1.0
    }

    /// Colour of the pixel centre `(x, y)` of a `w`×`h` illustration. The eyes, nose tip and
    /// mouth sit on the canonical landmarks of `MockFaceDetector::canonical_landmarks_for_box`
    /// for the centred box (eyes at x 42.1 % / 57.8 %, y 47.7 %; nose tip y 58.4 %; mouth
    /// y 69.5 %).
    fn colour(x: f32, y: f32, w: f32, h: f32) -> [u8; 3] {
        let (cx, cy) = (0.5 * w, 0.52 * h);
        let (rx, ry) = (0.2 * w, 0.3 * h);
        let eye_r = 0.018 * w;
        for ex in [0.421 * w, 0.578 * w] {
            if in_ellipse(x, y, ex, 0.477 * h, eye_r, eye_r) {
                return HAIR;
            }
        }
        if in_ellipse(x, y, cx, 0.695 * h, 0.066 * w, 0.022 * h) {
            return MOUTH;
        }
        if (x - cx).abs() <= 0.012 * w && y >= cy && y <= 0.584 * h {
            return SKIN_SHADE;
        }
        if in_ellipse(x, y, cx, cy, rx, ry) {
            // Hair cap on the top of the head.
            return if y < 0.38 * h { HAIR } else { SKIN };
        }
        // Neck, then shoulders.
        if (x - cx).abs() <= 0.06 * w && y > cy && y <= 0.9 * h {
            return SKIN_SHADE;
        }
        if in_ellipse(x, y, cx, 1.1 * h, 0.36 * w, 0.24 * h) {
            return SHIRT;
        }
        BACKGROUND
    }

    /// A rendered placeholder face: flat brand-coloured illustration (background, shoulders,
    /// head ellipse, hair cap, eyes, nose and mouth), no photo and no randomness. The head is
    /// centred in the box of `MockFaceDetector::new_centered_face(width, height, _)` (x 25–75 %,
    /// y 20–80 %). A side outside `16..=4096` yields a 16×16 frame of the background colour.
    pub fn create_placeholder_face_frame(width: u32, height: u32) -> Frame {
        let in_range = |side: u32| (MIN_SIDE..=MAX_SIDE).contains(&side);
        if !in_range(width) || !in_range(height) {
            let data = BACKGROUND.repeat((MIN_SIDE * MIN_SIDE) as usize);
            return Frame::new(data, MIN_SIDE, MIN_SIDE, 1_000_000, PixelFormat::Rgb24, 1);
        }
        let (w, h) = (width as f32, height as f32);
        let mut data = Vec::with_capacity((width as usize) * (height as usize) * 3);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&colour(x as f32 + 0.5, y as f32 + 0.5, w, h));
            }
        }
        Frame::new(data, width, height, 1_000_000, PixelFormat::Rgb24, 1)
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "Bounded request parsing and fixed-format bodies of a loopback-only test server"
)]
pub mod readme_web {
    //! Web app fixtures for the README captures of the phone companion (GitHub #381, spec
    //! `AI/architect_spec_readme_capture.md` §3.4, §3.5 and §10): a pure route table with
    //! canned, identity-free JSON in the shapes of the `soos-remote` views, a loopback-only
    //! server, and a PNG metadata stripper for browser screenshots. Never part of
    //! `soos-remote`.

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Placeholder host name used in documentation of the captures (never served).
    pub const DEMO_HOST: &str = "soos-demo.example";
    /// Largest request head read.
    pub const MAX_FIXTURE_REQUEST_HEAD_BYTES: usize = 8192;
    /// How long an event stream stays open after its prelude.
    pub const FIXTURE_SSE_HOLD_MS: u64 = 10_000;
    /// Most live connections; extra ones are closed at once.
    pub const MAX_FIXTURE_CONNECTIONS: usize = 16;
    /// Read and write timeout of a connection.
    pub const FIXTURE_IO_TIMEOUT: Duration = Duration::from_secs(2);

    /// The files of `crates/remote/assets` (`/` serves `index.html`).
    pub const ASSET_NAMES: [&str; 11] = [
        "app.js",
        "apple-touch-icon.png",
        "badge-96.png",
        "icon-192.png",
        "icon-512.png",
        "icon-maskable-512.png",
        "icon.svg",
        "index.html",
        "manifest.webmanifest",
        "style.css",
        "sw.js",
    ];

    /// A canned state of the phone companion.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WebScene {
        /// Unlocked, active session, no alert, charging.
        Unlocked,
        /// Locked and idle, two failed `sudo` passwords, on battery.
        LockedWithAlerts,
    }

    impl WebScene {
        /// Every scene.
        pub const ALL: [WebScene; 2] = [WebScene::Unlocked, WebScene::LockedWithAlerts];

        /// Command-line and file name of the scene.
        pub fn name(self) -> &'static str {
            match self {
                WebScene::Unlocked => "unlocked",
                WebScene::LockedWithAlerts => "locked-alerts",
            }
        }

        /// Parses [`WebScene::name`].
        pub fn parse(name: &str) -> Option<Self> {
            Self::ALL.into_iter().find(|scene| scene.name() == name)
        }
    }

    /// What the fixture server answers.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FixtureResponse {
        /// A file of [`ASSET_NAMES`].
        Asset(&'static str),
        /// A JSON body.
        Json {
            /// HTTP status.
            status: u16,
            /// Body.
            body: String,
        },
        /// A complete server-sent events prelude (status, alerts and battery events).
        EventStream(String),
        /// Anything else.
        NotFound,
    }

    fn status_body(scene: WebScene, now_unix_ms: u64) -> String {
        match scene {
            WebScene::Unlocked => format!(
                "{{\"state\":\"unlocked\",\"active\":true,\"idle\":false,\
                 \"idle_since_unix_s\":null,\"checked_unix_ms\":{now_unix_ms}}}"
            ),
            WebScene::LockedWithAlerts => format!(
                "{{\"state\":\"locked\",\"active\":true,\"idle\":true,\
                 \"idle_since_unix_s\":{},\"checked_unix_ms\":{now_unix_ms}}}",
                (now_unix_ms / 1000).saturating_sub(720)
            ),
        }
    }

    fn alerts_body(scene: WebScene, now_unix_ms: u64) -> String {
        match scene {
            WebScene::Unlocked => "{\"state\":\"active\",\"reason\":null,\"epoch\":null,\
                 \"lock_screen\":\"monitored\",\"unacknowledged_wrong_password\":0,\
                 \"unacknowledged_locked_out\":0,\"last_unix_ms\":null,\"last_source\":null,\
                 \"through\":0,\"history\":[]}"
                .to_string(),
            WebScene::LockedWithAlerts => {
                let last = now_unix_ms.saturating_sub(180_000);
                let first = now_unix_ms.saturating_sub(240_000);
                format!(
                    "{{\"state\":\"active\",\"reason\":null,\"epoch\":\"demo-epoch\",\
                     \"lock_screen\":\"monitored\",\"unacknowledged_wrong_password\":2,\
                     \"unacknowledged_locked_out\":0,\"last_unix_ms\":{last},\
                     \"last_source\":\"sudo\",\"through\":2,\"history\":[{{\"id\":1,\
                     \"first_unix_ms\":{first},\"last_unix_ms\":{last},\"source\":\"sudo\",\
                     \"account\":\"owner\",\"kind\":\"wrong_password\",\"count\":2}}]}}"
                )
            }
        }
    }

    fn battery_body(scene: WebScene) -> &'static str {
        match scene {
            WebScene::Unlocked => {
                "{\"state\":\"present\",\"percent\":82,\"charge\":\"charging\",\"external_power\":true}"
            }
            WebScene::LockedWithAlerts => {
                "{\"state\":\"present\",\"percent\":64,\"charge\":\"discharging\",\"external_power\":false}"
            }
        }
    }

    /// One `OfflineInterval` (`crates/remote/src/offline.rs`) that ended `ended_ago_ms` before
    /// `now_unix_ms` and lasted `lasted_ms` (saturating: `now = 0` gives zeros).
    fn offline_interval(
        reason: &str,
        now_unix_ms: u64,
        ended_ago_ms: u64,
        lasted_ms: u64,
    ) -> String {
        let end = now_unix_ms.saturating_sub(ended_ago_ms);
        let start = end.saturating_sub(lasted_ms);
        format!(
            "{{\"reason\":\"{reason}\",\"start_unix_ms\":{start},\"end_unix_ms\":{end},\
             \"duration_ms\":{}}}",
            end.saturating_sub(start)
        )
    }

    /// `GET /api/offline` (`OfflineView`): recording, NetworkManager readable, synthetic
    /// intervals newest first.
    fn offline_body(scene: WebScene, now_unix_ms: u64) -> String {
        const MINUTE: u64 = 60_000;
        const HOUR: u64 = 60 * MINUTE;
        let intervals = match scene {
            WebScene::Unlocked => offline_interval("sleep", now_unix_ms, HOUR, 8 * HOUR),
            WebScene::LockedWithAlerts => format!(
                "{},{}",
                offline_interval("network_lost", now_unix_ms, 30 * MINUTE, 12 * MINUTE),
                offline_interval("shutdown_or_crash", now_unix_ms, 24 * HOUR, 40 * MINUTE)
            ),
        };
        format!(
            "{{\"state\":\"active\",\"reason\":null,\"network_monitor\":\"active\",\
             \"intervals\":[{intervals}]}}"
        )
    }

    const AUTH_STATE_BODY: &str = "{\"mode\":\"tailnet\",\"authenticated\":true,\"passkeys\":true,\
         \"unlock_enabled\":true,\"enrollment\":false}";
    const PUSH_BODY: &str = "{\"state\":\"disabled\",\"reason\":null,\"public_key\":null,\
         \"subscriptions\":0,\"devices\":[],\"last_delivery\":null,\"sender\":\"unknown\"}";
    const CAMERA_BODY: &str = "{\"enabled\":true,\"reachable\":true,\"state\":\"idle\",\
         \"max_view_s\":120,\"fps\":5,\"width\":640}";

    fn json(body: String) -> FixtureResponse {
        FixtureResponse::Json { status: 200, body }
    }

    /// The fixture answer to `method path` (total: never panics). Only `GET` of an exact
    /// asset name or API path is served; a query string, a traversal or any other path is
    /// [`FixtureResponse::NotFound`].
    pub fn route(scene: WebScene, method: &str, path: &str, now_unix_ms: u64) -> FixtureResponse {
        if method != "GET" {
            return FixtureResponse::NotFound;
        }
        match path {
            "/" => FixtureResponse::Asset("index.html"),
            "/api/auth/state" => json(AUTH_STATE_BODY.to_string()),
            "/api/status" => json(status_body(scene, now_unix_ms)),
            "/api/alerts" => json(alerts_body(scene, now_unix_ms)),
            "/api/push" => json(PUSH_BODY.to_string()),
            "/api/camera" => json(CAMERA_BODY.to_string()),
            "/api/offline" => json(offline_body(scene, now_unix_ms)),
            "/api/events" => FixtureResponse::EventStream(format!(
                "event: status\ndata: {}\n\nevent: alerts\ndata: {}\n\nevent: battery\ndata: {}\n\n",
                status_body(scene, now_unix_ms),
                alerts_body(scene, now_unix_ms),
                battery_body(scene)
            )),
            other => other
                .strip_prefix('/')
                .and_then(|name| ASSET_NAMES.iter().find(|asset| **asset == name))
                .map_or(FixtureResponse::NotFound, |asset| FixtureResponse::Asset(asset)),
        }
    }

    fn content_type(name: &str) -> &'static str {
        match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("png") => "image/png",
            Some("svg") => "image/svg+xml",
            Some("webmanifest") => "application/manifest+json",
            _ => "application/octet-stream",
        }
    }

    fn now_unix_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    fn write_response(
        stream: &mut TcpStream,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
             Cache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        stream.flush()
    }

    /// Reads the request head (bounded) and returns `(method, path)`.
    fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
        let mut head = Vec::with_capacity(512);
        let mut buf = [0u8; 512];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                return None;
            }
            head.extend_from_slice(buf.get(..n)?);
            if head.len() > MAX_FIXTURE_REQUEST_HEAD_BYTES {
                return None;
            }
        }
        let text = String::from_utf8_lossy(&head);
        let mut parts = text.lines().next()?.split(' ');
        Some((parts.next()?.to_string(), parts.next()?.to_string()))
    }

    fn handle(mut stream: TcpStream, scene: WebScene, assets: &Path, deadline: Instant) {
        let _ = stream.set_read_timeout(Some(FIXTURE_IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(FIXTURE_IO_TIMEOUT));
        let Some((method, path)) = read_request_line(&mut stream) else {
            return;
        };
        let result = match route(scene, &method, &path, now_unix_ms()) {
            FixtureResponse::Asset(name) => match std::fs::read(assets.join(name)) {
                Ok(bytes) => write_response(&mut stream, "200 OK", content_type(name), &bytes),
                Err(_) => write_response(&mut stream, "404 Not Found", "text/plain", b"not found"),
            },
            FixtureResponse::Json { body, .. } => {
                write_response(&mut stream, "200 OK", "application/json", body.as_bytes())
            }
            FixtureResponse::EventStream(prelude) => {
                let sent = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                     Cache-Control: no-store\r\nConnection: close\r\n\r\n{prelude}"
                )
                .and_then(|()| stream.flush());
                // Hold the stream open (bounded) so the page keeps its live view.
                let hold = Duration::from_millis(FIXTURE_SSE_HOLD_MS)
                    .min(deadline.saturating_duration_since(Instant::now()));
                std::thread::sleep(hold);
                sent
            }
            FixtureResponse::NotFound => {
                write_response(&mut stream, "404 Not Found", "text/plain", b"not found")
            }
        };
        let _ = result;
    }

    /// Serves `scene` on an already bound (loopback) `listener` for `serve_for`, reading asset
    /// bytes only from `assets/<allow-listed name>`; at most [`MAX_FIXTURE_CONNECTIONS`]
    /// connections live at once.
    ///
    /// # Errors
    ///
    /// An I/O error of the listener itself.
    pub fn serve(
        listener: TcpListener,
        scene: WebScene,
        assets: &Path,
        serve_for: Duration,
    ) -> std::io::Result<()> {
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + serve_for;
        let live = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    if live.load(Ordering::Acquire) >= MAX_FIXTURE_CONNECTIONS {
                        drop(stream);
                        continue;
                    }
                    stream.set_nonblocking(false)?;
                    live.fetch_add(1, Ordering::AcqRel);
                    let live_worker = Arc::clone(&live);
                    let assets = assets.to_path_buf();
                    workers.push(std::thread::spawn(move || {
                        handle(stream, scene, &assets, deadline);
                        live_worker.fetch_sub(1, Ordering::AcqRel);
                    }));
                    workers.retain(|worker| !worker.is_finished());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(e),
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
        Ok(())
    }

    /// Chunks a stripped PNG keeps (image data only).
    const KEPT_PNG_CHUNKS: [&[u8; 4]; 5] = [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"];

    /// Removes every ancillary chunk (text, EXIF, time, ICC, physical size…) of a PNG, copying
    /// the kept chunks verbatim. Refuses an animated PNG (`acTL`), a truncated file or a file
    /// without `IEND`.
    ///
    /// # Errors
    ///
    /// A message naming the defect.
    pub fn strip_png_metadata(bytes: &[u8]) -> Result<Vec<u8>, String> {
        const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        if bytes.get(..8) != Some(&SIGNATURE[..]) {
            return Err("not a PNG file".to_string());
        }
        let mut out = SIGNATURE.to_vec();
        let mut offset = 8usize;
        loop {
            let header = bytes
                .get(offset..offset.saturating_add(8))
                .ok_or("truncated PNG chunk header")?;
            let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
            let kind = [header[4], header[5], header[6], header[7]];
            let end = offset
                .checked_add(12)
                .and_then(|v| v.checked_add(len))
                .ok_or("PNG chunk length overflow")?;
            let chunk = bytes.get(offset..end).ok_or("truncated PNG chunk")?;
            if &kind == b"acTL" {
                return Err("animated PNG refused".to_string());
            }
            if KEPT_PNG_CHUNKS.iter().any(|kept| **kept == kind) {
                out.extend_from_slice(chunk);
            }
            offset = end;
            if &kind == b"IEND" {
                return Ok(out);
            }
        }
    }
}

/// Synthetic, non-biometric near-infrared frame sequences for the strobe classifier of
/// `soos-camera-v4l` (GitHub #378, PR 1 of #353).
///
/// Every frame is `Grey`, stamped `SensorType::Infrared`, and draws a flat bright disc (the
/// "lit scene") over a dim background with a small deterministic texture. Nothing here is
/// derived from a face image.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "Synthetic fixture pixel generation over bounded coordinates"
)]
pub mod ir_strobe {
    use soos_frame::{Frame, PixelFormat, SensorType};

    /// Mean disc level of a lit frame.
    pub const LIT_DISC: u8 = 160;
    /// Background level of a lit frame.
    pub const LIT_BACKGROUND: u8 = 70;
    /// Disc level of an unlit frame (ambient only).
    pub const UNLIT_DISC: u8 = 14;
    /// Background level of an unlit frame.
    pub const UNLIT_BACKGROUND: u8 = 10;

    /// One frame of a sequence and the emitter state commanded for it (`None`: no command).
    pub struct IrSample {
        /// The synthetic frame.
        pub frame: Frame,
        /// `Some(true)` = emitter commanded on, `Some(false)` = off, `None` = not commanded.
        pub commanded_on: Option<bool>,
    }

    /// One synthetic IR frame, lit or unlit, `Grey` and `Infrared`.
    pub fn ir_frame(width: u32, height: u32, sequence: u64, lit: bool) -> Frame {
        let (disc, background) = if lit {
            (LIT_DISC, LIT_BACKGROUND)
        } else {
            (UNLIT_DISC, UNLIT_BACKGROUND)
        };
        let w = width as i64;
        let h = height as i64;
        let (cx, cy) = (w / 2, h / 2);
        let radius = (w.min(h) / 3).max(1);
        let mut data = Vec::with_capacity((width as usize).saturating_mul(height as usize));
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x - cx, y - cy);
                let base = if dx * dx + dy * dy <= radius * radius {
                    disc
                } else {
                    background
                };
                // +/-2 levels of texture so the frame is not perfectly flat.
                let texture = ((x * 7 + y * 13) % 5) as u8;
                data.push(base.saturating_add(texture).saturating_sub(2));
            }
        }
        Frame::new(
            data,
            width,
            height,
            1_000_000u64.saturating_add(sequence.saturating_mul(33_333_333)),
            PixelFormat::Grey,
            sequence,
        )
        .with_sensor_type(SensorType::Infrared)
    }

    fn sequence(
        n: usize,
        lit: impl Fn(usize) -> bool,
        cmd: impl Fn(usize) -> Option<bool>,
    ) -> Vec<IrSample> {
        (0..n)
            .map(|i| IrSample {
                frame: ir_frame(64, 48, i as u64, lit(i)),
                commanded_on: cmd(i),
            })
            .collect()
    }

    /// `n` lit frames, no command (emitter permanently on).
    pub fn steady_sequence(n: usize) -> Vec<IrSample> {
        sequence(n, |_| true, |_| None)
    }

    /// `n` frames alternating lit / unlit by sequence parity on their own (even = `first_lit`).
    pub fn native_alternating_sequence(n: usize, first_lit: bool) -> Vec<IrSample> {
        sequence(n, |i| (i % 2 == 0) == first_lit, |_| None)
    }

    /// One frame per pattern entry: commanded state = entry, and the scene follows it.
    pub fn controllable_sequence(pattern: &[bool]) -> Vec<IrSample> {
        let p = pattern.to_vec();
        let q = pattern.to_vec();
        sequence(
            pattern.len(),
            move |i| p.get(i).copied().unwrap_or(false),
            move |i| q.get(i).copied(),
        )
    }

    /// One frame per pattern entry: commands are issued but the scene stays lit (a control
    /// that does not drive the emitter).
    pub fn unresponsive_sequence(pattern: &[bool]) -> Vec<IrSample> {
        let q = pattern.to_vec();
        sequence(pattern.len(), |_| true, move |i| q.get(i).copied())
    }

    /// `n` unlit frames (no emitter, dark room).
    pub fn absent_sequence(n: usize) -> Vec<IrSample> {
        sequence(n, |_| false, |_| None)
    }

    /// `steady_len` lit frames, then `alternating_len` frames alternating by parity: a stream
    /// whose class changes mid-request.
    pub fn class_change_sequence(steady_len: usize, alternating_len: usize) -> Vec<IrSample> {
        sequence(
            steady_len + alternating_len,
            move |i| i < steady_len || i % 2 == 0,
            |_| None,
        )
    }
}
