//! Contract tests of GitHub #346 for the battery reader of `soos-remote` (ADR 2026-10-07
//! "Live Battery Level in `soos-remote`", architect spec `AI/architect_spec_remote_battery.md`
//! §3, §5.1–§5.4, §10.1 tests 1–17; matrix RBS2–RBS7).
//!
//! Pure parsers, the pure aggregation and the blocking sysfs reader over a [`FakeSysfs`]
//! tempdir root. Nothing reads the real `/sys`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/battery.rs"]
mod battery;

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use proptest::prelude::*;

use battery::*;
use soos_remote::battery::{
    aggregate, parse_capacity, parse_micro, parse_online, parse_present, parse_scope_included,
    parse_status, read_power_supplies, valid_supply_name, BatteryReading, BatterySource,
    BatteryState, BatteryView, ChargeStatus, EnergyPair, SysfsBattery,
};
use soos_remote::{
    MAX_BATTERIES, MAX_POWER_SUPPLIES, MAX_POWER_SUPPLY_NAME_LEN, MAX_SYSFS_MICRO_DIGITS,
    MAX_SYSFS_VALUE_BYTES, POWER_SUPPLY_ROOT,
};

fn view(
    state: BatteryState,
    percent: Option<u8>,
    charge: Option<ChargeStatus>,
    external_power: Option<bool>,
) -> BatteryView {
    BatteryView {
        state,
        percent,
        charge,
        external_power,
    }
}

fn reading(
    capacity: Option<u8>,
    status: ChargeStatus,
    energy: Option<EnergyPair>,
) -> BatteryReading {
    BatteryReading {
        capacity,
        status,
        energy,
    }
}

const DISCHARGING: &[u8] = b"Discharging\n";

// ---------------------------------------------------------------------------------------
// Tests 1–3: nominal readings
// ---------------------------------------------------------------------------------------

/// Test 1 (RBS2, B-9, F-3 a): one discharging battery on a laptop whose mains is offline;
/// a single battery always reports its firmware `capacity`, never a ratio of its energy or
/// charge pair.
#[test]
fn test_rbs_single_battery_discharging_on_battery() {
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"82\n"), ("status", DISCHARGING)]);
    sysfs.mains("ADP0", "0");
    let expected = view(
        BatteryState::Present,
        Some(82),
        Some(ChargeStatus::Discharging),
        Some(false),
    );
    assert_eq!(sysfs.read(), expected);
    assert_eq!(read_power_supplies(&sysfs.root()), expected);

    // An energy pair whose ratio (50 %) disagrees with the firmware capacity.
    sysfs.set("BAT0", "energy_now", b"50000000\n");
    sysfs.set("BAT0", "energy_full", b"100000000\n");
    assert_eq!(
        sysfs.read(),
        expected,
        "single battery: capacity, not energy"
    );

    // A charge pair (30 %) likewise.
    sysfs.remove("BAT0", "energy_now");
    sysfs.remove("BAT0", "energy_full");
    sysfs.set("BAT0", "charge_now", b"30\n");
    sysfs.set("BAT0", "charge_full", b"100\n");
    assert_eq!(
        sysfs.read(),
        expected,
        "single battery: capacity, not charge"
    );
}

/// Test 2 (RBS2): charging on mains.
#[test]
fn test_rbs_single_battery_charging_on_mains() {
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"41\n"), ("status", b"Charging\n")]);
    sysfs.mains("AC", "1");
    assert_eq!(
        sysfs.read(),
        view(
            BatteryState::Present,
            Some(41),
            Some(ChargeStatus::Charging),
            Some(true)
        )
    );
}

/// Test 3 (RBS3, B-10): a desktop: an empty class directory, or only a mains supply, is
/// `no_battery` with every optional field `None`.
#[test]
fn test_rbs_no_battery_desktop() {
    let sysfs = FakeSysfs::new();
    assert_eq!(sysfs.read(), BatteryView::no_battery());
    sysfs.mains("AC", "1");
    let v = sysfs.read();
    assert_eq!(v, BatteryView::no_battery());
    assert_eq!(v.state, BatteryState::NoBattery);
    assert_eq!((v.percent, v.charge, v.external_power), (None, None, None));
}

// ---------------------------------------------------------------------------------------
// Tests 4–6: fail closed
// ---------------------------------------------------------------------------------------

