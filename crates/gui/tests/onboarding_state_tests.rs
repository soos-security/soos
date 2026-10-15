//! Contract (GitHub #379, PR 1 of #363, matrix OBT1–OBT4): the first-run tour page machine and
//! its per-user state file.
//!
//! - The tour opens on first launch only, is skippable and can be reopened at any time.
//! - `Tour::generation` changes on every page or visibility change (the motion hook of #377).
//! - The state file lives in the user's configuration directory, is read with a bound, is
//!   written atomically with mode 0600 and is never read or written through a symbolic link.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use soos_gui::onboarding::{
    parse_tour_state, should_show_tour, tour_state_path, Tour, TourLoad, TourPage, TourStore,
    TourStoreError, FEATURES, MAX_TOUR_STATE_BYTES, TOUR_PAGE_COUNT, TOUR_STATE_FILE_NAME,
    TOUR_VERSION,
};

// ---------------------------------------------------------------------------------------------
// Page machine
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_page_count_is_intro_features_and_overview() {
    assert_eq!(FEATURES.len(), 9);
    assert_eq!(TOUR_PAGE_COUNT, FEATURES.len() + 2);
}

#[test]
fn test_obt_closed_tour_is_not_open() {
    let tour = Tour::closed();
    assert!(!tour.is_open());
}

#[test]
fn test_obt_first_run_opens_on_intro_and_walks_every_feature_in_order() {
    let mut tour = Tour::first_run();
    assert!(tour.is_open());
    assert_eq!(tour.index(), 0);
    assert_eq!(tour.page(), TourPage::Intro);
    for feature in FEATURES {
        tour.next();
        assert_eq!(tour.page(), TourPage::Feature(feature));
    }
    tour.next();
    assert_eq!(tour.page(), TourPage::Overview);
    assert_eq!(tour.index(), TOUR_PAGE_COUNT - 1);
    // Saturates at the overview.
    tour.next();
    assert_eq!(tour.page(), TourPage::Overview);
    assert_eq!(tour.index(), TOUR_PAGE_COUNT - 1);
}

#[test]
fn test_obt_back_saturates_at_intro() {
    let mut tour = Tour::first_run();
    tour.back();
    assert_eq!(tour.page(), TourPage::Intro);
    tour.next();
    tour.next();
    tour.back();
    assert_eq!(tour.page(), TourPage::Feature(FEATURES[0]));
}

#[test]
fn test_obt_close_restart_and_overview() {
    let mut tour = Tour::first_run();
    tour.next();
    tour.close();
    assert!(!tour.is_open());
    tour.open_overview();
    assert!(tour.is_open());
    assert_eq!(tour.page(), TourPage::Overview);
    tour.restart();
    assert!(tour.is_open());
    assert_eq!(tour.page(), TourPage::Intro);
}

#[test]
fn test_obt_generation_changes_on_every_page_or_visibility_change() {
    let mut tour = Tour::first_run();
    let mut last = tour.generation();
    let mut expect_change = |tour: &Tour, what: &str| {
        let now = tour.generation();
        assert_ne!(now, last, "generation must change after {what}");
        last = now;
    };
    tour.next();
    expect_change(&tour, "next");
    tour.back();
    expect_change(&tour, "back");
    tour.close();
    expect_change(&tour, "close");
    tour.open_overview();
    expect_change(&tour, "open_overview");
    tour.restart();
    expect_change(&tour, "restart");
}

#[test]
fn test_obt_generation_unchanged_when_navigation_saturates() {
    let mut tour = Tour::first_run();
    let before = tour.generation();
    tour.back();
    assert_eq!(
        tour.generation(),
        before,
        "back on the intro changes nothing"
    );
}

