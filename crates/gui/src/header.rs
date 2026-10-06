//! Brand header: blue band with the wordmark, the folder-shaped tab bar and the daemon panel.
//!
//! [`DaemonSwitch`] is the pure model of the daemon toggle. It replaces the old Pause/Resume
//! buttons with the same semantics: a click submits exactly `PauseDaemon` (daemon active) or
//! `ResumeDaemon` (daemon paused); the switch is disabled while a privileged action is pending
//! and inert while the daemon state is unknown. The painting helpers below never block.

#![forbid(unsafe_code)]
#![allow(
    unknown_lints,
    clippy::arithmetic_side_effects,
    reason = "GUI coordinate calculations on bounded f32 values"
)]

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response,
    Sense, Stroke, Ui,
};

use crate::daemon_control::DaemonState;
use crate::privileged::PrivilegedAction;
use crate::theme::{self, paint_faux_bold};
use crate::widgets;

/// Tab labels, in tab order (the mockup typo "Enrollement" is fixed).
pub const TAB_LABELS: [&str; 3] = [
    "Live Model Diagnostic",
    "Guided Enrollment",
    "Biometric Profiles",
];

/// Vector icon drawn in a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabIcon {
    /// Magnifying glass (live diagnostics).
    Search,
    /// Person bust (guided enrollment).
    Person,
    /// Stacked sheets (biometric profiles).
    Profiles,
}

/// Pure view model of the daemon toggle in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonSwitch {
    /// Daemon state read lock-free from the background monitor.
    pub state: DaemonState,
    /// `Some(true)` = on (active), `Some(false)` = off (paused), `None` = indeterminate.
    pub position: Option<bool>,
    /// Whether a click is accepted (known state and no privileged action pending).
    pub interactive: bool,
    /// A privileged action is pending (spinner shown instead of the switch).
    pub busy: bool,
    /// Text shown next to the switch.
    pub label: &'static str,
}

impl DaemonSwitch {
    /// Builds the switch for `state`, with `busy` = a privileged action is pending.
    pub fn new(state: DaemonState, busy: bool) -> Self {
        let position = match state {
            DaemonState::Active => Some(true),
            DaemonState::Inactive => Some(false),
            DaemonState::Unknown => None,
        };
        let label = if busy && position.is_some() {
            "Waiting for authorization..."
        } else {
            match state {
                DaemonState::Active => "Daemon Active",
                DaemonState::Inactive => "Daemon Paused",
                DaemonState::Unknown => "Daemon status unknown",
            }
        };
        Self {
            state,
            position,
            interactive: position.is_some() && !busy,
            busy,
            label,
        }
    }

    /// The privileged action a click submits (same actions as the old Pause/Resume buttons).
    pub fn click_action(&self) -> Option<PrivilegedAction> {
        if !self.interactive {
            return None;
        }
        match self.state {
            DaemonState::Active => Some(PrivilegedAction::PauseDaemon),
            DaemonState::Inactive => Some(PrivilegedAction::ResumeDaemon),
            DaemonState::Unknown => None,
        }
    }

    /// Hover text of the switch.
    pub fn tooltip(&self) -> &'static str {
        match self.state {
            DaemonState::Active => "Pause soos-daemon",
            DaemonState::Inactive => "Resume soos-daemon",
            DaemonState::Unknown => "soos-daemon state not known yet",
        }
    }
}

/// Paints the blue header band and the pale body (rounded top corners) over `full`.
pub fn paint_chrome(painter: &egui::Painter, full: Rect) {
    painter.rect_filled(full, CornerRadius::ZERO, theme::BLUE);
    let body = Rect::from_min_max(
        pos2(full.min.x, full.min.y + theme::HEADER_H - 1.0),
        full.max,
    );
    painter.rect_filled(
        body,
        CornerRadius {
            nw: theme::BODY_TOP_RADIUS,
            ne: theme::BODY_TOP_RADIUS,
            sw: 0,
            se: 0,
        },
        theme::PALE,
    );
}