/// Test 4 (RBS4, B-10): a missing root, a root that is a file and an unreadable root are
/// `unavailable`, never `no_battery`.
#[test]
fn test_rbs_unreadable_root_is_unavailable() {
    let sysfs = FakeSysfs::new();
    let missing = sysfs.root().join("does-not-exist");
    assert_eq!(read_power_supplies(&missing), BatteryView::unavailable());
    assert_eq!(
        SysfsBattery::new(missing).read(),
        BatteryView::unavailable()
    );

    let file_dir = tempfile::tempdir().unwrap();
    let file = file_dir.path().join("power_supply");
    std::fs::write(&file, b"not a directory\n").unwrap();
    assert_eq!(read_power_supplies(&file), BatteryView::unavailable());

    // Mode 000 (root can read anything: the last check is skipped when running as root).
    if !nix::unistd::geteuid().is_root() {
        sysfs.battery("BAT0", &[("capacity", b"82\n")]);
        sysfs.chmod_root(0o000);
        assert_eq!(
            read_power_supplies(&sysfs.root()),
            BatteryView::unavailable()
        );
        sysfs.chmod_root(0o700);
    }

    let unavailable = BatteryView::unavailable();
    assert_eq!(unavailable.state, BatteryState::Unavailable);
    assert_eq!(
        (
            unavailable.percent,
            unavailable.charge,
            unavailable.external_power
        ),
        (None, None, None)
    );
    assert_eq!(POWER_SUPPLY_ROOT, "/sys/class/power_supply");
}

/// Test 5 (RBS4, §5.2): a malformed `capacity` is unknown (`percent: None`) while the
/// battery stays `present`; the canonical decimals 0..=100 are accepted.
#[test]
fn test_rbs_malformed_capacity_is_unknown() {
    assert_eq!(MAX_SYSFS_VALUE_BYTES, 32);
    let oversize = vec![b'1'; MAX_SYSFS_VALUE_BYTES + 1];
    let malformed: Vec<&[u8]> = vec![
        b"",
        b"\n",
        b"abc",
        b"101",
        b"-1",
        b"82.5",
        b" 82",
        b"082",
        b"0x50",
        b"1000",
        &oversize,
        b"8\x002",
        &[0xff],
        b"82\n\n",
    ];
    for raw in malformed {
        assert_eq!(parse_capacity(raw), None, "parse_capacity({raw:?})");
        let sysfs = FakeSysfs::new();
        sysfs.battery("BAT0", &[("capacity", raw), ("status", DISCHARGING)]);
        let v = sysfs.read();
        assert_eq!(v.state, BatteryState::Present, "{raw:?}");
        assert_eq!(v.percent, None, "{raw:?}");
        assert_eq!(v.charge, Some(ChargeStatus::Discharging), "{raw:?}");
    }
    for (raw, expected) in [(&b"0"[..], 0u8), (b"100\n", 100), (b"7", 7), (b"82\n", 82)] {
        assert_eq!(
            parse_capacity(raw),
            Some(expected),
            "parse_capacity({raw:?})"
        );
        let sysfs = FakeSysfs::new();
        sysfs.battery("BAT0", &[("capacity", raw), ("status", DISCHARGING)]);
        assert_eq!(sysfs.read().percent, Some(expected), "{raw:?}");
    }
    // A missing capacity is unknown too.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("status", DISCHARGING)]);
    let v = sysfs.read();
    assert_eq!((v.state, v.percent), (BatteryState::Present, None));
}

/// Test 6 (RBS4, §5.2): the five kernel status strings map exactly; anything else (case,
/// trailing space, unknown word, oversize, missing file) is `Unknown`.
#[test]
fn test_rbs_status_strings_are_exact() {
    for (raw, expected) in [
        (&b"Charging"[..], ChargeStatus::Charging),
        (b"Charging\n", ChargeStatus::Charging),
        (b"Discharging\n", ChargeStatus::Discharging),
        (b"Full\n", ChargeStatus::Full),
        (b"Not charging\n", ChargeStatus::NotCharging),
        (b"Unknown\n", ChargeStatus::Unknown),
    ] {
        assert_eq!(parse_status(raw), expected, "parse_status({raw:?})");
    }
    let oversize = vec![b'F'; MAX_SYSFS_VALUE_BYTES + 1];
    for raw in [
        &b"charging"[..],
        b"CHARGING",
        b"Full ",
        b"Foo",
        b"",
        b"Not  charging",
        b"Full\n\n",
        &oversize,
    ] {
        assert_eq!(
            parse_status(raw),
            ChargeStatus::Unknown,
            "parse_status({raw:?})"
        );
    }
    for raw in [&b"Charging\n"[..], b"CHARGING\n", b"Full \n"] {
        let sysfs = FakeSysfs::new();
        sysfs.battery("BAT0", &[("capacity", b"50\n"), ("status", raw)]);
        assert_eq!(sysfs.read().charge, Some(parse_status(raw)), "{raw:?}");
    }
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"50\n")]);
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.charge, Some(ChargeStatus::Unknown), "missing status file");
}

