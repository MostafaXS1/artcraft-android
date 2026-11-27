//! Integrated configurable topbar action & modifier buttons for mobile / Android.
//!
//! Supports optional toggles configured in app Settings:
//!   Undo, Redo, Cut, Copy, Paste, Find (Ctrl+F), Delete, Space, Shift, Ctrl, Alt.
//! Only Shift / Ctrl / Alt are enabled by default.
//! Real part of the UI (not an overlay), responsive full-label vs compact icon-only mode.

use egui::{vec2, Color32, CornerRadius, Key, Modifiers, Pos2, Rect, Response, Stroke, Ui, WidgetText};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

const MODIFIER_FONT_BYTES: &[u8] = include_bytes!("modifier_symbols.ttf");
const TOPBAR_SYMBOLS1_BYTES: &[u8] = include_bytes!("topbar_symbols1.ttf");
const TOPBAR_SYMBOLS2_BYTES: &[u8] = include_bytes!("topbar_symbols2.ttf");
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
    // Tiny subsets (~3KB + ~7KB) containing only symbol glyphs (✂, ⎘, 📋, 🔍, ⌕) with zero
    // Latin characters, so Highest ensures crisp monochrome icons instead of OS color emoji.
    for (name, bytes) in [
        ("topbar-symbols-1", TOPBAR_SYMBOLS1_BYTES),
        ("topbar-symbols-2", TOPBAR_SYMBOLS2_BYTES),
    ] {
        ctx.add_font(egui::epaint::text::FontInsert::new(
            name,
            egui::FontData::from_static(bytes),
            vec![
                egui::epaint::text::InsertFontFamily {
                    family: egui::FontFamily::Proportional,
                    priority: egui::epaint::text::FontPriority::Highest,
                },
                egui::epaint::text::InsertFontFamily {
                    family: egui::FontFamily::Monospace,
                    priority: egui::epaint::text::FontPriority::Highest,
                },
            ],
        ));
    }
    ctx.request_repaint();
    FONT_INSTALLED.store(true, Ordering::Relaxed);
}

/// Whether a hardware keyboard is attached.
static HARDWARE_KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Whether Redo maps to `Ctrl+Y` (Office apps) instead of `Ctrl+Shift+Z` (Creative apps).
static REDO_CTRL_Y: AtomicBool = AtomicBool::new(false);

pub fn hardware_keyboard_connected() -> bool {
    HARDWARE_KEYBOARD.load(Ordering::Relaxed)
}

pub fn set_hardware_keyboard_connected(connected: bool) {
    HARDWARE_KEYBOARD.store(connected, Ordering::Relaxed);
}

pub fn set_redo_uses_ctrl_y(use_ctrl_y: bool) {
    REDO_CTRL_Y.store(use_ctrl_y, Ordering::Relaxed);
}

pub const BIT_UNDO: u32 = 1 << 0;
pub const BIT_REDO: u32 = 1 << 1;
pub const BIT_CUT: u32 = 1 << 2;
pub const BIT_COPY: u32 = 1 << 3;
pub const BIT_PASTE: u32 = 1 << 4;
pub const BIT_FIND: u32 = 1 << 5;
pub const BIT_DELETE: u32 = 1 << 6;
pub const BIT_SPACE: u32 = 1 << 7;
pub const BIT_SHIFT: u32 = 1 << 8;
pub const BIT_CTRL: u32 = 1 << 9;
pub const BIT_ALT: u32 = 1 << 10;

pub const DEFAULT_MASK: u32 = BIT_SHIFT | BIT_CTRL | BIT_ALT;

static ENABLED_MASK: AtomicU32 = AtomicU32::new(DEFAULT_MASK);
static MASK_HANDLER: Mutex<Option<Box<dyn Fn(u32) + Send + Sync>>> = Mutex::new(None);

pub fn enabled_mask() -> u32 {
    ENABLED_MASK.load(Ordering::Relaxed)
}

pub fn set_initial_mask(mask: u32) {
    ENABLED_MASK.store(mask, Ordering::Relaxed);
}

