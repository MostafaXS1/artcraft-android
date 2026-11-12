//! Integrated toggleable Shift / Ctrl / Alt modifier buttons for the topbar / ribbon.
//!
//! Provides `⇧ Shift`, `⌃ Ctrl`, `⌥ Alt` toggle buttons integrated directly into the
//! topbar next to menu/tab buttons (File, Edit, etc.).
//! Real part of the UI (not an overlay), toggleable on/off, matching the app's native visuals.

use egui::{vec2, Color32, CornerRadius, Modifiers, Pos2, Rect, Response, Stroke, Ui, WidgetText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Whether a hardware keyboard is attached.
///
/// With a keyboard the soft keyboard is not needed, and buttons that stand in for it (the
/// Shift/Ctrl/Alt toggles) only get in the way: the keyboard has those keys.
///
/// This module lives in the app's own crate, which is also built for the desktop and cannot
/// depend on Android, so the launcher tells it the state (see `set_hardware_keyboard_connected`).
static HARDWARE_KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Whether a hardware keyboard is attached.
pub fn hardware_keyboard_connected() -> bool {
    HARDWARE_KEYBOARD.load(Ordering::Relaxed)
}

/// Tells the module whether a hardware keyboard is attached. The launcher calls this.
pub fn set_hardware_keyboard_connected(connected: bool) {
    HARDWARE_KEYBOARD.store(connected, Ordering::Relaxed);
}

pub static SHIFT_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static CTRL_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static ALT_ACTIVE: AtomicBool = AtomicBool::new(false);

static LAST_COMBINED: Mutex<Option<Modifiers>> = Mutex::new(None);

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

/// Merges active topbar modifier toggles into egui `RawInput` events.
/// Does NOT mutate `physical`, so toggling off a modifier immediately clears it.
pub fn apply_to_raw_input(physical: &Modifiers, raw: &mut egui::RawInput) {
    let top = active_modifiers();
    let combined = Modifiers {
        alt: physical.alt || top.alt,
        ctrl: physical.ctrl || top.ctrl,
        shift: physical.shift || top.shift,
        mac_cmd: false,
        command: physical.command || top.ctrl,
    };

    for event in &mut raw.events {
        match event {
            egui::Event::PointerButton { modifiers, .. }
            | egui::Event::MouseWheel { modifiers, .. }
            | egui::Event::Key { modifiers, .. } => {
                modifiers.alt |= combined.alt;
                modifiers.ctrl |= combined.ctrl;
                modifiers.shift |= combined.shift;
                modifiers.mac_cmd |= combined.mac_cmd;
                modifiers.command |= combined.command;
            }
            _ => {}
        }
    }

    let mut changed = false;
    if let Ok(mut last) = LAST_COMBINED.lock() {
        if *last != Some(combined) {
            *last = Some(combined);
            changed = true;
        }
    }

    if changed {
        raw.events.push(egui::Event::ModifiersChanged(combined));
    }
}

/// Renders the toggleable modifier buttons (`⇧ Shift`, `⌃ Ctrl`, `⌥ Alt`) inline into the current UI row.
pub fn render_topbar_modifiers(ui: &mut Ui) {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() {
        return;
    }
    if hardware_keyboard_connected() {
        // A hardware keyboard is attached: it has the modifier keys, so these buttons would only
        // get in the way.
        return;
    }
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.spacing_mut().button_padding = vec2(6.0, 3.0);

    if ui.layout().prefer_right_to_left() {
        ui.add_space(4.0);
        render_toggle_button(ui, "⌥ Alt", &ALT_ACTIVE);
        ui.add_space(4.0);
        render_toggle_button(ui, "⌃ Ctrl", &CTRL_ACTIVE);
        ui.add_space(4.0);
        render_toggle_button(ui, "⇧ Shift", &SHIFT_ACTIVE);
        ui.add_space(4.0);
    } else {
        ui.add_space(4.0);
        render_toggle_button(ui, "⇧ Shift", &SHIFT_ACTIVE);
        ui.add_space(4.0);
        render_toggle_button(ui, "⌃ Ctrl", &CTRL_ACTIVE);
        ui.add_space(4.0);
        render_toggle_button(ui, "⌥ Alt", &ALT_ACTIVE);
        ui.add_space(4.0);
    }
}

/// Renders the toggleable modifier buttons docked on the right side of a UI row.
pub fn render_topbar_modifiers_right(ui: &mut Ui) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        render_topbar_modifiers(ui);
    });
}

/// Renders the toggleable modifier buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at(ui: &mut Ui, pos: Pos2) {
    let rect = Rect::from_min_size(pos, vec2(210.0, 26.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            render_topbar_modifiers(ui);
        },
    );
}

fn render_toggle_button(ui: &mut Ui, label: &str, state: &AtomicBool) -> Response {
    let active = state.load(Ordering::Relaxed);
    let visuals = ui.visuals().clone();

    let (bg_fill, fg_color) = if active {
        (
            visuals.selection.bg_fill,
            Color32::WHITE,
        )
    } else {
        (
            Color32::TRANSPARENT,
            visuals.widgets.inactive.fg_stroke.color,
        )
    };

    let text = WidgetText::from(label).color(fg_color);
    let btn = egui::Button::new(text)
        .fill(bg_fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(4))
        .frame(active);

    let resp = ui.add(btn);
    if resp.clicked() {
        let new_state = !active;
        state.store(new_state, Ordering::Relaxed);
        ui.ctx().request_repaint();
    }
    resp
}
