//! Non-blocking follow-ups from the P2 batch reviews (GitHub #285, matrix rows RFX1–RFX12).
//!
//! - documentation accuracy: IPC facts dropped by a hand-merge, fixture `#[path]` prose,
//!   the protocol v1 compatibility claim of the client message tag, the systemd ordering
//!   comment, the camera classification comment and the Docker toolchain comment;
//! - traceability: the superseded PCZ6 row and the tightened VAY6 copy invariant;
//! - hardening: the zeroized sharpness luma buffer and the mock daemon's unknown-tag parity
//!   with `soos_protocol::message::decode_client_message`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Text of the Markdown section that starts with the heading line `heading`, up to the next
/// heading of any level.
fn markdown_section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("missing section {heading}"));
    let body = &doc[start + heading.len()..];
    let end = body.find("\n#").unwrap_or(body.len());
    &body[..end]
}

/// The verification-matrix row whose first cell is `id`.
fn matrix_row(matrix: &str, id: &str) -> String {
    let prefix = format!("| {id} |");
    matrix
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("matrix row {id} is missing"))
        .to_string()
}

/// Source lines that are not `//` comments (documentation cannot satisfy a code scan).
fn code_lines(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
}

/// Module documentation (`//!` lines) of a Rust source file.
fn module_doc(source: &str) -> String {
    source
        .lines()
        .take_while(|line| line.starts_with("//!") || line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// RFX1: the three facts dropped by the #283 hand-merge are back in `Docs/IPC_PROTOCOL.md`.
#[test]
fn test_rfx_ipc_protocol_restores_dropped_facts() {
    let doc = read("Docs/IPC_PROTOCOL.md");
    let deadline = doc
        .lines()
        .find(|line| line.starts_with("- `deadline_monotonic_ns: u64`"))
        .expect("deadline_monotonic_ns field description");
    assert!(
        deadline.contains("DEFAULT_CONNECTION_TIMEOUT_MS") && deadline.contains("2500 ms"),
        "deadline_monotonic_ns must name DEFAULT_CONNECTION_TIMEOUT_MS = 2500 ms: {deadline}"
    );
    let status = markdown_section(&doc, "### `StatusResponse`");
    assert!(
        status.contains("carries no frame, template, embedding or UID data"),
        "StatusResponse must state that it carries no biometric or UID data: {status}"
    );
    let preview = markdown_section(&doc, "### `PreviewResponse`");
    assert!(
        preview.contains("the struct zeroizes on drop"),
        "PreviewResponse must state that the struct zeroizes on drop: {preview}"
    );
}

/// RFX2: fixture prose matches the code (no `#[path]` include remains, no embedding fixture)
/// and the Docker suite comment names the pinned toolchain instead of `stable`.
#[test]
fn test_rfx_fixture_and_toolchain_prose_matches_code() {
    let mock = read("AI/MOCK_STRATEGY.md");
    assert!(
        !mock.contains("still include `mod.rs` through `#[path]`"),
        "AI/MOCK_STRATEGY.md still describes legacy #[path] fixture includes"
    );
    let fil3 = matrix_row(&read("AI/VERIFICATION_MATRIX.md"), "FIL3");
    assert!(
        !fil3.contains("three frozen legacy files"),
        "matrix FIL3 still describes three frozen #[path] includes: {fil3}"
    );
    let contract = module_doc(&read("tests/invariants/src/fixtures_contract.rs"));
    assert!(
        !contract.contains("the three legacy"),
        "fixtures_contract.rs module doc still describes three legacy includes"
    );
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains("P2 Review Follow-ups (GitHub #285)")
            && decisions.contains("`LEGACY_PATH_INCLUDES` is empty"),
        "AI/DECISIONS.md must record that no legacy #[path] fixture include remains"
    );
    for rel in ["AGENTS.md", "AI/ARCHITECTURE.md"] {
        assert!(
            !read(rel).contains("shared synthetic frames and embeddings"),
            "{rel} still lists embedding fixtures (none exist, embeddings are 512D)"
        );
    }
    let suite = read("tests/docker/test_suite.sh");
    assert!(
        !suite.contains("The image installs `stable`"),
        "tests/docker/test_suite.sh still says the image installs `stable`"
    );
}

/// RFX3: the protocol module doc no longer implies that v1 readers accept tagged frames.
#[test]
fn test_rfx_message_doc_does_not_claim_v1_reader_compatibility() {
    let doc = module_doc(&read("crates/protocol/src/message.rs"));
    assert!(
        !doc.contains("backward compatible extension"),
        "message.rs must not call the tag trailer a backward compatible extension"
    );
    assert!(
        doc.contains("rejects a tagged frame as malformed"),
        "message.rs must state that a pre-#204 daemon rejects tagged frames"
    );
}

/// RFX4: the systemd unit does not claim the socket is bound before the greeter prompt.
#[test]
fn test_rfx_systemd_unit_does_not_overclaim_listen_ordering() {
    let unit = read("packaging/soos-daemon.service");
    assert!(
        !unit.contains("Listen before the greeter shows its first prompt"),
        "Type=simple gives no bind-before-greeter guarantee (models and warm-up run first)"
    );
    assert!(unit.contains("Before=display-manager.service"));
}

/// RFX5: every `.to_vec()` in an ORT run site lands in a wipe-on-drop container; the only
/// exceptions are tensor-shape metadata and the `BiometricEmbedding::to_vec` accessor.
#[test]
fn test_rfx_ort_output_copies_stay_in_zeroizing_containers() {
    const RUN_SITES: [&str; 3] = [
        "crates/inference-ort/src/detector.rs",
        "crates/inference-ort/src/embedding.rs",
        "crates/inference-ort/src/pad.rs",
    ];
    const ALLOWED: [&str; 2] = ["shape", "self.vector"];
    const WRAPPERS: [&str; 2] = ["Zeroizing::new(", "BiometricEmbedding::new("];
    let mut copies = 0usize;
    for rel in RUN_SITES {
        let source = read(rel);
        for line in code_lines(&source) {
            let mut rest = line;
            while let Some(pos) = rest.find(".to_vec()") {
                // The receiver: the identifier path right before `.to_vec()`.
                let before = &rest[..pos];
                let start = before
                    .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                    .map_or(0, |i| i + 1);
                let receiver = &before[start..];
                let prefix = &before[..start];
                let after = &rest[pos + ".to_vec()".len()..];
                rest = after;
                if ALLOWED.contains(&receiver) {
                    continue;
                }
                copies += 1;
                assert!(
                    WRAPPERS.iter().any(|w| prefix.ends_with(w)) && after.starts_with(')'),
                    "{rel}: `{receiver}.to_vec()` must be the direct argument of a zeroizing \
                     container (`{}`)",
                    line.trim()
                );
            }
        }
    }
    assert!(copies >= 2, "the copy scan is vacuous ({copies} copies)");
    let embedding = read("crates/inference-ort/src/embedding.rs");
    assert!(
        embedding.contains("vector: Zeroizing<Vec<f32>>"),
        "BiometricEmbedding must store its vector in Zeroizing"
    );
    let matrix = read("AI/VERIFICATION_MATRIX.md");
    let vay6 = matrix_row(&matrix, "VAY6");
    assert!(
        !vay6.contains("never moved out (`outputs.into_iter()`) or copied (`slice.to_vec()`)"),
        "VAY6 must not claim that ORT outputs are never copied: {vay6}"
    );
}

/// RFX6: PCZ6 (deferred tagged envelope) is superseded by the #204 tag trailer.
#[test]
fn test_rfx_pcz6_row_is_superseded() {
    let pcz6 = matrix_row(&read("AI/VERIFICATION_MATRIX.md"), "PCZ6");
    assert!(
        pcz6.contains("⏹ Superseded"),
        "PCZ6 must be superseded by the #204 trailer rows: {pcz6}"
    );
}

/// RFX7: the capture supervisor comment matches the documented (narrower) hints.
#[test]
fn test_rfx_capture_hint_comment_matches_docs() {
    let source = read("crates/camera-v4l/src/v4l_impl.rs");
    assert!(
        !source.contains("same hints as the resolver"),
        "the supervisor hints are narrower than the resolver's (Docs/CAMERA_V4L_CRATE.md)"
    );
}

/// RFX8: the Layer 1 documentation describes what the awk filter really blanks.
#[test]
fn test_rfx_candid_filter_documentation_matches_script() {
    let script = read("scripts/candid_review.sh");
    for construct in ["block comments", "raw strings"] {
        assert!(
            script.contains(construct),
            "RUST_PRODUCTION_FILTER must document how it handles {construct}"
        );
    }
    let doc = read("Docs/CI_CD_AND_SECURITY.md");
    assert!(
        doc.contains("nested block comments") && doc.contains("raw strings"),
        "Docs/CI_CD_AND_SECURITY.md must describe the multi-line lexing of the awk filter"
    );
}

/// RFX10: the sharpness luma buffer (derived from face pixels) is wiped on drop.
#[test]
fn test_rfx_laplacian_luma_buffer_is_zeroized() {
    let source = read("crates/vision/src/quality.rs");
    let start = source
        .find("pub fn laplacian_variance")
        .expect("laplacian_variance");
    let body = &source[start..];
    let end = body.find("\n}\n").expect("end of laplacian_variance");
    let body = &body[..end];
    assert!(
        body.contains("Zeroizing"),
        "laplacian_variance must hold its luma buffer in zeroize::Zeroizing"
    );
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "soos-rfx-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Postcard v1 `Request` body: Auth, 32-byte id, uid 1000, service "sudo", no deadline.
fn request_body() -> Vec<u8> {
    let mut body = vec![1u8, 0u8];
    body.extend_from_slice(&[0x11; 32]);
    body.extend_from_slice(&[0xE8, 0x07]);
    body.push(4);
    body.extend_from_slice(b"sudo");
    body.push(0);
    body
}

/// Sends one length-prefixed frame and returns every byte the mock sends back before EOF.
fn exchange(sock: &Path, payload: &[u8]) -> Vec<u8> {
    let mut stream = UnixStream::connect(sock).expect("connect mock daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let len = u32::try_from(payload.len()).expect("small payload");
    let mut frame = len.to_be_bytes().to_vec();
    frame.extend_from_slice(payload);
    stream.write_all(&frame).expect("write frame");
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    reply
}

/// RFX12: the mock daemon drops a frame whose trailer is an unknown tag (no handler, no
/// response), like `decode_client_message` (`MessageError::UnknownTag`).
#[test]
fn test_rfx_mock_daemon_drops_unknown_tag_frames() {
    let root = workspace_root();
    let dir = scratch_dir("tag");
    let sock = dir.join("daemon.sock");
    let record = dir.join("record.log");
    let group = String::from_utf8_lossy(
        &Command::new("id")
            .arg("-gn")
            .output()
            .expect("id -gn")
            .stdout,
    )
    .trim()
    .to_string();
    let mut child = Command::new("python3")
        .arg(root.join("tests/docker/mock_daemon.py"))
        .args(["--mode", "allow", "--socket"])
        .arg(&sock)
        .args(["--socket-group", &group, "--record"])
        .arg(&record)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mock_daemon.py");
    let start = Instant::now();
    while !sock.exists() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut unknown = request_body();
    unknown.push(0xFF);
    let unknown_reply = exchange(&sock, &unknown);
    let mut tagged = request_body();
    tagged.push(0xA0);
    let tagged_reply = exchange(&sock, &tagged);

    child.kill().ok();
    child.wait().ok();
    let log = fs::read_to_string(&record).unwrap_or_default();
    fs::remove_dir_all(&dir).ok();

    assert!(
        !tagged_reply.is_empty(),
        "sanity: a 0xA0-tagged request must be answered (mock log: {log})"
    );
    assert!(
        unknown_reply.is_empty(),
        "a frame with an unknown tag byte must get no response ({} bytes)",
        unknown_reply.len()
    );
    assert_eq!(
        log.lines().filter(|l| *l == "request").count(),
        1,
        "only the known-tag frame is handled as a request: {log}"
    );
    assert!(
        log.lines().any(|l| l == "rejected unknown-tag"),
        "the unknown-tag frame must be recorded as rejected: {log}"
    );
}
