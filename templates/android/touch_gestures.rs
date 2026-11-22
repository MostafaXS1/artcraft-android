//! Multi-touch gesture recognizer for Android:
//! - 2-finger pan/scroll: Holding 2 fingers and dragging pans/scrolls in any direction.
//! - 2-finger tap: Undo (Ctrl + Z)
//! - 3-finger tap: Redo (Ctrl + Shift + Z)
//! - Pinch-to-zoom protection: suppresses tool/brush strokes while 2 or more fingers are down.

use egui::{Event, Key, Modifiers, Pos2, TouchPhase, Vec2};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TAP_MAX_DURATION: Duration = Duration::from_millis(350);
const TAP_SLOP: f32 = 24.0; // Max movement in points to be considered a tap

struct ActiveFinger {
    start_pos: Pos2,
    last_pos: Pos2,
}

#[derive(Default)]
struct GestureState {
    fingers: HashMap<u64, ActiveFinger>,
    sequence_start: Option<Instant>,
    max_fingers: usize,
    moved: bool,
    was_multi: bool,
    prev_midpoint: Option<Pos2>,
}

static STATE: Mutex<Option<GestureState>> = Mutex::new(None);

/// Processes raw input events:
/// 1. Detects 2-finger hold-and-drag panning gestures in any direction.
/// 2. Detects quick 2-finger and 3-finger taps and injects Ctrl+Z (Undo) and Ctrl+Shift+Z (Redo).
/// 3. Suppresses brush/tool clicks while 2 or more fingers are touching (pinch-zoom protection).
pub fn process_raw_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let mut state_guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let state = state_guard.get_or_insert_with(Default::default);

    let mut trigger_tap: Option<usize> = None;
    let mut pan_delta: Option<Vec2> = None;

    for event in &raw.events {
        match event {
            Event::Touch { id, phase, pos, .. } => {
                let touch_key = id.0;
                match phase {
                    TouchPhase::Start => {
                        if state.fingers.is_empty() {
                            state.sequence_start = Some(Instant::now());
                            state.max_fingers = 1;
                            state.moved = false;
                            state.was_multi = false;
                            state.prev_midpoint = None;
                        } else {
                            state.max_fingers = state.max_fingers.max(state.fingers.len() + 1);
                            if state.fingers.len() + 1 >= 2 {
                                state.was_multi = true;
                            }
                        }
                        state.fingers.insert(
                            touch_key,
                            ActiveFinger {
                                start_pos: *pos,
                                last_pos: *pos,
                            },
                        );
                        if state.fingers.len() == 2 {
                            let pts: Vec<Pos2> = state.fingers.values().map(|f| f.last_pos).collect();
                            state.prev_midpoint = Some(Pos2::new(
                                (pts[0].x + pts[1].x) * 0.5,
                                (pts[0].y + pts[1].y) * 0.5,
                            ));
                        } else if state.fingers.len() > 2 {
                            state.prev_midpoint = None;
                        }
                    }
                    TouchPhase::Move => {
                        if let Some(f) = state.fingers.get_mut(&touch_key) {
                            f.last_pos = *pos;
                            if f.start_pos.distance(*pos) > TAP_SLOP {
                                state.moved = true;
                            }
                        }
                        if state.fingers.len() == 2 {
                            let pts: Vec<Pos2> = state.fingers.values().map(|f| f.last_pos).collect();
                            let curr_midpoint = Pos2::new(
                                (pts[0].x + pts[1].x) * 0.5,
                                (pts[0].y + pts[1].y) * 0.5,
                            );
                            if let Some(prev) = state.prev_midpoint {
                                let delta = curr_midpoint - prev;
                                if delta.length_sq() > 0.25 {
                                    pan_delta = Some(Vec2::new(delta.x, delta.y));
                                }
                            }
                            state.prev_midpoint = Some(curr_midpoint);
                        }
                    }
                    TouchPhase::End => {
                        if let Some(f) = state.fingers.remove(&touch_key) {
                            if f.start_pos.distance(*pos) > TAP_SLOP {
                                state.moved = true;
                            }
                        }
                        if state.fingers.len() < 2 {
                            state.prev_midpoint = None;
                        }
                        if state.fingers.is_empty() {
                            let duration = state
                                .sequence_start
                                .map(|t| t.elapsed())
                                .unwrap_or(Duration::from_secs(1));
                            if !state.moved && duration <= TAP_MAX_DURATION && state.max_fingers >= 2 {
                                trigger_tap = Some(state.max_fingers);
                            }
                            state.sequence_start = None;
                            state.max_fingers = 0;
                            state.moved = false;
                            state.was_multi = false;
                            state.prev_midpoint = None;
                        }
                    }
                    TouchPhase::Cancel => {
                        state.fingers.remove(&touch_key);
                        if state.fingers.len() < 2 {
                            state.prev_midpoint = None;
                        }
                        if state.fingers.is_empty() {
                            state.sequence_start = None;
                            state.max_fingers = 0;
                            state.moved = false;
                            state.was_multi = false;
                            state.prev_midpoint = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Pinch-to-zoom protection:
    // While 2 or more fingers are down (or during a multi-finger tap gesture),
    // suppress ALL pointer button events so painting/drawing tools and UI buttons
    // do not emit stray marks or accidental clicks.
    let is_multi_touch = state.fingers.len() >= 2 || state.was_multi;
    if is_multi_touch {
        raw.events.retain(|event| !matches!(event, Event::PointerButton { .. }));
        if let Some(mid) = state.prev_midpoint {
            raw.events.push(Event::PointerMoved(mid));
        }
    }

    // 2-finger panning: emit mouse wheel scroll events with Trackpad source for fluid navigation
    if let Some(delta) = pan_delta {
        raw.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
            modifiers: Modifiers::NONE,
        });
        ctx.request_repaint();
    }

    // Trigger Undo or Redo shortcuts
    if let Some(fingers) = trigger_tap {
        match fingers {
            2 => {
                // 2-finger tap: Undo (Ctrl + Z)
                let mods = Modifiers {
                    alt: false,
                    ctrl: true,
                    shift: false,
                    mac_cmd: false,
                    command: true,
                };
                raw.events.push(Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: true,
                    repeat: false,
                    modifiers: mods,
                });
                raw.events.push(Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: false,
                    repeat: false,
                    modifiers: mods,
                });
                ctx.request_repaint();
            }
            3 => {
                // 3-finger tap: Redo (Ctrl + Shift + Z)
                let mods = Modifiers {
                    alt: false,
                    ctrl: true,
                    shift: true,
                    mac_cmd: false,
                    command: true,
                };
                raw.events.push(Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: true,
                    repeat: false,
                    modifiers: mods,
                });
                raw.events.push(Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: false,
                    repeat: false,
                    modifiers: mods,
                });
                ctx.request_repaint();
            }
            _ => {}
        }
    }
}