pub fn set_mask_handler(handler: impl Fn(u32) + Send + Sync + 'static) {
    if let Ok(mut guard) = MASK_HANDLER.lock() {
        *guard = Some(Box::new(handler));
    }
}

pub fn set_enabled_mask(mask: u32) {
    ENABLED_MASK.store(mask, Ordering::Relaxed);
    if (mask & BIT_SPACE) == 0 {
        SPACE_ACTIVE.store(false, Ordering::Relaxed);
    }
    if (mask & BIT_SHIFT) == 0 {
        SHIFT_ACTIVE.store(false, Ordering::Relaxed);
    }
    if (mask & BIT_CTRL) == 0 {
        CTRL_ACTIVE.store(false, Ordering::Relaxed);
    }
    if (mask & BIT_ALT) == 0 {
        ALT_ACTIVE.store(false, Ordering::Relaxed);
    }
    if let Ok(guard) = MASK_HANDLER.lock() {
        if let Some(h) = guard.as_ref() {
            h(mask);
        }
    }
}

pub static SPACE_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static SHIFT_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static CTRL_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static ALT_ACTIVE: AtomicBool = AtomicBool::new(false);

static SPACE_HELD_SENT: AtomicBool = AtomicBool::new(false);
static LAST_COMBINED: Mutex<Option<Modifiers>> = Mutex::new(None);

#[derive(Clone, Copy)]
enum PendingAction {
    Undo,
    Redo,
    Cut { text_mode: bool },
    Copy { text_mode: bool },
    Paste,
    Find,
    Delete,
}

static PENDING_ACTIONS: Mutex<Vec<PendingAction>> = Mutex::new(Vec::new());

struct TopbarButtonSpec {
    bit: u32,
    icon: &'static str,
    full_label: &'static str,
    settings_label: &'static str,
    est_full_w: f32,
}

const BUTTONS: [TopbarButtonSpec; 11] = [
    TopbarButtonSpec { bit: BIT_UNDO, icon: "↺", full_label: "↺ Undo", settings_label: "↺ Undo (Ctrl+Z)", est_full_w: 62.0 },
    TopbarButtonSpec { bit: BIT_REDO, icon: "↻", full_label: "↻ Redo", settings_label: "↻ Redo", est_full_w: 62.0 },
    TopbarButtonSpec { bit: BIT_CUT, icon: "✂", full_label: "✂ Cut", settings_label: "✂ Cut (Ctrl+X)", est_full_w: 54.0 },
    TopbarButtonSpec { bit: BIT_COPY, icon: "⎘", full_label: "⎘ Copy", settings_label: "⎘ Copy (Ctrl+C)", est_full_w: 62.0 },
    TopbarButtonSpec { bit: BIT_PASTE, icon: "📋", full_label: "📋 Paste", settings_label: "📋 Paste (Ctrl+V)", est_full_w: 66.0 },
    TopbarButtonSpec { bit: BIT_FIND, icon: "🔍", full_label: "🔍 Find", settings_label: "🔍 Find (Ctrl+F)", est_full_w: 58.0 },
    TopbarButtonSpec { bit: BIT_DELETE, icon: "⌦", full_label: "⌦ Del", settings_label: "⌦ Delete", est_full_w: 56.0 },
    TopbarButtonSpec { bit: BIT_SPACE, icon: "␣", full_label: "␣ Space", settings_label: "␣ Space (Hold)", est_full_w: 68.0 },
    TopbarButtonSpec { bit: BIT_SHIFT, icon: "⇧", full_label: "⇧ Shift", settings_label: "⇧ Shift", est_full_w: 64.0 },
    TopbarButtonSpec { bit: BIT_CTRL, icon: "⌃", full_label: "⌃ Ctrl", settings_label: "⌃ Ctrl", est_full_w: 60.0 },
    TopbarButtonSpec { bit: BIT_ALT, icon: "⌥", full_label: "⌥ Alt", settings_label: "⌥ Alt", est_full_w: 54.0 },
];