/// Paints a tab icon of `kind` inside the 18 x 18 box at `min`.
pub fn paint_tab_icon(
    painter: &egui::Painter,
    min: Pos2,
    kind: TabIcon,
    color: Color32,
    band: Color32,
) {
    let p = |x: f32, y: f32| min + vec2(x, y);
    match kind {
        TabIcon::Search => {
            painter.circle_stroke(p(7.5, 7.5), 5.2, Stroke::new(2.6, color));
            painter.line_segment([p(11.5, 11.5), p(16.0, 16.0)], Stroke::new(3.2, color));
            painter.circle_filled(p(16.0, 16.0), 1.6, color);
        }
        TabIcon::Person => {
            painter.circle_filled(p(9.0, 4.8), 4.3, color);
            painter.rect_filled(
                Rect::from_min_max(p(0.5, 10.5), p(17.5, 18.0)),
                CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 1,
                    se: 1,
                },
                color,
            );
        }
        TabIcon::Profiles => {
            painter.rect_filled(
                Rect::from_min_max(p(5.0, 1.5), p(17.0, 13.5)),
                CornerRadius::same(2),
                color.gamma_multiply(0.6),
            );
            let front = Rect::from_min_max(p(1.5, 5.0), p(13.5, 17.0));
            painter.rect_filled(front, CornerRadius::same(2), color);
            painter.line_segment(
                [
                    front.left_top() + vec2(0.0, 0.5),
                    front.right_top() + vec2(0.0, 0.5),
                ],
                Stroke::new(1.5, band),
            );
        }
    }
}

/// Horizontal paddings of a tab: `(left, icon gap, right, minimum width)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabSpacing {
    /// Padding before the icon.
    pub pad_l: f32,
    /// Gap between the icon and the label.
    pub gap: f32,
    /// Padding after the label.
    pub pad_r: f32,
    /// Minimum tab width.
    pub min_w: f32,
}

/// Full-size spacing (reference mockup).
pub const TAB_SPACING_FULL: TabSpacing = TabSpacing {
    pad_l: 16.0,
    gap: 20.0,
    pad_r: 26.0,
    min_w: 200.0,
};

/// Tight spacing for narrow windows.
pub const TAB_SPACING_TIGHT: TabSpacing = TabSpacing {
    pad_l: 14.0,
    gap: 10.0,
    pad_r: 16.0,
    min_w: 0.0,
};

/// Width of an icon-only tab.
pub const TAB_ICON_ONLY_W: f32 = 50.0;
const TAB_ICON: f32 = 18.0;
/// Concave fillet radius at the bottom of the active tab.
const TAB_FILLET: f32 = 14.0;

/// Width of a tab showing `label_w` points of label with `spacing`.
pub fn tab_width(label_w: f32, spacing: TabSpacing) -> f32 {
    (spacing.pad_l + TAB_ICON + spacing.gap + label_w + spacing.pad_r).max(spacing.min_w)
}

/// Lays out tabs from `x0` so they end before `x_max`.
///
/// Returns each tab's width and whether its label is shown. Tries the full mockup spacing,
/// then tight spacing, then icon-only inactive tabs.
pub fn layout_tabs(label_widths: &[f32], active: usize, x0: f32, x_max: f32) -> Vec<(f32, bool)> {
    let room = (x_max - x0).max(0.0);
    for spacing in [TAB_SPACING_FULL, TAB_SPACING_TIGHT] {
        let widths: Vec<(f32, bool)> = label_widths
            .iter()
            .map(|w| (tab_width(*w, spacing), true))
            .collect();
        if widths.iter().map(|w| w.0).sum::<f32>() <= room {
            return widths;
        }
    }
    label_widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == active {
                (tab_width(*w, TAB_SPACING_TIGHT), true)
            } else {
                (TAB_ICON_ONLY_W, false)
            }
        })
        .collect()
}

