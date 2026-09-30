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
