//! Pair releases with the key that egui actually received on press.

use std::collections::HashMap;

use winit::keyboard::{NativeKeyCode, PhysicalKey};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Identity {
    Physical(PhysicalKey),
    // No scan code exists. Distinguish presses by their translated key, then
    // conservatively release all unidentified keys on an ambiguous key-up.
    Unidentified(egui::Key),
}

impl Identity {
    fn new(physical: PhysicalKey, logical: Option<egui::Key>) -> Option<Self> {
        if physical == PhysicalKey::Unidentified(NativeKeyCode::Unidentified) {
            logical.map(Self::Unidentified)
        } else {
            Some(Self::Physical(physical))
        }
    }
}

#[derive(Default)]
pub(super) struct HeldKeys {
    // None records a clipboard shortcut: no egui Key-down was emitted for it,
    // so its release must not clear another physical key's egui identity. If the
    // same physical key later repeats as an ordinary press (for example after
    // Ctrl is released while the key is still held), the marker is upgraded so
    // that press and its eventual release are paired normally.
    pressed: HashMap<Identity, Option<egui::Key>>,
}

impl HeldKeys {
    pub fn press(
        &mut self,
        physical: PhysicalKey,
        logical: Option<egui::Key>,
        emit_key: bool,
    ) -> Option<egui::Key> {
        use std::collections::hash_map::Entry;

        let identity = Identity::new(physical, logical)?;
        match self.pressed.entry(identity) {
            Entry::Occupied(mut entry) => {
                // A clipboard shortcut records `None` because it emitted no egui
                // Key-down. If the same physical key later produces an ordinary
                // press (e.g. Ctrl was released while the key is still held and
                // its repeat now carries normal text), upgrade the marker so the
                // ordinary press is emitted and its release can pair with it.
                if entry.get().is_none() && emit_key {
                    *entry.get_mut() = logical;
                }
                *entry.get()
            }
            Entry::Vacant(entry) => {
                let key = if emit_key { logical } else { None };
                entry.insert(key);
                key
            }
        }
    }

    /// Only release an egui key when its last physical alias goes up. An
    /// unmatched release (including a synthetic release after focus loss) has
    /// no authority to release a different held key.
    pub fn release(&mut self, physical: PhysicalKey) -> Vec<egui::Key> {
        let mut released = Vec::new();
        if physical == PhysicalKey::Unidentified(NativeKeyCode::Unidentified) {
            // Without any native identity, a layout switch makes exact pairing
            // impossible. Reset this ambiguous subset instead of sticking keys
            // down. Identified keys, including native scan codes, stay held.
            self.pressed.retain(|identity, key| {
                if matches!(identity, Identity::Unidentified(_)) {
                    if let Some(key) = key {
                        released.push(*key);
                    }
                    false
                } else {
                    true
                }
            });
        } else if let Some(Some(key)) = self.pressed.remove(&Identity::Physical(physical)) {
            released.push(key);
        }
        released.retain(|key| !self.pressed.values().any(|held| held.as_ref() == Some(key)));
        released.sort_unstable();
        released.dedup();
        released
    }

    pub fn clear(&mut self) {
        self.pressed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    use egui::{Event, Key, Modifiers, RawInput, ViewportId};
    use raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle};
    use winit::keyboard::KeyCode;

    /// A display target that deliberately has no native handle, so integration
    /// tests can construct a real [`State`] without opening a window or touching
    /// a real clipboard backend.
    struct NoDisplay;

    impl HasDisplayHandle for NoDisplay {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    fn test_state(ctx: egui::Context) -> State {
        State::new(ctx, ViewportId::ROOT, &NoDisplay, None, None, None)
    }

    fn feed_state(ctx: &egui::Context, state: &mut State) {
        feed(ctx, core::mem::take(&mut state.egui_input.events));
    }

    fn physical(key: KeyCode) -> PhysicalKey {
        PhysicalKey::Code(key)
    }

    fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn state_key_event(
        key: Key,
        physical_key: Option<Key>,
        pressed: bool,
        modifiers: Modifiers,
    ) -> Event {
        Event::Key {
            key,
            physical_key,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn feed(ctx: &egui::Context, events: Vec<Event>) {
        ctx.run_ui(
            RawInput {
                events,
                ..Default::default()
            },
            |_| {},
        )
        .drop_without_applying_deltas();
    }

    #[test]
    fn release_command_before_letter_releases_the_emitted_shortcut() {
        let ctx = egui::Context::default();
        let mut held = HeldKeys::default();
        let code = physical(KeyCode::KeyK);
        let key = held.press(code, Some(Key::K), true).unwrap();
        feed(&ctx, vec![key_event(key, true, Modifiers::COMMAND)]);
        assert!(ctx.input(|i| i.key_down(Key::K)));
        // Dvorak-QWERTY-Command now translates the same physical key to T.
        assert_eq!(held.press(code, Some(Key::T), true), Some(Key::K));
        let events = held
            .release(code)
            .into_iter()
            .map(|key| key_event(key, false, Modifiers::NONE))
            .collect();
        feed(&ctx, events);
        assert!(!ctx.input(|i| i.key_down(Key::K)));
        assert!(!ctx.input(|i| i.key_down(Key::T)));
        assert_eq!(held.press(code, Some(Key::T), true), Some(Key::T));
    }

    #[test]
    fn layout_switch_and_repeats_keep_press_identity_but_next_press_uses_new_layout() {
        let mut held = HeldKeys::default();
        let code = physical(KeyCode::KeyZ);
        assert_eq!(held.press(code, Some(Key::Z), true), Some(Key::Z));
        assert_eq!(held.press(code, Some(Key::Y), true), Some(Key::Z));
        assert_eq!(held.press(code, None, true), Some(Key::Z));
        assert_eq!(held.release(code), vec![Key::Z]);
        assert_eq!(held.press(code, Some(Key::Y), true), Some(Key::Y));
    }

    #[test]
    fn physical_aliases_remain_down_until_both_release() {
        let ctx = egui::Context::default();
        let mut held = HeldKeys::default();
        for code in [KeyCode::Enter, KeyCode::NumpadEnter] {
            let key = held.press(physical(code), Some(Key::Enter), true).unwrap();
            feed(&ctx, vec![key_event(key, true, Modifiers::NONE)]);
        }
        assert!(held.release(physical(KeyCode::Enter)).is_empty());
        feed(&ctx, vec![]);
        assert!(ctx.input(|i| i.key_down(Key::Enter)));
        let events = held
            .release(physical(KeyCode::NumpadEnter))
            .into_iter()
            .map(|key| key_event(key, false, Modifiers::NONE))
            .collect();
        feed(&ctx, events);
        assert!(!ctx.input(|i| i.key_down(Key::Enter)));
    }

    #[test]
    fn clipboard_shortcut_release_cannot_clear_another_keys_identity() {
        let mut held = HeldKeys::default();
        held.press(physical(KeyCode::KeyA), Some(Key::C), true);
        // Ctrl+C records a marker (no Key-down was emitted).
        assert_eq!(
            held.press(physical(KeyCode::KeyC), Some(Key::C), false),
            None
        );
        // Repeating the shortcut while modifiers are held stays a marker.
        assert_eq!(
            held.press(physical(KeyCode::KeyC), Some(Key::C), false),
            None
        );
        assert!(held.release(physical(KeyCode::KeyC)).is_empty());
        assert_eq!(held.release(physical(KeyCode::KeyA)), vec![Key::C]);
    }

    #[test]
    fn clipboard_marker_upgrades_when_a_repeat_becomes_an_ordinary_press() {
        let mut held = HeldKeys::default();
        let code = physical(KeyCode::KeyC);
        // Ctrl+C: no Key-down was emitted for this physical key.
        assert_eq!(held.press(code, Some(Key::C), false), None);
        // Ctrl is released while C is still physically held; the next repeat
        // carries normal text and must be emitted as a fresh ordinary press.
        assert_eq!(held.press(code, Some(Key::C), true), Some(Key::C));
        // The upgraded identity remains sticky across further layout changes.
        assert_eq!(held.press(code, Some(Key::X), true), Some(Key::C));
        // Release now pairs with the ordinary press.
        assert_eq!(held.release(code), vec![Key::C]);
        assert!(held.release(code).is_empty());
        // A later fresh press starts a new pair.
        assert_eq!(held.press(code, Some(Key::C), true), Some(Key::C));
    }

    #[test]
    fn focus_loss_and_ime_retirement_allow_a_fresh_press() {
        let ctx = egui::Context::default();
        let mut held = HeldKeys::default();
        let code = physical(KeyCode::KeyZ);
        held.press(code, Some(Key::Z), true);
        feed(&ctx, vec![key_event(Key::Z, true, Modifiers::CTRL)]);
        held.clear();
        feed(&ctx, vec![Event::WindowFocused(false)]);
        assert!(!ctx.input(|i| i.key_down(Key::Z)));
        assert!(held.release(code).is_empty());
        assert_eq!(held.press(code, Some(Key::Y), true), Some(Key::Y));
        // IME interception retires the exact key previously delivered to egui.
        assert_eq!(held.release(code), vec![Key::Y]);
        assert!(held.release(code).is_empty());
        assert_eq!(held.press(code, Some(Key::Z), true), Some(Key::Z));
    }

    #[test]
    fn unidentified_press_without_a_key_emits_nothing() {
        let mut held = HeldKeys::default();
        let unknown = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        assert_eq!(held.press(unknown, None, true), None);
        assert_eq!(held.press(unknown, None, false), None);
        assert!(held.release(unknown).is_empty());
    }

    #[test]
    fn release_helper_emits_nothing_for_an_unmatched_release() {
        let mut state = test_state(egui::Context::default());
        state.release_keyboard_input(physical(KeyCode::KeyQ));
        assert!(state.egui_input().events.is_empty());
    }

    #[test]
    fn release_helper_stamps_current_modifiers_and_physical_key() {
        let mut state = test_state(egui::Context::default());
        state.modifiers = Modifiers::COMMAND;
        let code = physical(KeyCode::KeyK);
        let _ = state.held_keys.press(code, Some(Key::K), true);
        state.release_keyboard_input(code);
        assert_eq!(
            state.egui_input().events,
            vec![state_key_event(
                Key::K,
                Some(Key::K),
                false,
                Modifiers::COMMAND
            )]
        );
    }

    #[test]
    fn release_helper_maps_native_unidentified_keys_without_a_physical_key() {
        let mut state = test_state(egui::Context::default());
        let native = PhysicalKey::Unidentified(NativeKeyCode::Xkb(1234));
        let _ = state.held_keys.press(native, Some(Key::A), true);
        state.release_keyboard_input(native);
        assert_eq!(
            state.egui_input().events,
            vec![state_key_event(Key::A, None, false, Modifiers::NONE)]
        );
    }

    #[test]
    fn clipboard_shortcut_emits_its_event_before_retiring_an_alias() {
        let mut state = test_state(egui::Context::default());
        state.modifiers = Modifiers::COMMAND;
        let a = physical(KeyCode::KeyA);
        let c = physical(KeyCode::KeyC);
        // A physical key already emitted C.
        let _ = state.held_keys.press(a, Some(Key::C), true);
        // Ctrl+C is a clipboard shortcut: the Copy event comes first.
        assert!(state.try_clipboard_shortcut(c, Some(Key::C)));
        assert_eq!(state.egui_input().events, vec![Event::Copy]);
        // Retiring the shortcut key's own (never emitted) identity must not
        // release C, which is still held by the other physical alias.
        state.release_keyboard_input(c);
        assert_eq!(state.egui_input().events, vec![Event::Copy]);
        // Only releasing the alias emits the key-up.
        state.release_keyboard_input(a);
        assert_eq!(
            state.egui_input().events,
            vec![
                Event::Copy,
                // The release stamps the releasing physical key (A), not the
                // egui identity that was emitted from it (C).
                state_key_event(Key::C, Some(Key::A), false, Modifiers::COMMAND),
            ]
        );
    }

    #[test]
    fn clipboard_marker_upgrade_is_emitted_and_paired_through_state() {
        let ctx = egui::Context::default();
        let mut state = test_state(ctx.clone());
        let code = physical(KeyCode::KeyC);
        state.modifiers = Modifiers::COMMAND;

        // Ctrl+C: the shortcut path records a None marker.
        assert!(state.try_clipboard_shortcut(code, Some(Key::C)));
        assert_eq!(state.egui_input().events, vec![Event::Copy]);
        feed_state(&ctx, &mut state);

        // Ctrl is released and the held key repeats with normal text.
        state.modifiers = Modifiers::NONE;
        assert_eq!(
            state.press_keyboard_input(code, Some(Key::C), Some(Key::C)),
            Some(Key::C)
        );
        assert_eq!(
            state.egui_input().events,
            vec![state_key_event(Key::C, Some(Key::C), true, Modifiers::NONE)]
        );
        feed_state(&ctx, &mut state);
        assert!(ctx.input(|i| i.key_down(Key::C)));

        // The release pairs with the upgraded ordinary press.
        state.release_keyboard_input(code);
        assert_eq!(
            state.egui_input().events,
            vec![state_key_event(
                Key::C,
                Some(Key::C),
                false,
                Modifiers::NONE
            )]
        );
        feed_state(&ctx, &mut state);
        assert!(!ctx.input(|i| i.key_down(Key::C)));

        // A fresh press after release starts a new pair.
        assert_eq!(
            state.press_keyboard_input(code, Some(Key::C), Some(Key::C)),
            Some(Key::C)
        );
    }

    #[test]
    fn repeats_through_state_keep_the_first_identity_and_count_once_per_frame() {
        let ctx = egui::Context::default();
        let mut state = test_state(ctx.clone());
        let code = physical(KeyCode::KeyZ);

        assert_eq!(
            state.press_keyboard_input(code, Some(Key::Z), Some(Key::Z)),
            Some(Key::Z)
        );
        feed_state(&ctx, &mut state);
        assert!(ctx.input(|i| i.key_down(Key::Z)));
        assert_eq!(ctx.input(|i| i.num_presses(Key::Z)), 1);

        // A layout change on repeat keeps the originally emitted identity.
        assert_eq!(
            state.press_keyboard_input(code, Some(Key::Y), Some(Key::Y)),
            Some(Key::Z)
        );
        let repeats: Vec<bool> = state
            .egui_input()
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    ..
                } if *key == Key::Z => Some(*repeat),
                _ => None,
            })
            .collect();
        assert_eq!(repeats, vec![false], "egui fills in `repeat` itself");
        feed_state(&ctx, &mut state);
        assert!(ctx.input(|i| i.key_down(Key::Z)));
        assert_eq!(ctx.input(|i| i.num_presses(Key::Z)), 1);
        assert!(ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Z,
                        pressed: true,
                        repeat: true,
                        ..
                    }
                )
            })
        }));

        state.release_keyboard_input(code);
        feed_state(&ctx, &mut state);
        assert!(!ctx.input(|i| i.key_down(Key::Z)));
    }

    #[test]
    fn unidentified_release_resets_only_ambiguous_keys() {
        let mut held = HeldKeys::default();
        let unknown = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        let native = PhysicalKey::Unidentified(NativeKeyCode::Xkb(1234));
        held.press(native, Some(Key::A), true);
        held.press(unknown, Some(Key::A), true);
        held.press(unknown, Some(Key::B), true);
        // A remains held by its identified native alias.
        assert_eq!(held.release(unknown), vec![Key::B]);
        assert_eq!(held.release(native), vec![Key::A]);
        assert!(held.release(unknown).is_empty());
    }
}
