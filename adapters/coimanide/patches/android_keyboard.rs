//! The soft-keyboard button in the bottom bar.
//!
//! Coiman's editor and terminal draw their own text; on Android they publish an IME output
//! while they have focus, which is what lets the soft keyboard follow them. This button says
//! whether it may: On keeps it up, Off keeps it down, Auto lets the text surfaces decide.
//!
//! The button sits at the right end of the status bar (the bottom bar) and carries three modes:
//!
//! * `On` — the keyboard stays up; it comes back when it is dismissed.
//! * `Off` — the app never opens the keyboard by itself.
//! * `Auto` — the keyboard follows the text: it opens while the editor, the terminal or a real
//!   text field (the search popups) has focus, and goes away when none of them does.
//!
//! The mode is kept in Coiman's own settings folder, so it survives a restart.
//!
//! This module lives in Coiman's crate because the button is drawn inside Coiman's status bar.
//! The Android shell — which owns the Activity, and with it the soft keyboard — reads the mode
//! and the user's requests from here; it is the only part that can talk to the platform.

use crate::ui::theme;
use coiman_core::settings;
use egui::{Color32, FontId, RichText, Sense};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, Once};

/// What the keyboard button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The soft keyboard stays up.
    On,
    /// The app never opens the soft keyboard by itself.
    Off,
    /// The keyboard follows the app.
    Auto,
}

impl Mode {
    /// The word the button shows.
    pub fn label(self) -> &'static str {
        match self {
            Mode::On => "On",
            Mode::Off => "Off",
            Mode::Auto => "Auto",
        }
    }

    /// What is written to the settings file.
    fn as_str(self) -> &'static str {
        match self {
            Mode::On => "on",
            Mode::Off => "off",
            Mode::Auto => "auto",
        }
    }

    /// The mode a settings file names.
    fn from_str(text: &str) -> Option<Mode> {
        match text {
            "on" => Some(Mode::On),
            "off" => Some(Mode::Off),
            "auto" => Some(Mode::Auto),
            _ => None,
        }
    }

    /// The mode as the number the button keeps it in.
    fn code(self) -> u8 {
        match self {
            Mode::Auto => 0,
            Mode::On => 1,
            Mode::Off => 2,
        }
    }

    /// The mode a number stands for.
    fn from_code(code: u8) -> Mode {
        match code {
            1 => Mode::On,
            2 => Mode::Off,
            _ => Mode::Auto,
        }
    }
}

/// The mode the button is in. `Auto` until the settings say otherwise.
static MODE: AtomicU8 = AtomicU8::new(0);

/// The settings are read once, the first time the mode is asked for.
static LOADED: Once = Once::new();

/// What the button asked the Android shell to do with the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// Show the soft keyboard now.
    Show,
    /// Hide the soft keyboard now.
    Hide,
    /// Forget an earlier Show/Hide and let the mode decide again.
    Automatic,
    /// The editor or the terminal was tapped: in Auto, the keyboard comes back.
    TextTapped,
}

/// The request the button is waiting for the shell to carry out.
static REQUEST: Mutex<Option<Request>> = Mutex::new(None);

/// The mode the button is in.
pub fn mode() -> Mode {
    ensure_loaded();
    Mode::from_code(MODE.load(Ordering::Relaxed))
}

/// Puts the button in another mode, and remembers it for the next start.
pub fn set_mode(mode: Mode) {
    ensure_loaded();
    MODE.store(mode.code(), Ordering::Relaxed);
    save(mode);
}

/// What the button asked for, if anything.
pub fn take_request() -> Option<Request> {
    REQUEST.lock().ok().and_then(|mut request| request.take())
}

/// Asks the Android shell to show or hide the soft keyboard.
fn request(request: Request) {
    if let Ok(mut slot) = REQUEST.lock() {
        *slot = Some(request);
    }
}

/// Called when the editor or the terminal is tapped: in Auto, that brings the keyboard back
/// after the user dismissed it.
pub fn text_tapped() {
    request(Request::TextTapped);
}