/// Paints one header tab in `rect` (top of the band to its bottom) and returns its response.
pub fn tab(
    ui: &Ui,
    rect: Rect,
    label: &str,
    icon: TabIcon,
    active: bool,
    show_label: bool,
    spacing: TabSpacing,
) -> Response {
    let id = ui.id().with(("soos_tab", label));
    let response = ui.interact(rect, id, Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(
            rect,
            CornerRadius {
                nw: 20,
                ne: 20,
                sw: 0,
                se: 0,
            },
            theme::PALE,
        );
        let y = rect.max.y - TAB_FILLET;
        // Concave fillets flaring the tab into the body.
        painter.rect_filled(
            Rect::from_min_max(
                pos2(rect.min.x - TAB_FILLET, y),
                pos2(rect.min.x, rect.max.y + 1.0),
            ),
            CornerRadius::ZERO,
            theme::PALE,
        );
        painter.circle_filled(pos2(rect.min.x - TAB_FILLET, y), TAB_FILLET, theme::BLUE);
        painter.rect_filled(
            Rect::from_min_max(
                pos2(rect.max.x, y),
                pos2(rect.max.x + TAB_FILLET, rect.max.y + 1.0),
            ),
            CornerRadius::ZERO,
            theme::PALE,
        );
        painter.circle_filled(pos2(rect.max.x + TAB_FILLET, y), TAB_FILLET, theme::BLUE);
    } else if response.hovered() {
        painter.rect_filled(
            rect.shrink2(vec2(6.0, 6.0)),
            CornerRadius::same(16),
            Color32::from_rgba_unmultiplied(237, 241, 255, 28),
        );
    }
    let (icon_color, text_color, band) = if active {
        (theme::BLUE, theme::INK, theme::PALE)
    } else {
        (theme::PALE, theme::PALE, theme::BLUE)
    };
    let icon_x = if show_label {
        rect.min.x + spacing.pad_l
    } else {
        rect.center().x - TAB_ICON / 2.0
    };
    let icon_min = pos2(icon_x, rect.center().y - TAB_ICON / 2.0);
    paint_tab_icon(painter, icon_min, icon, icon_color, band);
    if show_label {
        let pos = pos2(icon_x + TAB_ICON + spacing.gap, rect.center().y);
        let font = FontId::proportional(theme::F_TAB);
        let passes = 2;
        paint_faux_bold(
            painter,
            pos,
            Align2::LEFT_CENTER,
            label,
            font,
            text_color,
            passes,
            0.4,
        );
    }
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if show_label {
        response
    } else {
        response.on_hover_text(label)
    }
}

/// Paints the daemon panel (pale tab hanging from the top edge) with the label and the switch.
///
/// Returns the privileged action requested by a click on the switch, if any.
pub fn daemon_panel(ui: &mut Ui, rect: Rect, switch: &DaemonSwitch) -> Option<PrivilegedAction> {
    let painter = ui.painter().clone();
    painter.rect_filled(
        rect,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 20,
            se: 20,
        },
        theme::PALE,
    );
    let toggle_size = widgets::TOGGLE_SIZE;
    let toggle = Rect::from_min_size(
        pos2(
            rect.max.x - 20.0 - toggle_size.x,
            rect.center().y - toggle_size.y / 2.0,
        ),
        toggle_size,
    );
    let label_color = match (switch.busy, switch.state) {
        (true, _) => theme::INK,
        (false, DaemonState::Active) => theme::INK,
        (false, DaemonState::Inactive) => theme::WARN_TEXT,
        (false, DaemonState::Unknown) => theme::INK_MUTED,
    };
    let font = FontId::proportional(theme::F_TAB);
    let galley = painter.layout_no_wrap(switch.label.to_owned(), font.clone(), label_color);
    let group_w = galley.size().x + 16.0 + toggle_size.x;
    let label_x = (rect.center().x - group_w / 2.0 + 6.0)
        .min(toggle.min.x - 16.0 - galley.size().x)
        .max(rect.min.x + 16.0);
    paint_faux_bold(
        &painter,
        pos2(label_x, rect.center().y),
        Align2::LEFT_CENTER,
        switch.label,
        font,
        label_color,
        2,
        0.4,
    );

    if switch.busy {
        let spin = Rect::from_center_size(toggle.center(), vec2(18.0, 18.0));
        ui.put(spin, egui::Spinner::new().size(18.0).color(theme::BLUE));
        return None;
    }
    let id = ui.id().with("soos_daemon_switch");
    let response = widgets::toggle_switch(ui, toggle, id, switch.position, switch.interactive);
    let response = response.on_hover_text(switch.tooltip());
    if response.clicked() {
        switch.click_action()
    } else {
        None
    }
}
