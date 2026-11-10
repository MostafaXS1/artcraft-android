//! Topbar modifiers disabled (--no-buttons build).
//! Stubs for Shift / Ctrl / Alt virtual buttons.

use egui::{Modifiers, Pos2, Ui};

pub fn is_any_active() -> bool {
    false
}

pub fn active_modifiers() -> Modifiers {
    Modifiers::default()
}

pub fn apply_to_raw_input(_physical: &Modifiers, _raw: &mut egui::RawInput) {}

pub fn render_topbar_modifiers(_ui: &mut Ui) {}

pub fn render_topbar_modifiers_right(_ui: &mut Ui) {}

pub fn render_topbar_modifiers_at(_ui: &mut Ui, _pos: Pos2) {}
