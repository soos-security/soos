# Walkthrough 231 — soos-gui Edits the Safe remote.toml Keys Above an [android] Table

- **Date**: 2026-10-18
- **Issue**: GitHub #429, part of #363 / #379 (GitHub-only, no backlog id) — **Branch**: `feat/gui-android-safe-keys`
- **Decision**: owner decision of 2026-10-18 on #429, recorded as an amendment of item (3) of ADR 2026-10-13 "First-Run Onboarding Tour and GUI Feature Switches in `soos-gui`"
- **Matrix criteria**: GAK1–GAK6 (new). OBT9 is unchanged and still holds for every table other than one `[android]` table.

## 1. Context & Objectives

`feature_control::disable_remote_keys` refused any `remote.toml` that contained a line starting with `[` (`RemoteEditError::UnsupportedLayout`). A user who had configured the native Android app (`[android]`, ADR 2026-10-13) therefore could not turn off any companion feature from the `soos-gui` feature tour.

The owner decided that:

- remote unlock, push alerts, offline alerts and the camera view, with their dependent keys, can be turned off when the file has an `[android]` table;
- Internet access (Funnel) stays refused, with a clear and specific message, because `soos-remote` requires `allow_funnel = true` while `[android]` is configured (exit 78);
- any other table, or a target key inside a table, stays refused;
- the edit stays line-preserving, is verified by a TOML parse, and never writes a file `soos-remote` would refuse.

## 2. Architect Design

### Types

- `RemoteEditError::FunnelRequiredByAndroid` (new variant). Its message names Internet access, the `[android]` table and `allow_funnel`, and says what to do: remove `[android]` first, then set `allow_funnel = false`. It carries no file content.
- `RemoteEditError::UnsupportedLayout` keeps its name. Its doc comment and message now describe the real rule (a table other than `[android]`, or the key inside a table) instead of "tables".
- Private helpers `ANDROID_TABLE` and `is_android_header`. Accepted headers are exactly `[android]`, optionally spaced inside the brackets, optionally indented, optionally followed by a `#` comment. A quoted header (`["android"]`), a sub-table (`[android.x]`) and an array of tables (`[[...]]`) are not accepted.

### Edit rules (`disable_remote_keys`)

1. Parse the original text (invalid TOML is `Invalid`, as before).
2. If the parsed file has an `android` key and the switch's keys include `allow_funnel`, return `FunnelRequiredByAndroid` before any edit. This holds even when `allow_funnel` is already off or absent: that file is already refused by `soos-remote`, and a refusal is clearer than a silent no-op.
3. Walk the lines. The first `[android]` header switches to "in table" mode and is copied. Any other header, or any header once in table mode, is `UnsupportedLayout`. A line that is a key of the switch while in table mode is `UnsupportedLayout`, whatever its value. Other lines in the table are copied byte for byte.
4. The existing verification is unchanged: every target key reads `false` or is absent, and every other top-level key is equal to the original. `android` is a top-level key, so this also proves that the table did not change.

### Routing (`onboarding::toggle_route`)

`RemoteKeys` and `FeatureSnapshot` are unchanged. Adding an "android present" field would have changed struct literals inside existing test assertions. So the Funnel refusal is reported by the executor (the outcome message the tour already shows), and the shown "off" command for Internet access (running as root, or keys unknown) gets the `FUNNEL_ANDROID_NOTE` sentence.

### Invariants touched

Off-only edits, the single atomic 0600 write, no symbolic link, one `try-restart` and nothing written or restarted on refusal all hold as before. No `unsafe`, no new panic path (`is_android_header` uses `get` only), no logging, no IPC change, no Cargo change.

### Proof without `soos-remote`'s loader

`soos-remote` must stay a leaf crate (RMC-S4: no other manifest may name it, dev-dependencies included). A first draft added it as a `soos-gui` dev-dependency so the tests could call `parse_config_with_android`; `test_rmc_s4_remote_is_a_leaf_crate` failed, and the dependency was dropped. The tests instead re-parse the edited TOML, compare the `[android]` table with the original, and run a small rule checker (`load` in the test file). The checker mirrors the exit-78 cross-key rules of `soos-remote` for the keys this edit can touch: `allow_funnel` needs `rp_id`, `push_notifications` needs `password_alerts` and `rp_id`, `camera_view` needs `rp_id`, `camera_view_funnel` needs `camera_view` and `allow_funnel`, `offline_alerts` needs `push_notifications`, `[android]` needs `rp_id` and `allow_funnel`, and no key is unknown. GAK1 also proves that the checker rejects the broken variants.

## 3. Tester Contract (Red)

New file `crates/gui/tests/remote_android_edit_tests.rs`, built on a setup-profile fixture (every companion key on) followed by a commented `[android]` table with a multi-line fingerprint array:

- GAK1: the rule checker accepts the fixture with its binding, and rejects Funnel off under `[android]`, a dangling dependent key and an unknown key.
- GAK2: for each safe switch, only the target `key = true` lines change, the `[android]` suffix is byte-identical, the re-parsed `[android]` table equals the original, the checker accepts the result, and `allow_funnel` stays on. An already-off or absent key writes nothing.
- GAK3: Funnel returns `FunnelRequiredByAndroid` (on or off), with the expected message needles; without `[android]`, Funnel is turned off as before.
- GAK4: other tables before or after `[android]`, `[android.extra]`, `[[extra]]`, `["android"]` and a switch key inside `[android]` are `UnsupportedLayout`; the spacing and comment variants of the header are accepted.
- GAK5: the executor edits the file (mode 0600, one restart) and refuses Funnel without writing or restarting.
- GAK6: the shown "off" command for Internet access names the `[android]` requirement; Remote unlock's does not.

The red run failed to compile on the missing `RemoteEditError::FunnelRequiredByAndroid`. The existing `test_obt_disable_refuses_unsupported_layouts` (`[extra]` table) and every other existing test were left untouched.

## 4. Auditor Constraints

- No `unwrap`, `expect`, indexing or unchecked arithmetic in the new production code.
- The refusal must happen before any write and before any `systemctl` call.
- Error messages carry no file content.
- The verification parse must still cover the `[android]` table, so a header recognised by mistake can never change it.

## 5. Implementation (Green)

- `crates/gui/src/feature_control.rs`: the new variant, the header helper, the Funnel pre-check and the table-aware line walk. The doc comments now describe the `[android]` rule.
- `crates/gui/src/onboarding.rs`: `FUNNEL_ANDROID_NOTE` is appended to the Internet access "off" command.

## 6. Documentation

- `Docs/GUI_APPLICATION.md` §1e: the switch row and a paragraph on `remote.toml` with an `[android]` table.
- `Docs/REMOTE_COMPANION.md` §5: the GUI paragraph now lists `offline_alerts` and describes the `[android]` behaviour.
- `AI/DECISIONS.md`: amendment of item (3) of the onboarding tour ADR.
- `AI/VERIFICATION_MATRIX.md`: component `gui-android-safe-keys`, rows GAK1–GAK6.

## 7. Verification

```bash
LIBONNXRUNTIME_NO_PKG_CONFIG=1 cargo test -p soos-gui -p soos-invariants -- --test-threads=3
cargo clippy -p soos-gui --all-targets -- -D warnings
cargo fmt --all -- --check
```