/// Returns whether any sticky topbar toggle is currently active.
pub fn is_any_active() -> bool {
    SPACE_ACTIVE.load(Ordering::Relaxed)
        || SHIFT_ACTIVE.load(Ordering::Relaxed)
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

fn push_key_tap(raw: &mut egui::RawInput, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        raw.events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        });
    }
}

/// Merges active topbar toggles and one-shot action presses into egui `RawInput` events.
/// Does NOT mutate `physical`, so toggling off a modifier immediately clears it.
pub fn apply_to_raw_input(physical: &Modifiers, raw: &mut egui::RawInput) {
    if raw.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false))) {
        SPACE_ACTIVE.store(false, Ordering::Relaxed);
    }

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

    // Sticky Space hold (e.g. Space+drag to pan canvas on touch)
    let want_space = SPACE_ACTIVE.load(Ordering::Relaxed);
    let sent_space = SPACE_HELD_SENT.load(Ordering::Relaxed);
    if want_space != sent_space {
        raw.events.push(egui::Event::Key {
            key: Key::Space,
            physical_key: Some(Key::Space),
            pressed: want_space,
            repeat: false,
            modifiers: combined,
        });
        SPACE_HELD_SENT.store(want_space, Ordering::Relaxed);
    }

    // Drain any queued one-shot action button clicks from the previous UI frame
    let actions = if let Ok(mut q) = PENDING_ACTIONS.lock() {
        if q.is_empty() {
            Vec::new()
        } else {
            std::mem::take(&mut *q)
        }
    } else {
        Vec::new()
    };

    let ctrl_mod = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    let ctrl_shift_mod = Modifiers {
        ctrl: true,
        command: true,
        shift: true,
        ..Modifiers::NONE
    };

    for action in actions {
        match action {
            PendingAction::Undo => push_key_tap(raw, Key::Z, ctrl_mod),
            PendingAction::Redo => {
                if REDO_CTRL_Y.load(Ordering::Relaxed) {
                    push_key_tap(raw, Key::Y, ctrl_mod);
                } else {
                    push_key_tap(raw, Key::Z, ctrl_shift_mod);
                }
            }
            PendingAction::Cut { text_mode } => {
                if text_mode {
                    raw.events.push(egui::Event::Cut);
                } else {
                    push_key_tap(raw, Key::X, ctrl_mod);
                }
            }
            PendingAction::Copy { text_mode } => {
                if text_mode {
                    raw.events.push(egui::Event::Copy);
                } else {
                    push_key_tap(raw, Key::C, ctrl_mod);
                }
            }
            PendingAction::Paste => push_key_tap(raw, Key::V, ctrl_mod),
            PendingAction::Find => push_key_tap(raw, Key::F, ctrl_mod),
            PendingAction::Delete => push_key_tap(raw, Key::Delete, Modifiers::NONE),
        }
    }
}

/// Legacy static compact check (narrow screen fallback when unmeasured).
pub fn is_compact(ctx: &egui::Context) -> bool {
    ctx.content_rect().width() < 640.0
}

fn bar_disabled() -> bool {
    option_env!("ARTCRAFT_NO_BUTTONS").is_some()
        || hardware_keyboard_connected()
        || enabled_mask() == 0
}

/// Width needed for enabled full text+icon buttons.
pub fn modifier_bar_width_full() -> f32 {
    if bar_disabled() {
        return 0.0;
    }
    let mask = enabled_mask();
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for b in &BUTTONS {
        if (mask & b.bit) != 0 {
            sum += b.est_full_w;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else if mask == DEFAULT_MASK {
        210.0
    } else {
        sum + 10.0 * (count.saturating_sub(1)) as f32 + 12.0
    }
}

/// Width needed for enabled compact square icon-only buttons.
pub fn modifier_bar_width_compact() -> f32 {
    if bar_disabled() {
        return 0.0;
    }
    let count = (enabled_mask() & ((1 << 11) - 1)).count_ones();
    if count == 0 {
        0.0
    } else {
        24.0 * count as f32 + 3.0 * (count.saturating_sub(1)) as f32 + 8.0
    }
}

/// Returns whether the given topbar (`id`) detected tight horizontal space on the last frame.
pub fn is_tight(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("topbar_mod_tight", id))))
        .unwrap_or(false)
}

