//! Contract (GitHub #379, PR 1 of #363, matrix OBT15–OBT16): the tour window renders headless on
//! every page and state, emits nothing without input, closes on Escape only while no action is
//! pending, and its rendering code performs no I/O.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use eframe::egui;
use soos_gui::onboarding::{
    FeatureSnapshot, RemoteConfigState, RemoteKeys, Tour, UnitState, TOUR_PAGE_COUNT,
};
use soos_gui::onboarding_view::{show_tour, TourEvent, TourView};

fn screen_input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1120.0, 780.0),
        )),
        events,
        ..Default::default()
    }
}

fn render(
    ctx: &egui::Context,
    tour: &Tour,
    view: &TourView<'_>,
    input: egui::RawInput,
) -> Vec<TourEvent> {
    let mut events = Vec::new();
    let _ = ctx.run_ui(input, |ui| {
        events = show_tour(ui.ctx(), tour, view);
    });
    events
}

fn snapshots() -> Vec<FeatureSnapshot> {
    let on = FeatureSnapshot {
        global_disabled: Some(false),
        presence_disabled: Some(false),
        network_lock_installed: Some(true),
        network_lock_rfkill_installed: Some(false),
        daemon_presence_enabled: Some(true),
        daemon_remote_view: Some(true),
        remote_unit: UnitState::Active,
        remote_config: RemoteConfigState::Keys(RemoteKeys {
            allow_unlock: true,
            allow_funnel: true,
            password_alerts: true,
            push_notifications: true,
            camera_view: true,
            camera_view_funnel: true,
        }),
        running_as_root: false,
    };
    let off = FeatureSnapshot {
        global_disabled: Some(true),
        presence_disabled: Some(true),
        network_lock_installed: None,
        network_lock_rfkill_installed: None,
        daemon_presence_enabled: Some(false),
        daemon_remote_view: None,
        remote_unit: UnitState::NotInstalled,
        remote_config: RemoteConfigState::Missing,
        running_as_root: true,
    };
    vec![FeatureSnapshot::unknown(), on, off]
}

#[test]
fn test_obt_every_page_renders_headless_without_events() {
    let ctx = egui::Context::default();
    for snapshot in snapshots() {
        for (loaded, busy, message) in [
            (false, false, None),
            (
                true,
                false,
                Some(("Presence auto-unlock is now off.", false)),
            ),
            (true, true, Some(("Authorization denied.", true))),
        ] {
            let view = TourView {
                snapshot: &snapshot,
                loaded,
                busy,
                message,
            };
            let mut tour = Tour::first_run();
            for _ in 0..TOUR_PAGE_COUNT {
                // Two frames per page: the modal lays out on the first one.
                for _ in 0..2 {
                    let events = render(&ctx, &tour, &view, screen_input(Vec::new()));
                    assert!(events.is_empty(), "no input, no event: {events:?}");
                }
                tour.next();
            }
        }
    }
}

#[test]
fn test_obt_closed_tour_renders_nothing() {
    let ctx = egui::Context::default();
    let snapshot = FeatureSnapshot::unknown();
    let view = TourView {
        snapshot: &snapshot,
        loaded: false,
        busy: false,
        message: None,
    };
    let events = render(&ctx, &Tour::closed(), &view, screen_input(Vec::new()));
    assert!(events.is_empty());
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn test_obt_escape_closes_only_when_idle() {
    let snapshot = FeatureSnapshot::unknown();
    for (busy, expect_close) in [(false, true), (true, false)] {
        let ctx = egui::Context::default();
        let view = TourView {
            snapshot: &snapshot,
            loaded: true,
            busy,
            message: None,
        };
        let tour = Tour::first_run();
        let _ = render(&ctx, &tour, &view, screen_input(Vec::new()));
        let events = render(&ctx, &tour, &view, screen_input(vec![escape()]));
        assert_eq!(
            events.contains(&TourEvent::Close),
            expect_close,
            "busy = {busy}: {events:?}"
        );
    }
}

#[test]
fn test_obt_view_code_performs_no_io() {
    let src = include_str!("../src/onboarding_view.rs");
    for forbidden in [
        "Command::new",
        "std::process",
        "std::fs",
        "File::",
        "pkexec",
    ] {
        assert!(
            !src.contains(forbidden),
            "onboarding_view.rs runs on the UI thread and must not contain {forbidden}"
        );
    }
}

#[test]
fn test_obt_app_ui_code_still_spawns_no_process() {
    let src = include_str!("../src/app.rs");
    assert!(!src.contains("Command::new"));
    assert!(
        src.contains("FeatureRunner"),
        "app.rs must use the background FeatureRunner"
    );
}