// ---------------------------------------------------------------------------------------
// Test 7: peripherals and empty bays
// ---------------------------------------------------------------------------------------

/// Test 7 (RBS3): `scope=Device` batteries (mice, headsets) are ignored, `System` and
/// `Unknown` scopes and an absent scope are included, a malformed scope is excluded;
/// `present=0` (an empty bay) is skipped, `present` absent or `1` is included.
#[test]
fn test_rbs_peripheral_and_empty_bays_are_ignored() {
    // A desktop with a wireless mouse.
    let sysfs = FakeSysfs::new();
    sysfs.mains("AC", "1");
    sysfs.battery(
        "hidpp_battery_0",
        &[
            ("scope", b"Device\n"),
            ("capacity", b"55\n"),
            ("status", DISCHARGING),
        ],
    );
    assert_eq!(sysfs.read(), BatteryView::no_battery());

    // A laptop with a mouse: only the system battery counts.
    sysfs.battery("BAT0", &[("capacity", b"82\n"), ("status", DISCHARGING)]);
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.percent, Some(82));

    for scope in [&b"System\n"[..], b"Unknown\n"] {
        let sysfs = FakeSysfs::new();
        sysfs.battery(
            "BAT0",
            &[
                ("scope", scope),
                ("capacity", b"70\n"),
                ("status", DISCHARGING),
            ],
        );
        assert_eq!(sysfs.read().percent, Some(70), "scope {scope:?}");
    }
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("scope", b"device\n"),
            ("capacity", b"70\n"),
            ("status", DISCHARGING),
        ],
    );
    assert_eq!(
        sysfs.read(),
        BatteryView::no_battery(),
        "malformed scope excluded"
    );

    // Empty bay.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT1", &[("present", b"0\n"), ("capacity", b"0\n")]);
    assert_eq!(sysfs.read(), BatteryView::no_battery(), "present=0 skipped");
    sysfs.battery(
        "BAT0",
        &[
            ("present", b"1\n"),
            ("capacity", b"64\n"),
            ("status", DISCHARGING),
        ],
    );
    assert_eq!(
        sysfs.read().percent,
        Some(64),
        "present=1 included, bay ignored"
    );
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"64\n"), ("status", DISCHARGING)]);
    assert_eq!(sysfs.read().percent, Some(64), "present absent included");

    // Parser tables.
    assert!(parse_scope_included(None));
    assert!(parse_scope_included(Some(b"System\n")));
    assert!(parse_scope_included(Some(b"Unknown")));
    assert!(!parse_scope_included(Some(b"Device\n")));
    assert!(!parse_scope_included(Some(b"device")));
    assert!(!parse_scope_included(Some(b"")));
    assert!(!parse_present(Some(b"0\n")));
    assert!(!parse_present(Some(b"0")));
    assert!(parse_present(None));
    assert!(parse_present(Some(b"1\n")));
    assert!(parse_present(Some(b"yes")));
    assert!(parse_present(Some(b"00")));
}

// ---------------------------------------------------------------------------------------
// Tests 8, 9: several batteries
// ---------------------------------------------------------------------------------------

