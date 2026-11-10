//! Mobile text bar disabled (--no-buttons build).
//! Stubs for Cut/Copy/Paste/Delete floating text bar.

use egui::Context;

pub fn set_clipboard_getter<F: Fn() -> String + Send + Sync + 'static>(_getter: F) {}

pub fn set_clipboard_setter<F: Fn(&str) + Send + Sync + 'static>(_setter: F) {}

pub fn process_raw_input(_ctx: &Context, _raw: &mut egui::RawInput) {}

pub fn show_text_bar(_ctx: &Context) {}