/// Records measured natural width of the left bar and updates whether `id` lacks room for
/// left controls + `right_reserved_w` + enabled full text+icon buttons inside `bar_width`.
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
    let tight = mod_full > 0.0 && left_w > 0.0 && (left_w + right_reserved_w + mod_full + 12.0 > bar_width);
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

/// Width reserved for topbar buttons on the right side of the given topbar (`id`).
pub fn modifier_bar_width_for(ctx: &egui::Context, id: &str) -> f32 {
    if is_tight(ctx, id) {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    }
}

/// Width reserved for the topbar buttons on the right side of a topbar.
pub fn modifier_bar_width(ctx: &egui::Context) -> f32 {
    if is_compact(ctx) {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    }
}

fn queue_action(action: PendingAction, ctx: &egui::Context) {
    if let Ok(mut q) = PENDING_ACTIONS.lock() {
        q.push(action);
    }
    ctx.request_repaint();
}

fn render_spec_button(ui: &mut Ui, spec: &TopbarButtonSpec, compact: bool) {
    let label = if compact { spec.icon } else { spec.full_label };
    match spec.bit {
        BIT_SPACE => {
            render_toggle_button(ui, label, &SPACE_ACTIVE, compact);
        }
        BIT_SHIFT => {
            render_toggle_button(ui, label, &SHIFT_ACTIVE, compact);
        }
        BIT_CTRL => {
            render_toggle_button(ui, label, &CTRL_ACTIVE, compact);
        }
        BIT_ALT => {
            render_toggle_button(ui, label, &ALT_ACTIVE, compact);
        }
        _ => {
            if render_action_button(ui, spec.bit, label, compact).clicked() {
                let text_mode = ui.ctx().egui_wants_keyboard_input();
                let act = match spec.bit {
                    BIT_UNDO => PendingAction::Undo,
                    BIT_REDO => PendingAction::Redo,
                    BIT_CUT => PendingAction::Cut { text_mode },
                    BIT_COPY => PendingAction::Copy { text_mode },
                    BIT_PASTE => PendingAction::Paste,
                    BIT_FIND => PendingAction::Find,
                    BIT_DELETE => PendingAction::Delete,
                    _ => return,
                };
                queue_action(act, ui.ctx());
            }
        }
    }
}

/// Renders enabled topbar buttons in either full (`label`) or compact square icon-only mode.
pub fn render_topbar_modifiers_for(ui: &mut Ui, compact: bool) {
    if bar_disabled() {
        return;
    }
    ensure_modifier_fonts(ui.ctx());
    let mask = enabled_mask();
    let rtl = ui.layout().prefer_right_to_left();

    if compact {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.spacing_mut().button_padding = vec2(0.0, 0.0);
        ui.add_space(2.0);
    } else {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding = vec2(6.0, 3.0);
        ui.add_space(4.0);
    }

    if rtl {
        for spec in BUTTONS.iter().rev() {
            if (mask & spec.bit) != 0 {
                render_spec_button(ui, spec, compact);
            }
        }
    } else {
        for spec in &BUTTONS {
            if (mask & spec.bit) != 0 {
                render_spec_button(ui, spec, compact);
            }
        }
    }

    if compact {
        ui.add_space(2.0);
    } else {
        ui.add_space(4.0);
    }
}

/// Renders enabled topbar buttons inline, auto-detecting compact mode from available width.
pub fn render_topbar_modifiers(ui: &mut Ui) {
    let full_w = modifier_bar_width_full();
    let compact = is_compact(ui.ctx()) || (full_w > 0.0 && ui.available_width() < full_w);
    render_topbar_modifiers_for(ui, compact);
}

/// Renders enabled topbar buttons docked on the right side of a UI row.
pub fn render_topbar_modifiers_for_right(ui: &mut Ui, compact: bool) {
    if bar_disabled() {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        render_topbar_modifiers_for(ui, compact);
    });
}