/// Test 8 (RBS5, B-9): several batteries are energy-weighted when every one reports a usable
/// pair of the same kind, else the floor mean of the capacities, else unknown.
#[test]
fn test_rbs_multiple_batteries_are_aggregated() {
    // 20 Wh at 50 % + 80 Wh at 100 % → 90 (the mean of the capacities would be 75).
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("capacity", b"50\n"),
            ("status", DISCHARGING),
            ("energy_now", b"10000000\n"),
            ("energy_full", b"20000000\n"),
        ],
    );
    sysfs.battery(
        "BAT1",
        &[
            ("capacity", b"100\n"),
            ("status", b"Full\n"),
            ("energy_now", b"80000000\n"),
            ("energy_full", b"80000000\n"),
        ],
    );
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.percent, Some(90));
    assert_eq!(v.charge, Some(ChargeStatus::Discharging));

    // Charge pairs on both work the same way.
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("capacity", b"50\n"),
            ("charge_now", b"1000\n"),
            ("charge_full", b"2000\n"),
        ],
    );
    sysfs.battery(
        "BAT1",
        &[
            ("capacity", b"100\n"),
            ("charge_now", b"8000\n"),
            ("charge_full", b"8000\n"),
        ],
    );
    assert_eq!(sysfs.read().percent, Some(90));

    // Mixed kinds → mean of capacities.
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("capacity", b"50\n"),
            ("energy_now", b"10000000\n"),
            ("energy_full", b"20000000\n"),
        ],
    );
    sysfs.battery(
        "BAT1",
        &[
            ("capacity", b"100\n"),
            ("charge_now", b"8000\n"),
            ("charge_full", b"8000\n"),
        ],
    );
    assert_eq!(sysfs.read().percent, Some(75), "mixed kinds");

    // One battery without a pair → mean.
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("capacity", b"50\n"),
            ("energy_now", b"10000000\n"),
            ("energy_full", b"20000000\n"),
        ],
    );
    sysfs.battery("BAT1", &[("capacity", b"100\n")]);
    assert_eq!(sysfs.read().percent, Some(75), "one battery without a pair");

    // Floor mean.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"50\n")]);
    sysfs.battery("BAT1", &[("capacity", b"51\n")]);
    assert_eq!(sysfs.read().percent, Some(50), "floor of 50.5");

    // A malformed capacity and no full pair set → unknown.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"50\n"), ("status", DISCHARGING)]);
    sysfs.battery("BAT1", &[("capacity", b"abc\n"), ("status", DISCHARGING)]);
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.percent, None);
    // ... but a full energy pair set still yields the weighted value.
    sysfs.set("BAT0", "energy_now", b"10000000\n");
    sysfs.set("BAT0", "energy_full", b"20000000\n");
    sysfs.set("BAT1", "energy_now", b"80000000\n");
    sysfs.set("BAT1", "energy_full", b"80000000\n");
    assert_eq!(sysfs.read().percent, Some(90));

    // `full = 0` makes a pair unusable → mean.
    sysfs.set("BAT0", "capacity", b"50\n");
    sysfs.set("BAT1", "capacity", b"100\n");
    sysfs.set("BAT1", "energy_full", b"0\n");
    assert_eq!(sysfs.read().percent, Some(75), "full = 0 pair ignored");

    // `now > full` clamps at 100.
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BAT0",
        &[
            ("capacity", b"100\n"),
            ("energy_now", b"21000000\n"),
            ("energy_full", b"20000000\n"),
        ],
    );
    sysfs.battery(
        "BAT1",
        &[
            ("capacity", b"100\n"),
            ("energy_now", b"90000000\n"),
            ("energy_full", b"80000000\n"),
        ],
    );
    assert_eq!(sysfs.read().percent, Some(100), "clamped");

    // The same rules through the pure aggregation.
    let e = |now, full| Some(EnergyPair::Energy { now, full });
    let c = |now, full| Some(EnergyPair::Charge { now, full });
    let agg = |rs: &[BatteryReading]| aggregate(rs, &[]).percent;
    let d = ChargeStatus::Discharging;
    assert_eq!(
        agg(&[
            reading(Some(50), d, e(10, 20)),
            reading(Some(100), d, e(80, 80))
        ]),
        Some(90)
    );
    assert_eq!(
        agg(&[
            reading(Some(50), d, e(10, 20)),
            reading(Some(100), d, c(80, 80))
        ]),
        Some(75)
    );
    assert_eq!(
        agg(&[reading(Some(50), d, None), reading(Some(51), d, None)]),
        Some(50)
    );
    assert_eq!(
        agg(&[reading(None, d, None), reading(Some(51), d, None)]),
        None
    );
    assert_eq!(
        agg(&[
            reading(Some(1), d, e(30, 20)),
            reading(Some(1), d, e(90, 80))
        ]),
        Some(100)
    );
    assert_eq!(
        agg(&[reading(Some(37), d, e(1, 2))]),
        Some(37),
        "single: capacity"
    );
    assert_eq!(
        agg(&[reading(None, d, e(1, 2))]),
        None,
        "single: never from energy"
    );
}

/// Test 9 (RBS5, §5.3): charge aggregation precedence.
#[test]
fn test_rbs_charge_aggregation_table() {
    use ChargeStatus::{Charging, Discharging, Full, NotCharging, Unknown};
    for (a, b, expected) in [
        (Charging, Discharging, Charging),
        (Discharging, Charging, Charging),
        (Discharging, Full, Discharging),
        (Full, Full, Full),
        (Full, NotCharging, NotCharging),
        (NotCharging, Unknown, NotCharging),
        (Unknown, Full, Unknown),
        (Unknown, Unknown, Unknown),
        (Unknown, Charging, Charging),
        (Unknown, Discharging, Discharging),
    ] {
        let v = aggregate(
            &[reading(Some(50), a, None), reading(Some(50), b, None)],
            &[],
        );
        assert_eq!(v.state, BatteryState::Present);
        assert_eq!(v.charge, Some(expected), "{a:?} + {b:?}");
    }
    let one = aggregate(&[reading(Some(50), Full, None)], &[]);
    assert_eq!(one.charge, Some(Full));
    assert_eq!(aggregate(&[], &[true]), BatteryView::no_battery());
}

// ---------------------------------------------------------------------------------------
// Tests 10–12: bounds and incomplete scans
// ---------------------------------------------------------------------------------------