/// The keyboard button, drawn at the right end of the bottom bar.
///
/// The button carries the keyboard's glyph and the mode it is in. Tapping it opens the menu
/// with the three modes and the two immediate actions.
pub fn button(ui: &mut egui::Ui) {
    ensure_loaded();
    let colors = theme::colors();
    let size = ui.spacing().interact_size.y.clamp(14.0, 22.0);
    let font = FontId::proportional((size * 0.52).round());
    let galley = ui
        .painter()
        .layout_no_wrap(mode().label().to_owned(), font, colors.status_fg);
    // A keyboard is wider than it is tall; give the icon that shape so the keys stay legible
    // even in the short status bar.
    let icon = egui::vec2((size * 1.1).max(15.0), (size * 0.75).max(10.0));
    let width = icon.x + 6.0 + galley.size().x;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, size), Sense::hover());
    // An id of its own: the menu that opens on a tap is remembered by the id of the widget that
    // opened it, and an automatic id follows the position, which moves with the status bar.
    let response = ui.interact(rect, ui.scope_id().with("keyboard_button"), Sense::click());

    let glyph = egui::Rect::from_center_size(
        egui::pos2(rect.left() + icon.x / 2.0, rect.center().y),
        icon,
    );
    paint_keyboard(ui.painter(), glyph, colors.status_fg, colors.status_bg);
    ui.painter().galley(
        egui::pos2(rect.left() + icon.x + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        Color32::TRANSPARENT,
    );

    // Built every frame, like every other popup: the menu remembers whether it is open in the
    // response's memory, and drawing it only on the click frame would make it vanish at once.
    menu(&response);
    let _ = response.on_hover_text("Soft keyboard");
}

/// The menu the button opens: the three modes, and the keyboard now.
fn menu(response: &egui::Response) {
    egui::Popup::menu(response)
        // Close as soon as a choice is made: the keyboard this menu opens would otherwise
        // cover the menu itself, leaving the rest of it out of reach.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.label(RichText::new("Soft keyboard").strong());
            ui.separator();
            for choice in [Mode::On, Mode::Off, Mode::Auto] {
                // A radio button, not a button: a button would take the keyboard focus away
                // from the editor, and the text the user types would go nowhere.
                if ui
                    .add(egui::RadioButton::new(mode() == choice, choice.label()))
                    .clicked()
                {
                    set_mode(choice);
                    // Picking a mode also undoes an earlier "Show now"/"Hide now": choosing
                    // `On` again after "Hide now" must bring the keyboard back.
                    match choice {
                        Mode::On => request(Request::Show),
                        Mode::Off => request(Request::Hide),
                        Mode::Auto => request(Request::Automatic),
                    }
                }
            }
            ui.separator();
            if ui.selectable_label(false, "Show now").clicked() {
                request(Request::Show);
            }
            if ui.selectable_label(false, "Hide now").clicked() {
                request(Request::Hide);
            }
        });
}

/// Paints a small keyboard in `rect`: a filled body with key slots and a wide space bar.
///
/// The slots are cut in the status bar's own colour, so the icon reads the same way the text
/// does, whatever the theme and the display scale.
fn paint_keyboard(painter: &egui::Painter, rect: egui::Rect, color: Color32, slot: Color32) {
    painter.rect_filled(rect, 2.5, color);
    let inner = rect.shrink(2.0);
    let gap = 1.0;
    let key_h = ((inner.height() - gap * 2.0) / 3.0).max(1.0);
    let columns = 4;
    let key_w = ((inner.width() - gap * (columns as f32 - 1.0)) / columns as f32).max(1.0);
    let key = |x: f32, y: f32, w: f32| {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, key_h))
    };
    for row in 0..2 {
        let y = inner.top() + row as f32 * (key_h + gap);
        for column in 0..columns {
            let x = inner.left() + column as f32 * (key_w + gap);
            painter.rect_filled(key(x, y, key_w), 0.6, slot);
        }
    }
    let y = inner.bottom() - key_h;
    let w = (inner.width() * 0.62).max(4.0);
    painter.rect_filled(key(inner.center().x - w / 2.0, y, w), 0.6, slot);
}

/// Where the mode is kept: in Coiman's own settings folder.
fn settings_path() -> Option<std::path::PathBuf> {
    settings::config_dir().map(|dir| dir.join("keyboard-mode"))
}

/// The mode the settings name, if they name one.
fn load() -> Option<Mode> {
    let path = settings_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    Mode::from_str(text.trim())
}

/// Keeps the mode for the next start.
fn save(mode: Mode) {
    let Some(path) = settings_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, mode.as_str());
}

/// Reads the mode from the settings the first time it is asked for.
fn ensure_loaded() {
    LOADED.call_once(|| {
        if let Some(saved) = load() {
            MODE.store(saved.code(), Ordering::Relaxed);
        }
    });
}
