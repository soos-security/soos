//! Daemon documentation contract (GitHub #201 / DMN-12, #205 / DMN-16, #206 / DMN-17).
//!
//! The code is the truth; these checks keep the operator documentation aligned with it:
//! every `daemon.toml` key parsed by `crates/daemon/src/config.rs` is documented in
//! `Docs/DAEMON.md`, the IPC specification covers every v1 message kind and the persistent
//! connection loop, and the memory-protection document only presents protections that are
//! wired in production.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("Unable to locate workspace root directory")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
}

/// TOML table served by each private `*ConfigFile` struct of `crates/daemon/src/config.rs`.
/// An empty table name is the document root.
const CONFIG_FILE_TABLES: [(&str, &str); 9] = [
    ("DaemonConfigFile", ""),
    ("SocketConfigFile", "socket"),
    ("DispatcherConfigFile", "dispatcher"),
    ("PipelineConfigFile", "pipeline"),
    ("EvidenceConfigFile", "pipeline.evidence"),
    ("ThresholdConfigFile", "pipeline.thresholds"),
    ("RateLimitConfigFile", "pipeline.rate_limit"),
    ("PreviewConfigFile", "preview"),
    ("PeerLimitsConfigFile", "peer_limits"),
];

/// Returns `(struct name, [(field, type)])` for every `struct *ConfigFile` in `source`.
fn parse_config_file_structs(source: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut structs = Vec::new();
    let mut current: Option<(String, Vec<(String, String)>)> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some((name, fields)) = current.as_mut() {
            if trimmed == "}" {
                structs.push((name.clone(), std::mem::take(fields)));
                current = None;
                continue;
            }
            if trimmed.starts_with("#[") || trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }
            if let Some((field, ty)) = trimmed.trim_end_matches(',').split_once(':') {
                fields.push((field.trim().to_string(), ty.trim().to_string()));
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("struct ") {
            if let Some(name) = rest.strip_suffix(" {") {
                if name.ends_with("ConfigFile") {
                    current = Some((name.to_string(), Vec::new()));
                }
            }
        }
    }
    structs
}

