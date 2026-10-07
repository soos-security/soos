//! Card widget ids stay stable across egui passes.
//!
//! The right column measures its card on one pass and may show or hide a stat tile above it on
//! the next (egui runs a second pass in the same frame when a widget requests it). The ids of
//! the widgets inside a card used to come from the parent `Ui` counter, so they shifted with
//! whatever was laid out before the card: debug builds logged "Widget rect changed id between
//! passes" and egui lost per-widget state (hover, scroll, focus) for that frame. A card's
//! content ids must depend only on its `id_salt`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual integration tests use assertions"
)]

use eframe::egui::{self, Id, Pos2, Rect};
use soos_gui::widgets::{card_in_rect, card_in_rect_with_footer};

/// Lays out `extra_before` throwaway widgets, then a card, and returns the id of the first
/// widget inside the card.
fn inner_id(extra_before: usize, with_footer: bool) -> Id {
    let ctx = egui::Context::default();
    let mut id = Id::NULL;
    let _ = ctx.run_ui(Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            for i in 0..extra_before {
                ui.label(format!("before {i}"));
            }
            let rect = Rect::from_min_max(Pos2::new(10.0, 200.0), Pos2::new(310.0, 600.0));
            if with_footer {
                let _ = card_in_rect_with_footer(ui, rect, "id_test_card", 40.0, |ui| {
                    id = ui.button("inside").id;
                });
            } else {
                card_in_rect(ui, rect, "id_test_card", |ui| {
                    id = ui.button("inside").id;
                });
            }
        });
    });
    id
}

#[test]
fn test_card_content_ids_ignore_preceding_widgets() {
    assert_ne!(inner_id(0, false), Id::NULL);
    assert_eq!(inner_id(0, false), inner_id(2, false));
}

#[test]
fn test_footer_card_content_ids_ignore_preceding_widgets() {
    assert_ne!(inner_id(0, true), Id::NULL);
    assert_eq!(inner_id(0, true), inner_id(3, true));
}