/// Test 10 (RBS4, RBS6, B-10, F-6): at most `MAX_POWER_SUPPLIES` entries and `MAX_BATTERIES`
/// batteries; an entry with an invalid name is skipped and makes the scan incomplete, so it
/// never yields `no_battery`.
#[test]
fn test_rbs_entry_and_battery_bounds() {
    assert_eq!(MAX_POWER_SUPPLIES, 64);
    assert_eq!(MAX_BATTERIES, 8);
    assert_eq!(MAX_POWER_SUPPLY_NAME_LEN, 64);

    // Exactly 64 entries → read; 65 → unavailable.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"82\n"), ("status", DISCHARGING)]);
    for i in 1..MAX_POWER_SUPPLIES {
        sysfs.mains(&format!("ADP{i}"), "0");
    }
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present, "64 entries accepted");
    assert_eq!(v.percent, Some(82));
    sysfs.mains("ADPX", "0");
    assert_eq!(sysfs.read(), BatteryView::unavailable(), "65 entries");

    // Exactly 8 batteries → read; 9 → unavailable.
    let sysfs = FakeSysfs::new();
    for i in 0..MAX_BATTERIES {
        sysfs.battery(&format!("BAT{i}"), &[("capacity", b"40\n")]);
    }
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present, "8 batteries accepted");
    assert_eq!(v.percent, Some(40));
    sysfs.battery("BAT8", &[("capacity", b"40\n")]);
    assert_eq!(sysfs.read(), BatteryView::unavailable(), "9 batteries");
    let many: Vec<BatteryReading> = (0..=MAX_BATTERIES)
        .map(|_| reading(Some(40), ChargeStatus::Full, None))
        .collect();
    assert_eq!(aggregate(&many, &[]), BatteryView::unavailable());
    assert_eq!(aggregate(&many[..MAX_BATTERIES], &[]).percent, Some(40));

    // Invalid names.
    let long = vec![b'B'; MAX_POWER_SUPPLY_NAME_LEN + 1];
    let invalid: Vec<&[u8]> = vec![
        b".hidden",
        &long,
        b"bat 0",
        "b\u{e4}t0".as_bytes(),
        b"bat\xff0",
    ];
    for name in &invalid {
        assert!(!valid_supply_name(OsStr::from_bytes(name)), "{name:?}");
        // Next to a valid BAT0: present with BAT0's values only (10 % would change the mean).
        let sysfs = FakeSysfs::new();
        sysfs.battery("BAT0", &[("capacity", b"82\n"), ("status", DISCHARGING)]);
        sysfs.raw_entry(
            name,
            &[
                ("type", b"Battery\n"),
                ("capacity", b"10\n"),
                ("status", b"Charging\n"),
            ],
        );
        let v = sysfs.read();
        assert_eq!(v.state, BatteryState::Present, "{name:?}");
        assert_eq!(v.percent, Some(82), "{name:?} must be skipped");
        assert_eq!(v.charge, Some(ChargeStatus::Discharging), "{name:?}");
        // Alone → unavailable, never no_battery.
        let sysfs = FakeSysfs::new();
        sysfs.raw_entry(name, &[("type", b"Battery\n"), ("capacity", b"10\n")]);
        assert_eq!(sysfs.read(), BatteryView::unavailable(), "{name:?} alone");
        // Next to only a mains supply → unavailable.
        sysfs.mains("AC", "1");
        assert_eq!(sysfs.read(), BatteryView::unavailable(), "{name:?} + mains");
    }
    for name in [
        "BAT0",
        "AC",
        "ADP1",
        "hidpp_battery_0",
        "ucsi-source-psy-USBC000:001",
        "a.b",
        "x",
    ] {
        assert!(valid_supply_name(OsStr::new(name)), "{name}");
    }
    assert!(valid_supply_name(OsStr::from_bytes(
        &[b'B'; MAX_POWER_SUPPLY_NAME_LEN]
    )));
    assert!(!valid_supply_name(OsStr::new("")));
    assert!(!valid_supply_name(OsStr::new("a/b")));
}

/// Waits (real time, bounded) for `f` on another thread; a reader that blocks on the FIFO
/// fails the test instead of hanging it.
fn within_real_time<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
    unblock: impl FnOnce(),
) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(v) => v,
        Err(_) => {
            unblock();
            panic!("the reader blocked on a non-regular attribute");
        }
    }
}

