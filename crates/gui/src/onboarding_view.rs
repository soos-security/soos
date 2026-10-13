//! Rendering of the first-run tour window (GitHub #379, PR 1 of #363).
//!
//! [`show_tour`] only paints and reports [`TourEvent`]s: it never runs a process and never
//! touches a file. The page content sits in a scope salted with [`Tour::generation`], the hook
//! the motion system of #377 will use to animate page changes. No animation is added here, so
//! reduced-motion preferences are trivially respected.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    reason = "GUI coordinate calculations on bounded f32 values and small page counts"
)]

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::feature_control::FeatureAction;
use crate::onboarding::{
    toggle_route, Feature, FeatureSnapshot, FeatureState, ToggleRoute, Tour, TourPage, FEATURES,
    TOUR_PAGE_COUNT,
};
use crate::theme;
use crate::widgets::{self, BannerKind, BrandButton};

/// What the user asked for in the tour window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourEvent {
    /// Go to the next page.
    Next,
    /// Go to the previous page.
    Back,
    /// Close the tour (skip, done, Escape or a click outside).
    Close,
    /// Start the tour again from the introduction.
    Restart,
    /// Carry out a feature change.
    Apply(FeatureAction),
}

/// Data the tour window shows.
#[derive(Debug, Clone, Copy)]
pub struct TourView<'a> {
    /// Last probed facts.
    pub snapshot: &'a FeatureSnapshot,
    /// A probe has completed at least once.
    pub loaded: bool,
    /// A privileged or feature action is in flight: controls and closing are disabled.
    pub busy: bool,
    /// Result of the last change, `(text, is_error)`.
    pub message: Option<(&'a str, bool)>,
}

/// Preferred width of the tour window.
const TOUR_WIDTH: f32 = 540.0;

/// Paints the tour window when `tour` is open and returns the user's requests.
pub fn show_tour(ctx: &egui::Context, tour: &Tour, view: &TourView<'_>) -> Vec<TourEvent> {
    let mut events = Vec::new();
    if !tour.is_open() {
        return events;
    }
    let frame = egui::Frame::new()
        .fill(theme::PALE)
        .stroke(Stroke::new(theme::STROKE_CARD, theme::BLUE))
        .corner_radius(CornerRadius::same(theme::R_CARD))
        .inner_margin(egui::Margin::same(24));
    let width = TOUR_WIDTH.min((ctx.content_rect().width() - 64.0).max(280.0));
    let modal = egui::Modal::new(egui::Id::new("soos_onboarding_tour"))
        .frame(frame)
        .backdrop_color(Color32::from_rgba_unmultiplied(16, 24, 32, 90))
        .show(ctx, |ui| {
            ui.set_width(width);
            step_header(ui, tour);
            ui.add_space(12.0);
            // Motion hook (#377): page content is keyed by the tour generation.
            ui.push_id(("soos_tour_page", tour.generation()), |ui| {
                page(ui, tour.page(), view, &mut events);
            });
            if let Some((text, is_error)) = view.message {
                ui.add_space(8.0);
                let kind = if is_error {
                    BannerKind::Error
                } else {
                    BannerKind::Success
                };
                widgets::banner(ui, kind, text);
            }
            ui.add_space(16.0);
            footer(ui, tour, view.busy, &mut events);
        });
    // Escape or a click outside closes the tour, but never while an action is pending (the
    // modal keeps the rest of the window blocked until the answer arrives).
    if !view.busy && modal.should_close() && !events.contains(&TourEvent::Close) {
        events.push(TourEvent::Close);
    }
    events
}

fn step_header(ui: &mut egui::Ui, tour: &Tour) {
    let index = tour.index();
    let text = format!("Step {} of {}", index + 1, TOUR_PAGE_COUNT);
    ui.label(
        RichText::new(text)
            .size(theme::F_SMALL)
            .color(theme::INK_MUTED),
    );
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), theme::LINE);
    let fraction = (index + 1) as f32 / TOUR_PAGE_COUNT as f32;
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    painter.rect_filled(fill, CornerRadius::same(2), theme::BLUE);
}

fn body(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(14.0).color(theme::INK)).wrap());
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(theme::F_BODY)
                .color(theme::INK_MUTED),
        )
        .wrap(),
    );
}

fn page(ui: &mut egui::Ui, page: TourPage, view: &TourView<'_>, events: &mut Vec<TourEvent>) {
    match page {
        TourPage::Intro => {
            widgets::card_title(ui, "Welcome to soos");
            body(
                ui,
                "soos verifies your face on this PC and can let your phone see and control its \
                 lock state. The installer turned every feature on. The next steps explain each \
                 one, show its current state and let you turn it off here.",
            );
            ui.add_space(8.0);
            muted(
                ui,
                "Changes to system settings ask for an administrator password. You can reopen \
                 this tour at any time with the Features button in the header.",
            );
        }
        TourPage::Feature(feature) => {
            widgets::card_title(ui, feature.title());
            body(ui, feature.explanation());
            ui.add_space(6.0);
            muted(ui, feature.when_off());
            ui.add_space(14.0);
            feature_control(ui, feature, view, false, events);
        }
        TourPage::Overview => {
            widgets::card_title(ui, "Features");
            muted(
                ui,
                "Every feature with its current state. Open a step of the tour for the full \
                 explanation.",
            );
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .max_height((ui.ctx().content_rect().height() * 0.5).max(160.0))
                .show(ui, |ui| {
                    for feature in FEATURES {
                        feature_control(ui, feature, view, true, events);
                        ui.add_space(6.0);
                    }
                });
            ui.add_space(8.0);
            if ui
                .add_enabled(!view.busy, BrandButton::secondary("Replay the tour"))
                .clicked()
            {
                events.push(TourEvent::Restart);
            }
        }
    }
}