#[test]
fn test_daemon_doc_documents_every_daemon_toml_key() {
    let source = read("crates/daemon/src/config.rs");
    let doc = read("Docs/DAEMON.md");
    let structs = parse_config_file_structs(&source);
    assert!(
        structs.len() >= CONFIG_FILE_TABLES.len(),
        "Expected at least {} *ConfigFile structs in crates/daemon/src/config.rs, found {}",
        CONFIG_FILE_TABLES.len(),
        structs.len()
    );

    let mut missing = Vec::new();
    for (name, fields) in &structs {
        let table = CONFIG_FILE_TABLES
            .iter()
            .find(|(s, _)| s == name)
            .map(|(_, t)| *t)
            .unwrap_or_else(|| {
                panic!(
                    "New daemon.toml struct `{name}`: map it in CONFIG_FILE_TABLES and document it in Docs/DAEMON.md"
                )
            });
        if !table.is_empty() && !doc.contains(&format!("[{table}]")) {
            missing.push(format!("table [{table}]"));
        }
        for (field, ty) in fields {
            if ty.contains("ConfigFile") {
                continue; // sub-table, checked through its own struct
            }
            let key = format!("`{field}`");
            if !doc.contains(&key) {
                let location = if table.is_empty() { "<root>" } else { table };
                missing.push(format!("{location}.{field}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "Docs/DAEMON.md must document every daemon.toml key; missing: {missing:?}"
    );
}

#[test]
fn test_ipc_protocol_documents_v1_kinds_and_persistent_loop() {
    let doc = read("Docs/IPC_PROTOCOL.md");
    for needle in [
        "RequestKind::Status",
        "RequestKind::PreviewFrame",
        "`StatusResponse`",
        "`PreviewResponse`",
        "MAX_PREVIEW_MESSAGE_SIZE",
        "Persistent connection loop",
        "Docs/DAEMON.md",
    ] {
        assert!(
            doc.contains(needle),
            "Docs/IPC_PROTOCOL.md must document `{needle}` (GitHub #206, DMN-17)"
        );
    }
}

/// Returns true when a production source file outside `mlock.rs` references `needle`.
fn used_in_production(needle: &str) -> bool {
    let crates = workspace_root().join("crates");
    let mut stack = vec![crates];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name != "tests" && name != "target" && name != "benches" {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && path.components().any(|c| c.as_os_str() == "src")
                && path.file_name().and_then(|n| n.to_str()) != Some("mlock.rs")
            {
                let content = fs::read_to_string(&path).unwrap_or_default();
                if content.contains(needle) {
                    return true;
                }
            }
        }
    }
    false
}

#[test]
fn test_memory_protection_doc_only_claims_wired_layers() {
    let doc = read("Docs/MEMORY_PROTECTION_AND_SWAP.md");
    let main_rs = read("crates/daemon/src/main.rs");

    assert!(
        !main_rs.contains("granular buffer protection"),
        "soos-daemon must not claim a granular buffer protection layer on mlockall failure"
    );

    if !used_in_production("LockedBuffer") {
        assert!(
            !doc.contains("LockedBuffer::new(master_key_bytes)"),
            "Docs/MEMORY_PROTECTION_AND_SWAP.md shows LockedBuffer guarding the master key, but no production code uses LockedBuffer"
        );
        assert!(
            !doc.contains("Layer 2: Granular Buffer Locking"),
            "Docs/MEMORY_PROTECTION_AND_SWAP.md presents LockedBuffer as an active layer, but it is not wired in production"
        );
        assert!(
            doc.contains("not wired"),
            "Docs/MEMORY_PROTECTION_AND_SWAP.md must state that LockedBuffer is not wired in production"
        );
    }
    assert!(
        doc.contains("warn"),
        "Docs/MEMORY_PROTECTION_AND_SWAP.md must state that an mlockall refusal is logged at warn level"
    );
    assert!(
        doc.contains("memory_locked"),
        "Docs/MEMORY_PROTECTION_AND_SWAP.md must name the HealthState memory_locked flag"
    );
}

#[test]
fn test_daemon_doc_states_the_single_warmup_default() {
    let doc = read("Docs/DAEMON.md");
    let camera_doc = read("Docs/CAMERA_V4L_CRATE.md");
    assert!(
        doc.contains("DAEMON_DEFAULT_WARMUP_FRAMES"),
        "Docs/DAEMON.md must name the daemon warmup default constant"
    );
    assert!(
        camera_doc.contains("DAEMON_DEFAULT_WARMUP_FRAMES"),
        "Docs/CAMERA_V4L_CRATE.md must say that soos-daemon overrides the crate warmup default"
    );
}

/// Returns the body of the Markdown section that starts with `heading` (up to the next
/// heading of the same or a higher level).
fn markdown_section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("missing section heading `{heading}`"));
    let level = heading.chars().take_while(|c| *c == '#').count();
    let body = &doc[start + heading.len()..];
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes <= level && line[hashes..].starts_with(' ') {
            return &body[..offset];
        }
        offset += line.len();
    }
    body
}

/// Numbers of the top-level ordered-list items (`N. ` at column 0) of `text`.
fn top_level_list_numbers(text: &str) -> Vec<u32> {
    text.lines()
        .filter_map(|line| {
            let (number, rest) = line.split_once(". ")?;
            if rest.is_empty() || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            number.parse().ok()
        })
        .collect()
}

