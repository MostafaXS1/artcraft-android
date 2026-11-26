//! Integrated toggleable Shift / Ctrl / Alt modifier buttons for the topbar / ribbon.
//!
//! Provides `⇧ Shift`, `⌃ Ctrl`, `⌥ Alt` toggle buttons integrated directly into the
//! topbar next to menu/tab buttons (File, Edit, etc.).
//! Real part of the UI (not an overlay), toggleable on/off, matching the app's native visuals.

use egui::{vec2, Color32, CornerRadius, Modifiers, Pos2, Rect, Response, Stroke, Ui, WidgetText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const MODIFIER_FONT_BYTES: &[u8] = include_bytes!("modifier_symbols.ttf");
static FONT_INSTALLED: AtomicBool = AtomicBool::new(false);

fn ensure_modifier_fonts(ctx: &egui::Context) {
    if FONT_INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let already = ctx.fonts(|f| f.definitions().font_data.contains_key("modifier-symbols"));
    if already {
        FONT_INSTALLED.store(true, Ordering::Relaxed);
        return;
    }
    ctx.add_font(egui::epaint::text::FontInsert::new(
        "modifier-symbols",
        egui::FontData::from_static(MODIFIER_FONT_BYTES),
        vec![
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
            egui::epaint::text::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::epaint::text::FontPriority::Lowest,
            },
        ],
    ));
    ctx.request_repaint();
    FONT_INSTALLED.store(true, Ordering::Relaxed);
}

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

/// Legacy static compact check (narrow screen fallback when unmeasured).
pub fn is_compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < 640.0
}

/// Width needed for full text+icon buttons (`⇧ Shift  ⌃ Ctrl  ⌥ Alt`).
pub fn modifier_bar_width_full() -> f32 {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() || hardware_keyboard_connected() {
        0.0
    } else {
        210.0
    }
}

/// Width needed for compact square icon-only buttons (`⇧ ⌃ ⌥`).
pub fn modifier_bar_width_compact() -> f32 {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() || hardware_keyboard_connected() {
        0.0
    } else {
        86.0
    }
}

/// Returns whether the given topbar (`id`) detected tight horizontal space on the last frame.
/// Defaults to `false` so frame 1 measures natural unwrapped layout before compacting.
pub fn is_tight(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("topbar_mod_tight", id))))
        .unwrap_or(false)
}

/// Records measured natural width of the left bar (`measured_left_w` when unwrapped) and
/// updates whether `id` lacks room for left controls + `right_reserved_w` + full text+icon
/// modifier buttons inside `bar_width`.
pub fn update_tight_measured(
    ctx: &egui::Context,
    id: &str,
    bar_width: f32,
    measured_left_w: Option<f32>,
    right_reserved_w: f32,
) -> bool {
    let w_key = egui::Id::new(("topbar_mod_left_w", id));
    if let Some(lw) = measured_left_w.filter(|w| *w > 0.0) {
        ctx.data_mut(|d| d.insert_temp(w_key, lw));
    }
    let left_w = ctx
        .data(|d| d.get_temp::<f32>(w_key))
        .unwrap_or_else(|| measured_left_w.unwrap_or(0.0));
    let mod_full = modifier_bar_width_full();
    let tight = left_w > 0.0 && (left_w + right_reserved_w + mod_full + 12.0 > bar_width);
    let key = egui::Id::new(("topbar_mod_tight", id));
    let prev = ctx.data(|d| d.get_temp::<bool>(key));
    if prev != Some(tight) {
        ctx.data_mut(|d| d.insert_temp(key, tight));
        ctx.request_repaint();
    }
    tight
}

/// Convenience wrapper when `left_content_w` is always directly measurable on every frame.
pub fn update_tight(
    ctx: &egui::Context,
    id: &str,
    bar_width: f32,
    left_content_w: f32,
    right_reserved_w: f32,
) -> bool {
    update_tight_measured(ctx, id, bar_width, Some(left_content_w), right_reserved_w)
}

/// Width reserved for modifier buttons on the right side of the given topbar (`id`).
pub fn modifier_bar_width_for(ctx: &egui::Context, id: &str) -> f32 {
    if is_tight(ctx, id) {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    }
}

/// Width reserved for the modifier buttons on the right side of a topbar.
pub fn modifier_bar_width(ctx: &egui::Context) -> f32 {
    if is_compact(ctx) {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    }
}

