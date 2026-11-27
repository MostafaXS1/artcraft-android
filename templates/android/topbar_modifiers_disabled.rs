//! Topbar modifiers disabled (--no-buttons build).
//! Stubs for configurable Undo / Redo / Cut / Copy / Paste / Find / Delete / Space / Shift / Ctrl / Alt buttons.

use egui::{Modifiers, Pos2, Ui};

pub const DEFAULT_MASK: u32 = 0;

pub fn hardware_keyboard_connected() -> bool {
    false
}

pub fn set_hardware_keyboard_connected(_connected: bool) {}

pub fn set_redo_uses_ctrl_y(_use_ctrl_y: bool) {}

pub fn set_initial_mask(_mask: u32) {}

pub fn set_mask_handler(_handler: impl Fn(u32) + Send + Sync + 'static) {}

pub fn is_any_active() -> bool {
    false
}

pub fn active_modifiers() -> Modifiers {
    Modifiers::default()
}

pub fn apply_to_raw_input(_physical: &Modifiers, _raw: &mut egui::RawInput) {}

pub fn modifier_bar_width_full() -> f32 {
    0.0
}

pub fn modifier_bar_width_compact() -> f32 {
    0.0
}

pub fn is_tight(_ctx: &egui::Context, _id: &str) -> bool {
    false
}

pub fn update_tight_measured(
    _ctx: &egui::Context,
    _id: &str,
    _bar_width: f32,
    _measured_left_w: Option<f32>,
    _right_reserved_w: f32,
) -> bool {
    false
}

pub fn update_tight(
    _ctx: &egui::Context,
    _id: &str,
    _bar_width: f32,
    _left_content_w: f32,
    _right_reserved_w: f32,
) -> bool {
    false
}

pub fn modifier_bar_width_for(_ctx: &egui::Context, _id: &str) -> f32 {
    0.0
}

pub fn modifier_bar_width(_ctx: &egui::Context) -> f32 {
    0.0
}

pub fn render_topbar_modifiers_for(_ui: &mut Ui, _compact: bool) {}

pub fn render_topbar_modifiers(_ui: &mut Ui) {}

pub fn render_topbar_modifiers_for_right(_ui: &mut Ui, _compact: bool) {}

pub fn render_topbar_modifiers_right(_ui: &mut Ui) {}

pub fn render_topbar_modifiers_at_for(_ui: &mut Ui, _pos: Pos2, _compact: bool) {}

pub fn render_topbar_modifiers_at(_ui: &mut Ui, _pos: Pos2) {}

pub fn settings_ui(_ui: &mut Ui) {}
