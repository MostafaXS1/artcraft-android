//! Floating mobile text selection context menu for egui text fields.
//!
//! Provides a touch-friendly "Select all, Cut, Copy, Paste, Delete" bar that appears
//! above (or below) the currently focused text field on mobile/Android.
//! Cut, Copy, and Delete buttons appear ONLY when text is currently selected.
//! Uses the native egui visuals/styling of the host application, with small outlines
//! around each button.

use egui::{
    pos2, vec2, Align2, Area, Context, CornerRadius, Event, FontId, Id, Key, Modifiers,
    Order, Rect, Stroke,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cut,
    Copy,
    Paste,
    Delete,
}

static BAR_BUTTONS: std::sync::Mutex<Vec<(Rect, Action)>> = std::sync::Mutex::new(Vec::new());
static ACTIVE_TOUCH: std::sync::Mutex<Option<(egui::TouchId, Action)>> = std::sync::Mutex::new(None);
static ACTIVE_POINTER: std::sync::Mutex<Option<Action>> = std::sync::Mutex::new(None);

pub static CLIPBOARD_GETTER: std::sync::RwLock<Option<Box<dyn Fn() -> String + Send + Sync>>> =
    std::sync::RwLock::new(None);
pub static CLIPBOARD_SETTER: std::sync::RwLock<Option<Box<dyn Fn(&str) + Send + Sync>>> =
    std::sync::RwLock::new(None);

pub fn set_clipboard_getter<F: Fn() -> String + Send + Sync + 'static>(getter: F) {
    if let Ok(mut g) = CLIPBOARD_GETTER.write() {
        *g = Some(Box::new(getter));
    }
}

pub fn set_clipboard_setter<F: Fn(&str) + Send + Sync + 'static>(setter: F) {
    if let Ok(mut s) = CLIPBOARD_SETTER.write() {
        *s = Some(Box::new(setter));
    }
}