/// Test 11 (RBS6, §5.4 step 4): a FIFO, a directory or a symlink attribute is never read
/// (`O_NOFOLLOW | O_NONBLOCK` and `fstat` regular-file check): the value is unknown and the
/// read finishes promptly.
#[test]
fn test_rbs_non_regular_attributes_are_not_read() {
    // FIFO.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("status", DISCHARGING)]);
    let fifo = sysfs.fifo("BAT0", "capacity");
    let root = sysfs.root();
    let v = within_real_time(
        move || read_power_supplies(&root),
        move || {
            // Opening the write end releases a reader blocked in `open`.
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
        },
    );
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.percent, None, "FIFO capacity");

    // Directory.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("status", DISCHARGING)]);
    sysfs.dir_attr("BAT0", "capacity");
    let v = sysfs.read();
    assert_eq!(
        (v.state, v.percent),
        (BatteryState::Present, None),
        "directory capacity"
    );

    // Symlink to a valid file.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("status", DISCHARGING)]);
    sysfs.symlink_attr("BAT0", "capacity", b"82\n");
    let v = sysfs.read();
    assert_eq!(
        (v.state, v.percent),
        (BatteryState::Present, None),
        "symlink capacity"
    );

    // A symlinked `type` is unreadable: the scan is incomplete.
    let sysfs = FakeSysfs::new();
    sysfs.raw_entry(b"BAT0", &[("capacity", b"82\n")]);
    sysfs.symlink_attr("BAT0", "type", b"Battery\n");
    assert_eq!(sysfs.read(), BatteryView::unavailable(), "symlink type");
}

/// Test 12 (RBS3, B-10): an entry whose `type` is missing or malformed makes the scan
/// incomplete: alone it is `unavailable`, next to a valid battery the battery is reported.
#[test]
fn test_rbs_type_unreadable_never_claims_no_battery() {
    let oversize = vec![b'B'; MAX_SYSFS_VALUE_BYTES + 1];
    let malformed: Vec<Option<&[u8]>> = vec![
        None,
        Some(b""),
        Some(b"\n"),
        Some(&[0xff]),
        Some(&oversize),
        Some(b"Bat\x00tery"),
    ];
    for ty in &malformed {
        let sysfs = FakeSysfs::new();
        match ty {
            None => sysfs.raw_entry(b"BAT0", &[("capacity", b"82\n")]),
            Some(raw) => sysfs.raw_entry(b"BAT0", &[("type", raw), ("capacity", b"82\n")]),
        };
        assert_eq!(
            sysfs.read(),
            BatteryView::unavailable(),
            "type {ty:?} alone"
        );
        sysfs.mains("AC", "1");
        assert_eq!(
            sysfs.read(),
            BatteryView::unavailable(),
            "type {ty:?} + mains"
        );
        sysfs.battery("BAT1", &[("capacity", b"64\n"), ("status", DISCHARGING)]);
        let v = sysfs.read();
        assert_eq!(v.state, BatteryState::Present, "type {ty:?} + battery");
        assert_eq!(v.percent, Some(64), "type {ty:?}");
    }
}

// ---------------------------------------------------------------------------------------
// Test 13: external power
// ---------------------------------------------------------------------------------------

/// Test 13 (RBS4, B-12, F-5): `online` is `0` → `false`, `1` and `2` → `true`, anything
/// else unknown; any online non-battery system supply means external power; a `Device`
/// supply is ignored.
#[test]
fn test_rbs_external_power_rules() {
    let with = |kind: &str, attrs: &[(&str, &[u8])]| {
        let sysfs = FakeSysfs::new();
        sysfs.battery("BAT0", &[("capacity", b"50\n"), ("status", DISCHARGING)]);
        sysfs.supply("PSU0", kind, attrs);
        sysfs.read().external_power
    };
    assert_eq!(with("USB", &[("online", b"1\n")]), Some(true));
    assert_eq!(with("Mains", &[("online", b"2\n")]), Some(true));
    assert_eq!(with("Mains", &[("online", b"1\n")]), Some(true));
    assert_eq!(with("UPS", &[("online", b"0\n")]), Some(false));
    for bad in [&b"3\n"[..], b"01\n", b"-1\n", b"", b"yes\n", b"1\n\n"] {
        assert_eq!(
            with("Mains", &[("online", bad)]),
            None,
            "online {bad:?} ignored"
        );
    }
    assert_eq!(with("Mains", &[]), None, "no online attribute");
    assert_eq!(
        with("USB", &[("scope", b"Device\n"), ("online", b"1\n")]),
        None,
        "Device supply ignored"
    );

    // Any `true` wins; only `false` values → `false`.
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"50\n"), ("status", DISCHARGING)]);
    sysfs.mains("AC", "0");
    sysfs.supply("ucsi-source-psy-USBC000:001", "USB", &[("online", b"1\n")]);
    assert_eq!(sysfs.read().external_power, Some(true));
    assert_eq!(
        aggregate(
            &[reading(Some(1), ChargeStatus::Full, None)],
            &[false, true]
        )
        .external_power,
        Some(true)
    );
    assert_eq!(
        aggregate(&[reading(Some(1), ChargeStatus::Full, None)], &[false]).external_power,
        Some(false)
    );
    assert_eq!(
        aggregate(&[reading(Some(1), ChargeStatus::Full, None)], &[]).external_power,
        None
    );

    for (raw, expected) in [
        (&b"0"[..], Some(false)),
        (b"1", Some(true)),
        (b"2", Some(true)),
        (b"2\n", Some(true)),
        (b"0\n", Some(false)),
        (b"3", None),
        (b"01", None),
        (b"-1", None),
        (b"", None),
        (b"yes", None),
        (b" 1", None),
    ] {
        assert_eq!(parse_online(raw), expected, "parse_online({raw:?})");
    }
}

