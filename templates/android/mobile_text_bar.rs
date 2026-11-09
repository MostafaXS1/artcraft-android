//! Floating mobile text selection context menu for egui text fields.
//!
//! Provides a touch-friendly "Select all, Cut, Copy, Paste, Delete" bar that appears
//! above (or below) the currently focused text field on mobile/Android.
//! The "Delete" button appears ONLY when text is currently selected.
//! Uses the native egui visuals/styling of the host application.

use egui::{
    vec2, pos2, Align2, Area, Color32, Context, CornerRadius, Event, FontId, Id, Key, Modifiers,
    Order, Rect, Stroke, Ui, ViewportCommand,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    SelectAll,
    Cut,
    Copy,
    Paste,
    Delete,
}

/// Global clipboard provider hook for Android JNI clipboard integration.
static CLIPBOARD_GET: std::sync::RwLock<Option<fn() -> String>> = std::sync::RwLock::new(None);

pub fn set_clipboard_getter(getter: fn() -> String) {
    if let Ok(mut g) = CLIPBOARD_GET.write() {
        *g = Some(getter);
    }
}

fn get_clipboard_text() -> Option<String> {
    if let Ok(g) = CLIPBOARD_GET.read() {
        if let Some(getter) = *g {
            let text = getter();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Renders the mobile text context bar if a text field is currently focused.
/// Safe to call every frame inside the UI pass.
pub fn show_text_bar(ctx: &Context) {
    let Some(field_id) = ctx.memory(|m| m.focused()) else {
        return;
    };

    // Only display if the context actually wants keyboard input or is editing text
    if !ctx.egui_wants_keyboard_input() && ctx.output(|o| o.ime.is_none()) {
        return;
    }

    // Determine if text is currently selected
    let has_selection = egui::text_edit::TextEditState::load(ctx, field_id)
        .and_then(|state| state.cursor.char_range())
        .map_or(false, |range| !range.is_empty());

    // Determine field geometry (prefer IME output rect, fallback to response rect)
    let field_rect = ctx
        .output(|o| o.ime.as_ref().map(|i| i.rect))
        .or_else(|| ctx.read_response(field_id).map(|r| r.rect));

    let Some(field) = field_rect else {
        return;
    };

    // Build the list of actions: "Delete" appears ONLY when text is selected
    let mut actions = Vec::with_capacity(5);
    actions.push(("Select all", Action::SelectAll));
    if has_selection {
        actions.push(("Cut", Action::Cut));
    }
    actions.push(("Copy", Action::Copy));
    actions.push(("Paste", Action::Paste));
    if has_selection {
        actions.push(("Delete", Action::Delete));
    }

    let btn_w = 68.0f32;
    let h = 32.0f32;
    let gap = 6.0f32;
    let total_w = actions.len() as f32 * btn_w + (actions.len().saturating_sub(1)) as f32 * 2.0;

    let screen = ctx.content_rect();
    let x = (field.center().x - total_w / 2.0)
        .clamp(screen.left() + 4.0, (screen.right() - total_w - 4.0).max(screen.left() + 4.0));
    let y = if field.top() - h - gap >= screen.top() + 4.0 {
        field.top() - h - gap
    } else {
        field.bottom() + gap
    };

    let mut chosen = None;

    // Use egui's Order::Tooltip to float above everything else
    Area::new(Id::unique("artcraft_mobile_text_bar"))
        .order(Order::Tooltip)
        .fixed_pos(pos2(x, y))
        .show(ctx, |ui| {
            let visuals = ui.visuals().clone();
            let bg_color = visuals.window_fill();
            let stroke = visuals.window_stroke();
            let text_color = visuals.widgets.inactive.fg_stroke.color;
            let sep_color = visuals.widgets.noninteractive.bg_stroke.color;
            let corner_radius = 8.0f32;

            let bar_rect = Rect::from_min_size(pos2(x, y), vec2(total_w, h));
            ui.painter().rect(
                bar_rect,
                CornerRadius::same(corner_radius as u8),
                bg_color,
                stroke,
                egui::StrokeKind::Inside,
            );

            for (i, (label, action)) in actions.iter().enumerate() {
                let r = Rect::from_min_size(pos2(x + i as f32 * (btn_w + 2.0), y), vec2(btn_w, h));
                if i > 0 {
                    ui.painter().line_segment(
                        [pos2(r.left() - 1.0, r.top() + 6.0), pos2(r.left() - 1.0, r.bottom() - 6.0)],
                        Stroke::new(1.0, sep_color),
                    );
                }

                let id = ui.scope_id().with(("action", i));
                let resp = ui.interact(r, id, egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(
                        r.shrink(1.0),
                        CornerRadius::same(corner_radius as u8 / 2),
                        visuals.widgets.hovered.weak_bg_fill,
                    );
                }

                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    *label,
                    FontId::proportional(12.5),
                    text_color,
                );

                if resp.clicked() {
                    chosen = Some(*action);
                }
            }
        });

    if let Some(action) = chosen {
        // Prevent focus surrender on menu button clicks
        ctx.memory_mut(|m| m.request_focus(field_id));
        match action {
            Action::SelectAll => {
                ctx.input_mut(|i| {
                    i.events.push(Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    });
                    i.events.push(Event::Key {
                        key: Key::A,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::COMMAND,
                    });
                });
            }
            Action::Cut => {
                ctx.input_mut(|i| i.events.push(Event::Cut));
            }
            Action::Copy => {
                ctx.input_mut(|i| i.events.push(Event::Copy));
            }
            Action::Paste => {
                if let Some(text) = get_clipboard_text() {
                    ctx.input_mut(|i| i.events.push(Event::Paste(text)));
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
                }
            }
            Action::Delete => {
                ctx.input_mut(|i| {
                    i.events.push(Event::Key {
                        key: Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                    i.events.push(Event::Key {
                        key: Key::Backspace,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    });
                });
            }
        }
        ctx.request_repaint();
    }
}