fn get_clipboard_text() -> Option<String> {
    if let Ok(g) = CLIPBOARD_GETTER.read() {
        if let Some(getter) = g.as_ref() {
            let text = getter();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Intercepts touches and pointer events on the mobile text bar before egui sees them.
/// Consumes interactions with the bar so egui does NOT surrender focus on the active TextEdit,
/// and synthesizes the corresponding editing events directly into `raw.events`.
pub fn process_raw_input(ctx: &Context, raw: &mut egui::RawInput) {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() {
        return;
    }
    let buttons = {
        if let Ok(g) = BAR_BUTTONS.lock() {
            g.clone()
        } else {
            Vec::new()
        }
    };

    if buttons.is_empty() {
        return;
    }

    let mut tap_action = None;

    raw.events.retain(|event| {
        match event {
            Event::Touch { id, phase, pos, .. } => {
                if *phase == egui::TouchPhase::Start {
                    if let Some((_, action)) = buttons.iter().find(|(r, _)| r.contains(*pos)) {
                        if let Ok(mut touch) = ACTIVE_TOUCH.lock() {
                            *touch = Some((*id, *action));
                        }
                        return false;
                    }
                }

                let mut active = ACTIVE_TOUCH.lock();
                if let Ok(ref mut active_opt) = active {
                    if let Some((t_id, action)) = **active_opt {
                        if t_id == *id {
                            if *phase == egui::TouchPhase::End {
                                if buttons.iter().any(|(r, a)| *a == action && r.contains(*pos)) {
                                    tap_action = Some(action);
                                }
                                **active_opt = None;
                            } else if *phase == egui::TouchPhase::Cancel {
                                **active_opt = None;
                            }
                            return false;
                        }
                    }
                }
                true
            }
            Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } => {
                if *pressed {
                    if let Some((_, action)) = buttons.iter().find(|(r, _)| r.contains(*pos)) {
                        if let Ok(mut ptr) = ACTIVE_POINTER.lock() {
                            *ptr = Some(*action);
                        }
                        return false;
                    }
                } else {
                    let mut ptr = ACTIVE_POINTER.lock();
                    if let Ok(ref mut ptr_opt) = ptr {
                        if let Some(action) = ptr_opt.take() {
                            if buttons.iter().any(|(r, a)| *a == action && r.contains(*pos)) {
                                tap_action = Some(action);
                            }
                            return false;
                        }
                    }
                }
                true
            }
            Event::PointerMoved(_) => {
                if let Ok(ptr) = ACTIVE_POINTER.lock() {
                    if ptr.is_some() {
                        return false;
                    }
                }
                true
            }
            _ => true,
        }
    });

    if let Some(action) = tap_action {
        match action {
            Action::Cut => {
                raw.events.push(Event::Cut);
            }
            Action::Copy => {
                raw.events.push(Event::Copy);
            }
            Action::Paste => {
                if let Some(text) = get_clipboard_text() {
                    raw.events.push(Event::Paste(text));
                } else {
                    for pressed in [true, false] {
                        raw.events.push(Event::Key {
                            key: Key::V,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Modifiers::COMMAND,
                        });
                    }
                }
            }
            Action::Delete => {
                for pressed in [true, false] {
                    raw.events.push(Event::Key {
                        key: Key::Backspace,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
        }
        ctx.request_repaint();
    }
}

/// Renders the mobile text context bar if a text field is currently focused.
/// Safe to call every frame inside the UI pass.
pub fn show_text_bar(ctx: &Context) {
    if option_env!("ARTCRAFT_NO_BUTTONS").is_some() {
        return;
    }
    let Some(field_id) = ctx.memory(|m| m.focused()) else {
        if let Ok(mut b) = BAR_BUTTONS.lock() {
            b.clear();
        }
        return;
    };

    // Only display if the context actually wants keyboard input or is editing text
    if !ctx.egui_wants_keyboard_input() && ctx.output(|o| o.ime.is_none()) {
        if let Ok(mut b) = BAR_BUTTONS.lock() {
            b.clear();
        }
        return;
    }

    // Determine if text is currently selected
    let has_selection = egui::text_edit::TextEditState::load(ctx, field_id)
        .and_then(|state| state.cursor.char_range())
        .map_or(false, |range| !range.is_empty());

    // The context bar only appears when text is selected (Cut, Copy, Paste, Delete)
    if !has_selection {
        if let Ok(mut b) = BAR_BUTTONS.lock() {
            b.clear();
        }
        return;
    }

    // Determine field geometry (prefer IME output rect, fallback to response rect)
    let field_rect = ctx
        .output(|o| o.ime.as_ref().map(|i| i.rect))
        .or_else(|| ctx.read_response(field_id).map(|r| r.rect));

    let Some(field) = field_rect else {
        if let Ok(mut b) = BAR_BUTTONS.lock() {
            b.clear();
        }
        return;
    };

    // All 4 buttons appear on selected text
    let actions = [
        ("Cut", Action::Cut),
        ("Copy", Action::Copy),
        ("Paste", Action::Paste),
        ("Delete", Action::Delete),
    ];

    let btn_w = 70.0f32;
    let btn_h = 30.0f32;
    let pad = 4.0f32;
    let spacing = 4.0f32;
    let total_w = 2.0 * pad + actions.len() as f32 * btn_w + actions.len().saturating_sub(1) as f32 * spacing;
    let total_h = btn_h + 2.0 * pad;

    let screen = ctx.content_rect();
    let x = (field.center().x - total_w / 2.0)
        .clamp(screen.left() + 4.0, (screen.right() - total_w - 4.0).max(screen.left() + 4.0));
    let y = if field.top() - total_h - 6.0 >= screen.top() + 4.0 {
        field.top() - total_h - 6.0
    } else {
        field.bottom() + 6.0
    };

    let mut new_buttons = Vec::with_capacity(actions.len());

    // Use non-interactable Area with Order::Tooltip to float above everything without stealing focus
    Area::new(Id::new("artcraft_mobile_text_bar"))
        .order(Order::Tooltip)
        .fixed_pos(pos2(x, y))
        .interactable(false)
        .show(ctx, |ui| {
            let visuals = ui.visuals().clone();
            let bg_color = visuals.window_fill();
            let stroke = visuals.window_stroke();
            let text_color = visuals.widgets.inactive.fg_stroke.color;
            let btn_stroke = Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
            let btn_fill = visuals.widgets.inactive.weak_bg_fill;
            let btn_active_fill = visuals.widgets.active.bg_fill;

            let bar_rect = Rect::from_min_size(pos2(x, y), vec2(total_w, total_h));
            ui.painter().rect(
                bar_rect,
                CornerRadius::same(8),
                bg_color,
                stroke,
                egui::StrokeKind::Inside,
            );

            let active_action = {
                let t = ACTIVE_TOUCH.lock().ok().and_then(|g| g.map(|(_, a)| a));
                let p = ACTIVE_POINTER.lock().ok().and_then(|g| *g);
                t.or(p)
            };

            for (i, (label, action)) in actions.iter().enumerate() {
                let btn_x = x + pad + i as f32 * (btn_w + spacing);
                let btn_y = y + pad;
                let r = Rect::from_min_size(pos2(btn_x, btn_y), vec2(btn_w, btn_h));

                let fill = if active_action == Some(*action) {
                    btn_active_fill
                } else {
                    btn_fill
                };

                // Each button has a clean pill fill and a small outline
                ui.painter().rect(
                    r,
                    CornerRadius::same(5),
                    fill,
                    btn_stroke,
                    egui::StrokeKind::Inside,
                );

                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    *label,
                    FontId::proportional(12.5),
                    text_color,
                );

                new_buttons.push((r, *action));
            }
        });

    if let Ok(mut b) = BAR_BUTTONS.lock() {
        *b = new_buttons;
    }
}
