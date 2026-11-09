//! Integrated toggleable Shift / Ctrl / Alt modifier buttons for the topbar / ribbon.
//!
//! Provides `⇧ Shift`, `⌃ Ctrl`, `⌥ Alt` toggle buttons integrated directly into the
//! topbar next to menu/tab buttons (File, Edit, etc.).
//! Real part of the UI (not an overlay), toggleable on/off, matching the app's native visuals.

use egui::{vec2, Color32, CornerRadius, Modifiers, Pos2, Rect, Response, Stroke, Ui, WidgetText};
use std::sync::atomic::{AtomicBool, Ordering};

pub static SHIFT_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static CTRL_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static ALT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Returns whether any topbar modifier is currently toggled on.
pub fn is_any_active() -> bool {
    SHIFT_ACTIVE.load(Ordering::Relaxed)
        || CTRL_ACTIVE.load(Ordering::Relaxed)
        || ALT_ACTIVE.load(Ordering::Relaxed)
}

/// Returns the current active modifier state from topbar toggles.
pub fn active_modifiers() -> Modifiers {
    let shift = SHIFT_ACTIVE.load(Ordering::Relaxed);
    let ctrl = CTRL_ACTIVE.load(Ordering::Relaxed);
    let alt = ALT_ACTIVE.load(Ordering::Relaxed);
    Modifiers {
        alt,
        ctrl,
        shift,
        mac_cmd: false,
        command: ctrl,
    }
}

/// Merges active topbar modifier toggles into an egui `RawInput` and modifier state.
pub fn apply_to_raw_input(state: &mut Modifiers, raw: &mut egui::RawInput) {
    let top = active_modifiers();
    let before = *state;

    state.shift |= top.shift;
    state.ctrl |= top.ctrl;
    state.command |= top.ctrl;
    state.alt |= top.alt;

    for event in &mut raw.events {
        match event {
            egui::Event::PointerButton { modifiers, .. }
            | egui::Event::MouseWheel { modifiers, .. }
            | egui::Event::Key { modifiers, .. } => {
                modifiers.shift |= top.shift;
                modifiers.ctrl |= top.ctrl;
                modifiers.command |= top.ctrl;
                modifiers.alt |= top.alt;
            }
            _ => {}
        }
    }

    if *state != before {
        raw.events.push(egui::Event::ModifiersChanged(*state));
    }
}

/// Renders the toggleable modifier buttons (`⇧ Shift`, `⌃ Ctrl`, `⌥ Alt`) inline in a UI row.
pub fn render_topbar_modifiers(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().button_padding = vec2(6.0, 3.0);

        render_toggle_button(ui, "⇧ Shift", &SHIFT_ACTIVE);
        render_toggle_button(ui, "⌃ Ctrl", &CTRL_ACTIVE);
        render_toggle_button(ui, "⌥ Alt", &ALT_ACTIVE);
    });
}

/// Renders the toggleable modifier buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at(ui: &mut Ui, pos: Pos2) {
    let rect = Rect::from_min_size(pos, vec2(220.0, 26.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        render_topbar_modifiers(ui);
    });
}

fn render_toggle_button(ui: &mut Ui, label: &str, state: &AtomicBool) -> Response {
    let active = state.load(Ordering::Relaxed);
    let visuals = ui.visuals().clone();

    let (bg_fill, fg_color, stroke) = if active {
        (
            visuals.selection.bg_fill,
            Color32::WHITE,
            Stroke::new(1.0, visuals.selection.stroke.color),
        )
    } else {
        (
            visuals.widgets.inactive.weak_bg_fill,
            visuals.widgets.inactive.fg_stroke.color,
            Stroke::NONE,
        )
    };

    let text = WidgetText::from(label).color(fg_color);
    let btn = egui::Button::new(text)
        .fill(bg_fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(4));

    let resp = ui.add(btn);
    if resp.clicked() {
        let new_state = !active;
        state.store(new_state, Ordering::Relaxed);
        ui.ctx().request_repaint();
    }
    resp
}