// ---------------------------------------------------------------------------------------------
// State path and parsing
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_state_path_prefers_absolute_xdg_config_home() {
    let path = tour_state_path(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/u")));
    assert_eq!(
        path,
        Some(PathBuf::from("/xdg/soos").join(TOUR_STATE_FILE_NAME))
    );
    assert_eq!(TOUR_STATE_FILE_NAME, "gui-tour.json");
}

#[test]
fn test_obt_state_path_falls_back_to_home_config() {
    for xdg in [None, Some(OsStr::new("")), Some(OsStr::new("relative/dir"))] {
        assert_eq!(
            tour_state_path(xdg, Some(OsStr::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config/soos/gui-tour.json")),
            "xdg = {xdg:?}"
        );
    }
}

#[test]
fn test_obt_state_path_none_without_absolute_dirs() {
    assert_eq!(tour_state_path(None, None), None);
    assert_eq!(
        tour_state_path(Some(OsStr::new("rel")), Some(OsStr::new("also-rel"))),
        None
    );
    assert_eq!(tour_state_path(None, Some(OsStr::new(""))), None);
}

#[test]
fn test_obt_parse_tour_state_accepts_only_seen_records() {
    assert_eq!(parse_tour_state(br#"{"version":1,"seen":true}"#), Some(1));
    assert_eq!(
        parse_tour_state(b"{\"seen\": true, \"version\": 7, \"other\": 3}\n"),
        Some(7)
    );
    for bad in [
        &br#"{"version":1,"seen":false}"#[..],
        br#"{"version":1}"#,
        br#"{"seen":true}"#,
        br#"{"version":"1","seen":true}"#,
        br#"{"version":-1,"seen":true}"#,
        br#"{"version":4294967296,"seen":true}"#,
        br#"{"version":1,"seen":"yes"}"#,
        br"[1, true]",
        b"",
        b"\xff\xfe garbage",
    ] {
        assert_eq!(
            parse_tour_state(bad),
            None,
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn test_obt_should_show_tour_only_when_not_seen_or_older() {
    assert!(should_show_tour(TourLoad::NotSeen));
    assert!(!should_show_tour(TourLoad::Unavailable));
    assert!(!should_show_tour(TourLoad::Seen {
        version: TOUR_VERSION
    }));
    assert!(!should_show_tour(TourLoad::Seen {
        version: TOUR_VERSION + 1
    }));
    assert!(should_show_tour(TourLoad::Seen {
        version: TOUR_VERSION - 1
    }));
}

// ---------------------------------------------------------------------------------------------
// TourStore
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_store_missing_file_is_not_seen() {
    let dir = tempfile::tempdir().unwrap();
    let store = TourStore::new(dir.path().join("soos").join(TOUR_STATE_FILE_NAME));
    assert_eq!(store.load(), TourLoad::NotSeen);
}

#[test]
fn test_obt_store_mark_seen_round_trip_with_private_modes() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("cfg").join("soos");
    let path = parent.join(TOUR_STATE_FILE_NAME);
    let store = TourStore::new(path.clone());
    store.mark_seen().unwrap();
    assert_eq!(
        store.load(),
        TourLoad::Seen {
            version: TOUR_VERSION
        }
    );
    let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600);
    let dir_mode = std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    // No temporary file is left behind.
    let names: Vec<_> = std::fs::read_dir(&parent)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(TOUR_STATE_FILE_NAME)]);
    // Idempotent.
    store.mark_seen().unwrap();
    assert_eq!(
        store.load(),
        TourLoad::Seen {
            version: TOUR_VERSION
        }
    );
}

#[test]
fn test_obt_store_malformed_or_oversized_file_is_not_seen_and_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(TOUR_STATE_FILE_NAME);
    std::fs::write(&path, b"not json").unwrap();
    let store = TourStore::new(path.clone());
    assert_eq!(store.load(), TourLoad::NotSeen);

    let big = vec![b' '; usize::try_from(MAX_TOUR_STATE_BYTES).unwrap() + 1];
    std::fs::write(&path, &big).unwrap();
    assert_eq!(store.load(), TourLoad::NotSeen);

    store.mark_seen().unwrap();
    assert_eq!(
        store.load(),
        TourLoad::Seen {
            version: TOUR_VERSION
        }
    );
}

#[test]
fn test_obt_store_padded_valid_file_at_the_bound_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(TOUR_STATE_FILE_NAME);
    let mut body = br#"{"version":1,"seen":true}"#.to_vec();
    body.resize(usize::try_from(MAX_TOUR_STATE_BYTES).unwrap(), b' ');
    std::fs::write(&path, &body).unwrap();
    assert_eq!(TourStore::new(path).load(), TourLoad::Seen { version: 1 });
}

#[test]
fn test_obt_store_refuses_symlinks_for_read_and_write() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.json");
    std::fs::write(&target, br#"{"version":1,"seen":true}"#).unwrap();
    let link = dir.path().join(TOUR_STATE_FILE_NAME);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let store = TourStore::new(link.clone());
    assert_eq!(store.load(), TourLoad::Unavailable);
    assert!(matches!(store.mark_seen(), Err(TourStoreError::Symlink)));
    // The link target is untouched.
    assert_eq!(
        std::fs::read(&target).unwrap(),
        br#"{"version":1,"seen":true}"#.to_vec()
    );
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn test_obt_store_directory_in_place_of_file_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(TOUR_STATE_FILE_NAME);
    std::fs::create_dir(&path).unwrap();
    assert_eq!(TourStore::new(path).load(), TourLoad::Unavailable);
}