/// Renders toggleable modifier buttons in either full (`⇧ Shift`, `⌃ Ctrl`, `⌥ Alt`) or
/// compact square icon-only (`⇧`, `⌃`, `⌥`) mode.
pub fn render_topbar_modifiers_for(ui: &mut Ui, compact: bool) {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() {
        return;
    }
    if hardware_keyboard_connected() {
        // A hardware keyboard is attached: it has the modifier keys, so these buttons would only
        // get in the way.
        return;
    }
    ensure_modifier_fonts(ui.ctx());

    if compact {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.spacing_mut().button_padding = vec2(0.0, 0.0);
        if ui.layout().prefer_right_to_left() {
            ui.add_space(2.0);
            render_toggle_button(ui, "⌥", &ALT_ACTIVE, true);
            render_toggle_button(ui, "⌃", &CTRL_ACTIVE, true);
            render_toggle_button(ui, "⇧", &SHIFT_ACTIVE, true);
            ui.add_space(2.0);
        } else {
            ui.add_space(2.0);
            render_toggle_button(ui, "⇧", &SHIFT_ACTIVE, true);
            render_toggle_button(ui, "⌃", &CTRL_ACTIVE, true);
            render_toggle_button(ui, "⌥", &ALT_ACTIVE, true);
            ui.add_space(2.0);
        }
    } else {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding = vec2(6.0, 3.0);

        if ui.layout().prefer_right_to_left() {
            ui.add_space(4.0);
            render_toggle_button(ui, "⌥ Alt", &ALT_ACTIVE, false);
            ui.add_space(4.0);
            render_toggle_button(ui, "⌃ Ctrl", &CTRL_ACTIVE, false);
            ui.add_space(4.0);
            render_toggle_button(ui, "⇧ Shift", &SHIFT_ACTIVE, false);
            ui.add_space(4.0);
        } else {
            ui.add_space(4.0);
            render_toggle_button(ui, "⇧ Shift", &SHIFT_ACTIVE, false);
            ui.add_space(4.0);
            render_toggle_button(ui, "⌃ Ctrl", &CTRL_ACTIVE, false);
            ui.add_space(4.0);
            render_toggle_button(ui, "⌥ Alt", &ALT_ACTIVE, false);
            ui.add_space(4.0);
        }
    }
}

/// Renders the toggleable modifier buttons inline, auto-detecting compact mode from available width.
pub fn render_topbar_modifiers(ui: &mut Ui) {
    let compact = is_compact(ui.ctx()) || ui.available_width() < 200.0;
    render_topbar_modifiers_for(ui, compact);
}

/// Renders the toggleable modifier buttons docked on the right side of a UI row.
pub fn render_topbar_modifiers_for_right(ui: &mut Ui, compact: bool) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        render_topbar_modifiers_for(ui, compact);
    });
}

/// Renders the toggleable modifier buttons docked on the right side of a UI row.
pub fn render_topbar_modifiers_right(ui: &mut Ui) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        render_topbar_modifiers(ui);
    });
}

/// Renders the toggleable modifier buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at_for(ui: &mut Ui, pos: Pos2, compact: bool) {
    let w = if compact {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    }
    .max(86.0);
    let rect = Rect::from_min_size(pos, vec2(w, 26.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            render_topbar_modifiers_for(ui, compact);
        },
    );
}

/// Renders the toggleable modifier buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at(ui: &mut Ui, pos: Pos2) {
    render_topbar_modifiers_at_for(ui, pos, is_compact(ui.ctx()));
}

fn render_toggle_button(ui: &mut Ui, label: &str, state: &AtomicBool, compact: bool) -> Response {
    let active = state.load(Ordering::Relaxed);
    let visuals = ui.visuals().clone();

    let (bg_fill, fg_color, stroke) = if active {
        (visuals.selection.bg_fill, Color32::WHITE, Stroke::NONE)
    } else if compact {
        (
            visuals.widgets.inactive.weak_bg_fill,
            visuals.widgets.inactive.fg_stroke.color,
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        )
    } else {
        (
            Color32::TRANSPARENT,
            visuals.widgets.inactive.fg_stroke.color,
            Stroke::NONE,
        )
    };

    let text = WidgetText::from(label).color(fg_color);
    let mut btn = egui::Button::new(text)
        .fill(bg_fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(4))
        .frame(active || compact);
    if compact {
        btn = btn.min_size(vec2(24.0, 24.0));
    }

    let resp = ui.add(btn);
    if resp.clicked() {
        let new_state = !active;
        state.store(new_state, Ordering::Relaxed);
        ui.ctx().request_repaint();
    }
    resp
}