/// Renders enabled topbar buttons docked on the right side of a UI row.
pub fn render_topbar_modifiers_right(ui: &mut Ui) {
    if bar_disabled() {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        render_topbar_modifiers(ui);
    });
}

/// Renders enabled topbar buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at_for(ui: &mut Ui, pos: Pos2, compact: bool) {
    let w = if compact {
        modifier_bar_width_compact()
    } else {
        modifier_bar_width_full()
    };
    if w <= 0.0 {
        return;
    }
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

/// Renders enabled topbar buttons at a specific coordinate inside a parent container.
pub fn render_topbar_modifiers_at(ui: &mut Ui, pos: Pos2) {
    render_topbar_modifiers_at_for(ui, pos, is_compact(ui.ctx()));
}

/// Settings panel UI allowing the user to toggle each of the 11 topbar buttons.
pub fn settings_ui(ui: &mut Ui) {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() {
        return;
    }
    ensure_modifier_fonts(ui.ctx());
    let mut mask = enabled_mask();
    let old = mask;
    egui::Grid::new("artcraft_topbar_buttons_settings")
        .num_columns(2)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for (i, spec) in BUTTONS.iter().enumerate() {
                let mut on = (mask & spec.bit) != 0;
                if ui.checkbox(&mut on, spec.settings_label).changed() {
                    if on {
                        mask |= spec.bit;
                    } else {
                        mask &= !spec.bit;
                    }
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
            if ui.button("Reset Defaults").clicked() {
                mask = DEFAULT_MASK;
            }
            ui.end_row();
        });
    if mask != old {
        set_enabled_mask(mask);
        ui.ctx().request_repaint();
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8
}

fn lerp_color_premul(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        lerp_u8(a.r(), b.r(), t),
        lerp_u8(a.g(), b.g(), t),
        lerp_u8(a.b(), b.b(), t),
        lerp_u8(a.a(), b.a(), t),
    )
}

fn render_action_button(ui: &mut Ui, bit: u32, label: &str, compact: bool) -> Response {
    let visuals = ui.visuals().clone();
    let anim_id = egui::Id::new(("topbar_action_highlight", bit));
    let now = ui.input(|i| i.time);
    let (was_down, flash_until) = ui
        .ctx()
        .data(|d| d.get_temp::<(bool, f64)>(anim_id))
        .unwrap_or((false, 0.0));

    let highlight_t = if was_down {
        1.0
    } else if now < flash_until {
        let rem = (flash_until - now) as f32;
        // Hold full active highlight briefly, then fade smoothly back to resting style.
        (rem / 0.12).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let (base_bg, base_fg, base_stroke) = if compact {
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

    let (bg_fill, fg_color, stroke) = if highlight_t > 0.0 {
        (
            lerp_color_premul(base_bg, visuals.selection.bg_fill, highlight_t),
            lerp_color_premul(base_fg, Color32::WHITE, highlight_t),
            if compact && highlight_t < 1.0 {
                Stroke::new(
                    base_stroke.width,
                    lerp_color_premul(base_stroke.color, Color32::TRANSPARENT, highlight_t),
                )
            } else {
                Stroke::NONE
            },
        )
    } else {
        (base_bg, base_fg, base_stroke)
    };

    let text = WidgetText::from(label).color(fg_color);
    let mut btn = egui::Button::new(text)
        .fill(bg_fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(4))
        .frame(compact || highlight_t > 0.0);
    if compact {
        btn = btn.min_size(vec2(24.0, 24.0));
    }

    let resp = ui.add(btn);
    let down_now = resp.is_pointer_button_down_on();
    let clicked_now = resp.clicked();
    let next_flash_until = if clicked_now {
        (now + 0.18).max(flash_until)
    } else if now >= flash_until {
        0.0
    } else {
        flash_until
    };

    if down_now != was_down || next_flash_until != flash_until || now < next_flash_until || down_now {
        ui.ctx()
            .data_mut(|d| d.insert_temp(anim_id, (down_now, next_flash_until)));
        if down_now != was_down || clicked_now || now < next_flash_until {
            ui.ctx().request_repaint();
        }
    }

    resp
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