/// GitHub #315 (DMN-NEW-5, matrix DRM): the start-up sequence of `Docs/DAEMON.md` §4 is one
/// numbered list (no duplicate step), includes the inference warm-up and ends with the bind
/// + `READY=1`; no stray start-up step is left in §5.
#[test]
fn test_daemon_doc_startup_steps_are_sequential_and_include_warmup() {
    let doc = read("Docs/DAEMON.md");
    let startup = markdown_section(&doc, "## 4. Startup and Swap Protection");
    let numbers = top_level_list_numbers(startup);
    let expected: Vec<u32> = (1..=u32::try_from(numbers.len()).unwrap()).collect();
    assert_eq!(
        numbers, expected,
        "Docs/DAEMON.md §4 start-up steps must be numbered 1..N without duplicates"
    );
    assert!(
        numbers.len() >= 5,
        "§4 must list configuration, swap protection, pipeline, warm-up and bind"
    );
    assert!(
        startup.contains("warmed_inference_gate") && startup.contains("WARMUP_PASSES"),
        "§4 must document the inference warm-up step (warmed_inference_gate, WARMUP_PASSES)"
    );
    let warmup = startup.find("warmed_inference_gate").unwrap();
    let bind = startup
        .find("READY=1")
        .expect("§4 must document READY=1 after the bind");
    assert!(warmup < bind, "the warm-up runs before the socket is bound");
    let shutdown = markdown_section(&doc, "## 5. Shutdown, Panic Reporting");
    assert!(
        top_level_list_numbers(shutdown).is_empty(),
        "no numbered start-up step may be left in §5"
    );
}

/// GitHub #315 (DMN-NEW-5): `AI/ARCHITECTURE.md` describes what the code does.
#[test]
fn test_architecture_doc_matches_daemon_code_claims() {
    let arch = read("AI/ARCHITECTURE.md");
    let dispatcher = read("crates/daemon/src/dispatcher.rs");
    let socket = read("crates/daemon/src/socket.rs");
    let session = read("crates/daemon/src/session.rs");

    // Evidence is sealed for PasswordFailed and for PAD-vetoed Auth requests.
    assert!(
        dispatcher.contains("capture_spoof_evidence"),
        "precondition: the dispatcher seals PAD evidence"
    );
    assert!(
        !arch.contains("only following PasswordFailed"),
        "AI/ARCHITECTURE.md must not say evidence is written only after PasswordFailed"
    );
    let evidence_line = arch
        .lines()
        .find(|l| l.contains("EvidenceStore ("))
        .expect("the architecture diagram names the EvidenceStore");
    assert!(
        evidence_line.contains("PasswordFailed") && evidence_line.contains("PadFailed"),
        "the EvidenceStore line must name both reasons: {evidence_line}"
    );

    // Invariant 3 must not claim an /etc/passwd cross-check the daemon does not perform.
    let invariant3 = arch
        .lines()
        .find(|l| l.starts_with("3. The daemon never trusts"))
        .expect("Invariant 3 exists");
    let daemon_sources = [dispatcher.as_str(), socket.as_str(), session.as_str()];
    let reads_passwd = daemon_sources
        .iter()
        .any(|s| s.contains("getpwuid") || s.contains("User::from_uid"));
    assert!(
        reads_passwd || !invariant3.contains("/etc/passwd"),
        "Invariant 3 claims an /etc/passwd cross-check that the daemon does not perform"
    );
    assert!(
        !session.contains("/etc/passwd"),
        "crates/daemon/src/session.rs must not claim an /etc/passwd cross-check"
    );

    // The /run/soos group claim is backed by a check in socket.rs.
    if arch.contains("is owned by `root:soos`") {
        assert!(
            socket.contains("pub fn validate_directory_group("),
            "AI/ARCHITECTURE.md claims a root:soos check of /run/soos that socket.rs does not do"
        );
    }
}

/// GitHub #315 suggestion: `sd_notify` must not claim that an abstract `NOTIFY_SOCKET` is
/// reachable inside `PrivateNetwork=yes` (abstract sockets are network-namespace scoped).
#[test]
fn test_sd_notify_doc_does_not_overclaim_private_network_reachability() {
    let source = read("crates/daemon/src/sd_notify.rs");
    let header: String = source
        .lines()
        .take_while(|l| l.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !header.contains("reachable inside `PrivateNetwork=yes`)"),
        "the sd_notify module doc over-claims PrivateNetwork reachability"
    );
    assert!(
        header.contains("abstract") && header.contains("network namespace"),
        "the sd_notify module doc must say an abstract address is network-namespace scoped"
    );
}