fn state_label(state: FeatureState, loaded: bool) -> (&'static str, Color32) {
    if !loaded {
        return ("Checking...", theme::INK_MUTED);
    }
    match state {
        FeatureState::On => ("On", theme::SUCCESS_TEXT),
        FeatureState::Off => ("Off", theme::INK),
        FeatureState::NotInstalled => ("Not installed", theme::INK_MUTED),
        FeatureState::NotAvailable => ("Not available in this version", theme::INK_MUTED),
        FeatureState::Unknown => ("Unknown", theme::WARN_TEXT),
    }
}

fn state_chip(ui: &mut egui::Ui, state: FeatureState, loaded: bool) {
    let (text, color) = state_label(state, loaded);
    egui::Frame::new()
        .fill(theme::PALE_2)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(theme::F_SMALL).color(color));
        });
}

fn command_block(ui: &mut egui::Ui, caption: &str, command: &str) {
    muted(ui, caption);
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(theme::PALE_2)
            .corner_radius(CornerRadius::same(theme::R_INPUT))
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - 90.0).max(120.0));
                ui.add(
                    egui::Label::new(
                        RichText::new(command)
                            .monospace()
                            .size(theme::F_SMALL)
                            .color(theme::INK),
                    )
                    .wrap()
                    .selectable(true),
                );
            });
        if ui.add(BrandButton::secondary("Copy")).clicked() {
            ui.ctx().copy_text(command.to_owned());
        }
    });
}

/// Paints the state and control of `feature`. `compact` is the overview row.
fn feature_control(
    ui: &mut egui::Ui,
    feature: Feature,
    view: &TourView<'_>,
    compact: bool,
    events: &mut Vec<TourEvent>,
) {
    let state = view.snapshot.state(feature);
    let loaded = view.loaded;
    let current_on = match state {
        FeatureState::On => Some(true),
        FeatureState::Off => Some(false),
        _ => None,
    };
    let switch_route = current_on.map(|on| toggle_route(view.snapshot, feature, !on));
    ui.horizontal(|ui| {
        if compact {
            ui.label(
                RichText::new(feature.title())
                    .size(theme::F_BODY)
                    .color(theme::INK),
            );
        } else {
            ui.label(
                RichText::new("Current state")
                    .size(theme::F_BODY)
                    .color(theme::INK_MUTED),
            );
        }
        state_chip(ui, state, loaded);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let (Some(on), Some(route)) = (current_on, switch_route.as_ref()) {
                let action = match route {
                    ToggleRoute::Action(action) => Some(*action),
                    _ => None,
                };
                let interactive = loaded && !view.busy && action.is_some();
                let (rect, _) = ui.allocate_exact_size(widgets::TOGGLE_SIZE, egui::Sense::hover());
                let id = egui::Id::new(("soos_tour_switch", feature, compact));
                let response = widgets::toggle_switch(ui, rect, id, Some(on), interactive);
                let hover = if on {
                    format!("Turn {} off", feature.title())
                } else {
                    format!("Turn {} on", feature.title())
                };
                if response.on_hover_text(hover).clicked() {
                    if let Some(action) = action {
                        events.push(TourEvent::Apply(action));
                    }
                }
            } else if loaded && state == FeatureState::Unknown && !compact {
                for (label, turn_on) in [("Turn off", false), ("Turn on", true)] {
                    if let ToggleRoute::Action(action) =
                        toggle_route(view.snapshot, feature, turn_on)
                    {
                        if ui
                            .add_enabled(!view.busy, BrandButton::secondary(label))
                            .clicked()
                        {
                            events.push(TourEvent::Apply(action));
                        }
                    }
                }
            }
        });
    });
    if compact || !loaded {
        return;
    }
    ui.add_space(6.0);
    match (state, switch_route) {
        (FeatureState::On | FeatureState::Off, Some(ToggleRoute::Command(command))) => {
            let caption = if current_on == Some(true) {
                "To turn it off, run or apply:"
            } else {
                "To turn it on, run or apply:"
            };
            command_block(ui, caption, &command);
        }
        (FeatureState::On | FeatureState::Off, Some(ToggleRoute::Unavailable(reason))) => {
            muted(ui, reason);
        }
        (FeatureState::Unknown, _) => {
            muted(ui, "The current state could not be read.");
            if let ToggleRoute::Command(command) = toggle_route(view.snapshot, feature, false) {
                command_block(ui, "To turn it off, run or apply:", &command);
            }
        }
        (FeatureState::NotInstalled, _) => match toggle_route(view.snapshot, feature, true) {
            ToggleRoute::Unavailable(reason) => muted(ui, reason),
            _ => muted(
                ui,
                "The phone companion is not installed for this user, so this feature is off.",
            ),
        },
        _ => {}
    }
}

fn footer(ui: &mut egui::Ui, tour: &Tour, busy: bool, events: &mut Vec<TourEvent>) {
    let last = tour.page() == TourPage::Overview;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 36.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let (label, event) = if last {
                ("Done", TourEvent::Close)
            } else {
                ("Next", TourEvent::Next)
            };
            let enabled = !busy || event == TourEvent::Next;
            if ui
                .add_enabled(enabled, BrandButton::primary(label))
                .clicked()
            {
                events.push(event);
            }
            if tour.index() > 0 && ui.add(BrandButton::secondary("Back")).clicked() {
                events.push(TourEvent::Back);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if !last
                    && ui
                        .add_enabled(!busy, BrandButton::secondary("Skip tour"))
                        .clicked()
                {
                    events.push(TourEvent::Close);
                }
            });
        },
    );
}