// ---------------------------------------------------------------------------------------
// Test 14: JSON shape and identity
// ---------------------------------------------------------------------------------------

/// Test 14 (RBS7, RBS-I2): the exact JSON of the three §6.7 examples, exactly four keys, and
/// no supply name, serial, model or manufacturer in the serialized view or its `Debug`.
#[test]
fn test_rbs_view_json_shape_has_no_identity() {
    let examples = [
        (
            view(
                BatteryState::Present,
                Some(82),
                Some(ChargeStatus::Discharging),
                Some(false),
            ),
            r#"{"state":"present","percent":82,"charge":"discharging","external_power":false}"#,
        ),
        (
            BatteryView::no_battery(),
            r#"{"state":"no_battery","percent":null,"charge":null,"external_power":null}"#,
        ),
        (
            BatteryView::disabled(),
            r#"{"state":"disabled","percent":null,"charge":null,"external_power":null}"#,
        ),
        (
            BatteryView::unavailable(),
            r#"{"state":"unavailable","percent":null,"charge":null,"external_power":null}"#,
        ),
    ];
    for (v, json) in examples {
        assert_eq!(serde_json::to_string(&v).unwrap(), json);
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["charge", "external_power", "percent", "state"]);
    }
    for (charge, word) in [
        (ChargeStatus::Charging, "charging"),
        (ChargeStatus::Discharging, "discharging"),
        (ChargeStatus::Full, "full"),
        (ChargeStatus::NotCharging, "not_charging"),
        (ChargeStatus::Unknown, "unknown"),
    ] {
        assert_eq!(
            serde_json::to_string(&charge).unwrap(),
            format!("\"{word}\"")
        );
    }

    const MARKER: &str = "IDMARKER77";
    let sysfs = FakeSysfs::new();
    sysfs.battery(
        "BATMARKER0",
        &[
            ("capacity", b"82\n"),
            ("status", DISCHARGING),
            ("serial_number", format!("{MARKER}\n").as_bytes()),
            ("model_name", format!("{MARKER}\n").as_bytes()),
            ("manufacturer", format!("{MARKER}\n").as_bytes()),
        ],
    );
    sysfs.mains("ADPMARKER1", "0");
    let v = sysfs.read();
    assert_eq!(v.percent, Some(82));
    let json = serde_json::to_string(&v).unwrap();
    let debug = format!("{v:?}");
    for text in [&json, &debug] {
        for needle in [MARKER, "BATMARKER0", "ADPMARKER1", "MARKER"] {
            assert!(!text.contains(needle), "{text} leaks {needle}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tests 15, 16: never panic, never overflow
// ---------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Test 15 (RBS4): every parser accepts arbitrary bytes without panicking, and
    /// `parse_capacity` accepts only the canonical decimal of a value ≤ 100 (optionally
    /// followed by one newline).
    #[test]
    fn test_rbs_parsers_never_panic(raw in proptest::collection::vec(any::<u8>(), 0..=64)) {
        let capacity = parse_capacity(&raw);
        let _ = parse_status(&raw);
        let online = parse_online(&raw);
        let micro = parse_micro(&raw);
        let _ = parse_scope_included(Some(&raw));
        let _ = parse_present(Some(&raw));
        let _ = valid_supply_name(OsStr::from_bytes(&raw));
        if let Some(v) = capacity {
            prop_assert!(v <= 100);
            let canonical = v.to_string().into_bytes();
            let mut with_newline = canonical.clone();
            with_newline.push(b'\n');
            prop_assert!(raw == canonical || raw == with_newline, "{raw:?} → {v}");
        }
        if online.is_some() {
            prop_assert!([&b"0"[..], b"1", b"2", b"0\n", b"1\n", b"2\n"].contains(&raw.as_slice()));
        }
        if let Some(v) = micro {
            let digits = raw.strip_suffix(b"\n").unwrap_or(&raw);
            prop_assert!(!digits.is_empty() && digits.len() <= MAX_SYSFS_MICRO_DIGITS);
            prop_assert!(digits.iter().all(u8::is_ascii_digit));
            prop_assert_eq!(std::str::from_utf8(digits).unwrap().parse::<u64>().unwrap(), v);
        }
    }

    /// Test 15 (canonical capacities): every value 0..=100 round-trips.
    #[test]
    fn test_rbs_parsers_never_panic_canonical_capacity(v in 0u8..=100, newline in any::<bool>()) {
        let mut raw = v.to_string().into_bytes();
        if newline {
            raw.push(b'\n');
        }
        prop_assert_eq!(parse_capacity(&raw), Some(v));
    }
}

/// Test 16 (RBS5): energies at `u64::MAX` and 19-digit values never panic; an overflowing
/// weighted sum falls back to the mean of the capacities (§5.3).
#[test]
fn test_rbs_aggregate_never_overflows() {
    assert_eq!(MAX_SYSFS_MICRO_DIGITS, 19);
    assert_eq!(
        parse_micro(b"9999999999999999999"),
        Some(9_999_999_999_999_999_999)
    );
    assert_eq!(
        parse_micro(b"9999999999999999999\n"),
        Some(9_999_999_999_999_999_999)
    );
    assert_eq!(parse_micro(b"18446744073709551615"), None, "20 digits");
    assert_eq!(
        parse_micro(b"00000000000000000042"),
        None,
        "20 digits with zeros"
    );
    assert_eq!(parse_micro(b"0000042"), Some(42), "leading zeros accepted");
    assert_eq!(parse_micro(b""), None);
    assert_eq!(parse_micro(b"-1"), None);
    assert_eq!(parse_micro(b"1 2"), None);

    let d = ChargeStatus::Discharging;
    let max = Some(EnergyPair::Energy {
        now: u64::MAX,
        full: u64::MAX,
    });
    let v = aggregate(&[reading(Some(30), d, max), reading(Some(70), d, max)], &[]);
    assert_eq!(v.state, BatteryState::Present);
    assert_eq!(v.percent, Some(50), "overflow falls back to the mean");
    let v = aggregate(&[reading(None, d, max), reading(Some(70), d, max)], &[]);
    assert_eq!(v.percent, None, "overflow and an unknown capacity");
    let max_charge = Some(EnergyPair::Charge {
        now: u64::MAX,
        full: 1,
    });
    let v = aggregate(
        &[
            reading(Some(10), d, max_charge),
            reading(Some(20), d, max_charge),
        ],
        &[],
    );
    assert_eq!(v.percent, Some(15));

    // 19-digit values through the reader: no panic, a bounded result.
    let sysfs = FakeSysfs::new();
    for name in ["BAT0", "BAT1"] {
        sysfs.battery(
            name,
            &[
                ("capacity", b"40\n"),
                ("energy_now", b"9999999999999999999\n"),
                ("energy_full", b"9999999999999999999\n"),
            ],
        );
    }
    let v = sysfs.read();
    assert_eq!(v.state, BatteryState::Present);
    assert!(v.percent.is_some_and(|p| p <= 100), "{v:?}");
    // A 20-digit value is malformed: no pair, the mean of the capacities.
    sysfs.set("BAT0", "energy_now", b"18446744073709551615\n");
    assert_eq!(sysfs.read().percent, Some(40));
}

// ---------------------------------------------------------------------------------------
// Test 17: live re-read
// ---------------------------------------------------------------------------------------

/// Test 17 (RBS2, F-3 a): the production source re-reads sysfs on every call (no internal
/// cache) and keeps following the firmware capacity of a single battery.
#[test]
fn test_rbs_sysfs_source_rereads_live() {
    let sysfs = FakeSysfs::new();
    sysfs.battery("BAT0", &[("capacity", b"82\n"), ("status", DISCHARGING)]);
    let source = SysfsBattery::new(sysfs.root());
    assert_eq!(source.read().percent, Some(82));
    sysfs.set("BAT0", "capacity", b"81\n");
    assert_eq!(source.read().percent, Some(81));
    sysfs.set("BAT0", "status", b"Charging\n");
    assert_eq!(source.read().charge, Some(ChargeStatus::Charging));
    sysfs.set("BAT0", "energy_now", b"10\n");
    sysfs.set("BAT0", "energy_full", b"100\n");
    assert_eq!(
        source.read().percent,
        Some(81),
        "capacity, not the energy ratio"
    );
    sysfs.set("BAT0", "capacity", b"79\n");
    assert_eq!(source.read().percent, Some(79));
    let dynamic: &dyn BatterySource = &source;
    assert_eq!(dynamic.read().percent, Some(79));
    let _ = Path::new(POWER_SUPPLY_ROOT);
    let _kernel = SysfsBattery::kernel();
}
