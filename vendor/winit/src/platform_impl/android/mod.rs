use std::cell::Cell;
use std::collections::VecDeque;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use android_activity::input::{
    Axis, Button, Classification, InputEvent, KeyAction, Keycode, MotionAction, ToolType,
};
use android_activity::{
    AndroidApp, AndroidAppWaker, ConfigurationRef, InputStatus, MainEvent, Rect,
};
use tracing::{debug, trace, warn};

use crate::cursor::Cursor;
use crate::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use crate::error;
use crate::error::EventLoopError;
use crate::event::{self, Force, InnerSizeWriter, StartCause};
use crate::event_loop::{self, ActiveEventLoop as RootAEL, ControlFlow, DeviceEvents};
use crate::platform::pump_events::PumpStatus;
use crate::platform_impl::Fullscreen;
use crate::window::{
    self, CursorGrabMode, CustomCursor, CustomCursorSource, ImePurpose, ResizeDirection, Theme,
    WindowButtons, WindowLevel,
};

mod device_kind;
mod keycodes;

pub(crate) use crate::cursor::{
    NoCustomCursor as PlatformCustomCursor, NoCustomCursor as PlatformCustomCursorSource,
};
pub(crate) use crate::icon::NoIcon as PlatformIcon;

static HAS_FOCUS: AtomicBool = AtomicBool::new(true);

/// Returns the minimum `Option<Duration>`, taking into account that `None`
/// equates to an infinite timeout, not a zero timeout (so can't just use
/// `Option::min`)
fn min_timeout(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    a.map_or(b, |a_timeout| {
        b.map_or(Some(a_timeout), |b_timeout| Some(a_timeout.min(b_timeout)))
    })
}

/// Points a smooth scroll amount is worth per axis unit. egui's line scroll speed is 40 points per
/// line and a mouse notch is worth a line, so an amount that is counted in notches is converted
/// with the same number.
///
/// A hardware touchpad does not need it: Android reports its gesture in pixels, and those are
/// the pixels the content moves by.
const POINTS_PER_AXIS: f64 = 40.0;

/// How far the two fingers' midpoint must move (physical pixels) before a two-finger gesture
/// counts as a pan (a scroll) rather than staying undecided.
const PAN_DEADZONE_PX: f64 = 12.0;

/// How far the two fingers' spread must change (physical pixels) before a two-finger gesture can
/// be a pinch (a zoom) rather than a pan.
const PINCH_DEADZONE_PX: f64 = 36.0;

/// How much more the two fingers' spread must change than their midpoint moves for a pinch to win
/// the gesture. A pan holds the fingers a fixed distance apart while the pair slides; a pinch
/// mostly changes the distance. Well above 1, so a slightly uneven drag still scrolls.
const PINCH_SPREAD_RATIO: f64 = 1.5;

/// How much faster a two-finger pan scrolls than the fingers move.
const PAN_SPEED: f64 = 1.5;

fn pointer_position(pointer: &android_activity::input::Pointer<'_>) -> PhysicalPosition<f64> {
    PhysicalPosition {
        x: pointer.x() as f64,
        y: pointer.y() as f64,
    }
}

/// The distance between the first two pointers of a motion event, if it has two.
fn pointer_spread(motion_event: &android_activity::input::MotionEvent<'_>) -> Option<f64> {
    let mut pointers = motion_event.pointers().map(|pointer| pointer_position(&pointer));
    let first = pointers.next()?;
    let second = pointers.next()?;
    Some(((first.x - second.x).powi(2) + (first.y - second.y).powi(2)).sqrt())
}

/// Every finger currently reported by `motion_event`, as (id, position).
fn collect_touch_points(
    motion_event: &android_activity::input::MotionEvent<'_>,
) -> Vec<(i32, PhysicalPosition<f64>)> {
    motion_event
        .pointers()
        .map(|pointer| (pointer.pointer_id(), pointer_position(&pointer)))
        .collect()
}

/// The fingers still on the glass after `motion_event`: an `Up` event carries the finger that is
/// leaving as well, and that one is gone.
fn remaining_touch_points(
    motion_event: &android_activity::input::MotionEvent<'_>,
) -> Vec<(i32, PhysicalPosition<f64>)> {
    let mut points = collect_touch_points(motion_event);
    if matches!(
        motion_event.action(),
        MotionAction::Up | MotionAction::PointerUp
    ) {
        let leaving = motion_event
            .pointer_at_index(motion_event.pointer_index())
            .pointer_id();
        points.retain(|(id, _)| *id != leaving);
    }
    points
}

fn android_mouse_button(button: android_activity::input::Button) -> event::MouseButton {
    use android_activity::input::Button;
    match button {
        Button::Primary => event::MouseButton::Left,
        Button::Secondary => event::MouseButton::Right,
        Button::Tertiary => event::MouseButton::Middle,
        Button::Back => event::MouseButton::Back,
        Button::Forward => event::MouseButton::Forward,
        // Not mappable to a named button; egui ignores `Other` buttons anyway.
        _ => event::MouseButton::Other(0),
    }
}

struct PeekableReceiver<T> {
    recv: mpsc::Receiver<T>,
    first: Option<T>,
}

impl<T> PeekableReceiver<T> {
    pub fn from_recv(recv: mpsc::Receiver<T>) -> Self {
        Self { recv, first: None }
    }

    pub fn has_incoming(&mut self) -> bool {
        if self.first.is_some() {
            return true;
        }
        match self.recv.try_recv() {
            Ok(v) => {
                self.first = Some(v);
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                warn!("Channel was disconnected when checking incoming");
                false
            }
        }
    }

    pub fn try_recv(&mut self) -> Result<T, mpsc::TryRecvError> {
        if let Some(first) = self.first.take() {
            return Ok(first);
        }
        self.recv.try_recv()
    }
}

#[derive(Clone)]
struct SharedFlagSetter {
    flag: Arc<AtomicBool>,
}
impl SharedFlagSetter {
    pub fn set(&self) -> bool {
        self.flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }
}

struct SharedFlag {
    flag: Arc<AtomicBool>,
}

// Used for queuing redraws from arbitrary threads. We don't care how many
// times a redraw is requested (so don't actually need to queue any data,
// we just need to know at the start of a main loop iteration if a redraw
// was queued and be able to read and clear the state atomically)
impl SharedFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn setter(&self) -> SharedFlagSetter {
        SharedFlagSetter {
            flag: self.flag.clone(),
        }
    }

    pub fn get_and_reset(&self) -> bool {
        self.flag.swap(false, std::sync::atomic::Ordering::AcqRel)
    }
}

#[derive(Clone)]
pub struct RedrawRequester {
    flag: SharedFlagSetter,
    waker: AndroidAppWaker,
}

impl RedrawRequester {
    fn new(flag: &SharedFlag, waker: AndroidAppWaker) -> Self {
        RedrawRequester {
            flag: flag.setter(),
            waker,
        }
    }

    pub fn request_redraw(&self) {
        if self.flag.set() {
            // Only explicitly try to wake up the main loop when the flag
            // value changes
            self.waker.wake();
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct KeyEventExtra {}

pub struct EventLoop<T: 'static> {
    pub(crate) android_app: AndroidApp,
    window_target: event_loop::ActiveEventLoop,
    redraw_flag: SharedFlag,
    user_events_sender: mpsc::Sender<T>,
    user_events_receiver: PeekableReceiver<T>, // must wake looper whenever something gets sent
    loop_running: bool,                        // Dispatched `NewEvents<Init>`
    running: bool,
    pending_redraw: bool,
    cause: StartCause,
    ignore_volume_keys: bool,
    combining_accent: Option<char>,
    /// Active touch points (finger id -> position in physical pixels), for the two-finger
    /// gesture recognition below. Empty when no fingers are down.
    touch_points: Vec<(i32, PhysicalPosition<f64>)>,
    /// What the current multi-finger gesture has been recognised as.
    gesture: Gesture,
    /// The fingers' spread and midpoint where the gesture started: the deadzone is measured
    /// against them.
    gesture_origin: Option<(f64, PhysicalPosition<f64>)>,
    /// The spread at the last reported pinch step, so each zoom is relative to the last one.
    gesture_spread: Option<f64>,
    /// The finger that was already on the glass when a second one joined it: the one whose
    /// touch has to be cancelled when the gesture takes over.
    gesture_pointer: Option<(i32, PhysicalPosition<f64>)>,
    /// Where a hardware touchpad's gesture was when its last event came: the gesture is
    /// reported as a pointer that walks across the screen, and the distance it walked is
    /// the scroll.
    pad_position: Option<PhysicalPosition<f64>>,
    /// How far apart a hardware touchpad's pinch fingers were when its last event came:
    /// the pinch is reported as two pointers that move apart or together.
    pad_spread: Option<f64>,
}

/// What a multi-finger gesture has been recognised as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gesture {
    /// Not decided yet: the fingers are still inside the deadzone.
    Undecided,
    /// A two-finger pan: reported as scrolling.
    Pan,
    /// A pinch: reported as a zoom.
    Pinch,
}

/// What a hardware touchpad is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PadGesture {
    /// Not a gesture: the events are ordinary pointer events.
    None,
    /// A two-finger slide: reported as scrolling.
    Scroll,
    /// A two-finger pinch: reported as a zoom.
    Pinch,
}

/// What the backend made of a multi-finger gesture.
enum GestureStep {
    /// Not a two-finger gesture, or still inside the deadzone: the fingers are reported as
    /// touches.
    None,
    /// A pan: the two fingers' midpoint moved by this much (physical pixels).
    Pan(PhysicalPosition<f64>),
    /// A pinch: the fingers spread (or closed) by this factor since the last step.
    Pinch(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlatformSpecificEventLoopAttributes {
    pub(crate) android_app: Option<AndroidApp>,
    pub(crate) ignore_volume_keys: bool,
}

impl Default for PlatformSpecificEventLoopAttributes {
    fn default() -> Self {
        Self {
            android_app: Default::default(),
            ignore_volume_keys: true,
        }
    }
}

impl<T: 'static> EventLoop<T> {
    pub(crate) fn new(
        attributes: &PlatformSpecificEventLoopAttributes,
    ) -> Result<Self, EventLoopError> {
        let (user_events_sender, user_events_receiver) = mpsc::channel();

        let android_app = attributes.android_app.as_ref().expect(
            "An `AndroidApp` as passed to android_main() is required to create an `EventLoop` on \
             Android",
        );
        // Enable the scroll axes for the GameActivity backend: unlike the
        // NativeActivity InputQueue, GameActivity only fills `axisValues` for
        // axes enabled here (X and Y by default), so the mouse wheel reads 0.
        android_app.enable_motion_axis(Axis::Vscroll);
        android_app.enable_motion_axis(Axis::Hscroll);
        let redraw_flag = SharedFlag::new();

        Ok(Self {
            android_app: android_app.clone(),
            window_target: event_loop::ActiveEventLoop {
                p: ActiveEventLoop {
                    app: android_app.clone(),
                    control_flow: Cell::new(ControlFlow::default()),
                    exit: Cell::new(false),
                    redraw_requester: RedrawRequester::new(
                        &redraw_flag,
                        android_app.create_waker(),
                    ),
                },
                _marker: PhantomData,
            },
            redraw_flag,
            user_events_sender,
            user_events_receiver: PeekableReceiver::from_recv(user_events_receiver),
            loop_running: false,
            running: false,
            pending_redraw: false,
            cause: StartCause::Init,
            ignore_volume_keys: attributes.ignore_volume_keys,
            combining_accent: None,
            touch_points: Vec::new(),
            gesture: Gesture::Undecided,
            gesture_origin: None,
            gesture_spread: None,
            gesture_pointer: None,
            pad_position: None,
            pad_spread: None,
        })
    }

    fn single_iteration<F>(&mut self, main_event: Option<MainEvent<'_>>, callback: &mut F)
    where
        F: FnMut(event::Event<T>, &RootAEL),
    {
        trace!("Mainloop iteration");

        let cause = self.cause;
        let mut pending_redraw = self.pending_redraw
            || matches!(
                cause,
                StartCause::Init | StartCause::Poll | StartCause::ResumeTimeReached { .. }
            );
        let mut resized = false;

        callback(event::Event::NewEvents(cause), self.window_target());

        if let Some(event) = main_event {
            trace!("Handling main event {:?}", event);

            match event {
                MainEvent::InitWindow { .. } => {
                    callback(event::Event::Resumed, self.window_target());
                }
                MainEvent::TerminateWindow { .. } => {
                    callback(event::Event::Suspended, self.window_target());
                }
                MainEvent::WindowResized { .. } => resized = true,
                MainEvent::RedrawNeeded { .. } => pending_redraw = true,
                MainEvent::ContentRectChanged { .. } => {
                    warn!("TODO: find a way to notify application of content rect change");
                }
                MainEvent::GainedFocus => {
                    HAS_FOCUS.store(true, Ordering::Relaxed);
                    callback(
                        event::Event::WindowEvent {
                            window_id: window::WindowId(WindowId),
                            event: event::WindowEvent::Focused(true),
                        },
                        self.window_target(),
                    );
                }
                MainEvent::LostFocus => {
                    HAS_FOCUS.store(false, Ordering::Relaxed);
                    callback(
                        event::Event::WindowEvent {
                            window_id: window::WindowId(WindowId),
                            event: event::WindowEvent::Focused(false),
                        },
                        self.window_target(),
                    );
                }
                MainEvent::ConfigChanged { .. } => {
                    let monitor = MonitorHandle::new(self.android_app.clone());
                    let old_scale_factor = monitor.scale_factor();
                    let scale_factor = monitor.scale_factor();
                    if (scale_factor - old_scale_factor).abs() < f64::EPSILON {
                        let new_inner_size = Arc::new(Mutex::new(
                            MonitorHandle::new(self.android_app.clone()).size(),
                        ));
                        let event = event::Event::WindowEvent {
                            window_id: window::WindowId(WindowId),
                            event: event::WindowEvent::ScaleFactorChanged {
                                inner_size_writer: InnerSizeWriter::new(Arc::downgrade(
                                    &new_inner_size,
                                )),
                                scale_factor,
                            },
                        };
                        callback(event, self.window_target());
                    }
                }
                MainEvent::LowMemory => {
                    callback(event::Event::MemoryWarning, self.window_target());
                }
                MainEvent::Start => {
                    // XXX: how to forward this state to applications?
                    warn!("TODO: forward onStart notification to application");
                }
                MainEvent::Resume { .. } => {
                    debug!("App Resumed - is running");
                    self.running = true;
                }
                MainEvent::SaveState { .. } => {
                    // XXX: how to forward this state to applications?
                    // XXX: also how do we expose state restoration to apps?
                    warn!("TODO: forward saveState notification to application");
                }
                MainEvent::Pause => {
                    debug!("App Paused - stopped running");
                    self.running = false;
                }
                MainEvent::Stop => {
                    // XXX: how to forward this state to applications?
                    warn!("TODO: forward onStop notification to application");
                }
                MainEvent::Destroy => {
                    // XXX: maybe exit mainloop to drop things before being
                    // killed by the OS?
                    warn!("TODO: forward onDestroy notification to application");
                }
                MainEvent::InsetsChanged { .. } => {
                    // XXX: how to forward this state to applications?
                    warn!("TODO: handle Android InsetsChanged notification");
                }
                unknown => {
                    trace!("Unknown MainEvent {unknown:?} (ignored)");
                }
            }
        } else {
            trace!("No main event to handle");
        }

        // temporarily decouple `android_app` from `self` so we aren't holding
        // a borrow of `self` while iterating
        let android_app = self.android_app.clone();

        // Process input events
        match android_app.input_events_iter() {
            Ok(mut input_iter) => loop {
                let read_event =
                    input_iter.next(|event| self.handle_input_event(&android_app, event, callback));

                if !read_event {
                    break;
                }
            },
            Err(err) => {
                tracing::warn!("Failed to get input events iterator: {err:?}");
            }
        }

        // Empty the user event buffer
        {
            while let Ok(event) = self.user_events_receiver.try_recv() {
                callback(crate::event::Event::UserEvent(event), self.window_target());
            }
        }

        if self.running {
            if resized {
                let size = if let Some(native_window) = self.android_app.native_window().as_ref() {
                    let width = native_window.width() as _;
                    let height = native_window.height() as _;
                    PhysicalSize::new(width, height)
                } else {
                    PhysicalSize::new(0, 0)
                };
                let event = event::Event::WindowEvent {
                    window_id: window::WindowId(WindowId),
                    event: event::WindowEvent::Resized(size),
                };
                callback(event, self.window_target());
            }

            pending_redraw |= self.redraw_flag.get_and_reset();
            if pending_redraw {
                pending_redraw = false;
                let event = event::Event::WindowEvent {
                    window_id: window::WindowId(WindowId),
                    event: event::WindowEvent::RedrawRequested,
                };
                callback(event, self.window_target());
            }
        }

        // This is always the last event we dispatch before poll again
        callback(event::Event::AboutToWait, self.window_target());

        self.pending_redraw = pending_redraw;
    }

    /// Recognise a multi-finger gesture and report it the way a trackpad does.
    ///
    /// A phone or tablet has no wheel, so a two-finger drag is the platform's scroll gesture (as
    /// on a trackpad). Upstream winit forwards each finger as a separate `Touch` event and
    /// nothing turns the gesture into scrolling, so a two-finger drag did nothing.
    ///
    /// Chromium makes the same decision for a trackpad, and the WebView build of WordCraft
    /// inherits it: a two-finger slide is a scroll (a `DOM_DELTA_PIXEL` wheel event, applied 1:1)
    /// and a two-finger pinch is a zoom (a ctrl-wheel event, which `eframe` turns into
    /// `egui::Event::Zoom`). Android hands us the raw finger positions instead, so the backend
    /// has to make that call — and it has to make it *once*: a gesture that flips from scroll to
    /// zoom halfway through leaves the document half zoomed, and a gesture egui reads as a pinch
    /// (it computes a zoom from the distance between two fingers) turns every scroll into a slow
    /// zoom.
    ///
    /// So the gesture is decided once, inside a deadzone, and then sticks until the fingers lift:
    /// a pan that moves the fingers' midpoint further than [`PAN_DEADZONE_PX`] scrolls, a gesture
    /// that changes their spread further than [`PINCH_DEADZONE_PX`] (and much further than it
    /// moves the midpoint) zooms, and anything in between does nothing at all.
    fn recognise_touch_gesture(
        &mut self,
        motion_event: &android_activity::input::MotionEvent<'_>,
    ) -> GestureStep {
        match motion_event.action() {
            MotionAction::Down
            | MotionAction::PointerDown
            | MotionAction::Up
            | MotionAction::PointerUp
            | MotionAction::Cancel => {
                // A finger went down or up: the gesture starts over. Rebuild the set of fingers
                // from the event (a `Down` carries every active pointer; an `Up` carries the
                // finger that is leaving as well, and that one is gone).
                let previous = std::mem::replace(
                    &mut self.touch_points,
                    remaining_touch_points(motion_event),
                );
                self.gesture = Gesture::Undecided;
                self.gesture_origin = None;
                self.gesture_spread = None;
                // The finger that was already on the glass when this one joined: the one whose
                // touch has to be cancelled when the gesture takes over.
                self.gesture_pointer = previous.first().copied();
                GestureStep::None
            }
            MotionAction::Move => {
                // The number of fingers this event carries, and the number we were tracking. A
                // change in either (a third finger landing, one of two lifting) ends the gesture.
                if self.touch_points.len() != 2 || motion_event.pointer_count() != 2 {
                    // Keep the tracked positions current even for 1 or 3+ fingers (we just don't
                    // scroll for those), so a later two-finger move has a correct previous point.
                    self.sync_touch_points(motion_event);
                    self.gesture = Gesture::Undecided;
                    self.gesture_origin = None;
                    self.gesture_spread = None;
                    return GestureStep::None;
                }
                let Some(midpoint) = self.touch_midpoint() else {
                    return GestureStep::None;
                };
                let Some(spread) = self.touch_spread() else {
                    return GestureStep::None;
                };
                // Update the tracked positions first, so the next event measures against these.
                self.sync_touch_points(motion_event);
                let Some(new_midpoint) = self.touch_midpoint() else {
                    return GestureStep::None;
                };
                let Some(new_spread) = self.touch_spread() else {
                    return GestureStep::None;
                };

                // The deadzone is measured from where the gesture started.
                let (origin_spread, origin_midpoint) =
                    *self.gesture_origin.get_or_insert((spread, midpoint));
                let spread_change = (new_spread - origin_spread).abs();
                let moved = ((new_midpoint.x - origin_midpoint.x).powi(2)
                    + (new_midpoint.y - origin_midpoint.y).powi(2))
                .sqrt();

                match self.gesture {
                    Gesture::Undecided => {
                        // A pinch changes the spread much more than the midpoint moves; that is a
                        // zoom, not a scroll. Until one of the two is clearly bigger, the
                        // gesture stays undecided and nothing is reported: that is the
                        // deadzone between scrolling and zooming.
                        if spread_change > PINCH_DEADZONE_PX
                            && spread_change > PINCH_SPREAD_RATIO * moved
                        {
                            self.gesture = Gesture::Pinch;
                        } else if moved > PAN_DEADZONE_PX {
                            self.gesture = Gesture::Pan;
                        } else {
                            return GestureStep::None;
                        }
                    }
                    Gesture::Pan => {
                        // A pan that turns into a clear pinch hands over to the zoom, but only
                        // once the spread has changed twice as much as the deadzone: a gesture
                        // that has already started scrolling does not flip on a twitch.
                        if spread_change > 2.0 * PINCH_DEADZONE_PX {
                            self.gesture = Gesture::Pinch;
                        }
                    }
                    Gesture::Pinch => {}
                }

                match self.gesture {
                    Gesture::Pinch => {
                        // The zoom is relative to the last step, so a pinch that starts at the
                        // deadzone's edge does not jump: the spread change that got us here is
                        // the deadzone's, and the zoom starts from where the fingers are now.
                        let last = self.gesture_spread.replace(new_spread).unwrap_or(new_spread);
                        let factor = if last > 0.0 { new_spread / last } else { 1.0 };
                        GestureStep::Pinch(factor)
                    }
                    Gesture::Pan => {
                        // The fingers' midpoint moved: that is the scroll. The delta is signed
                        // the way a wheel event is (positive = the content moves down/right) and
                        // the content follows the fingers, so it is their own motion.
                        GestureStep::Pan(PhysicalPosition {
                            x: (new_midpoint.x - midpoint.x) * PAN_SPEED,
                            y: (new_midpoint.y - midpoint.y) * PAN_SPEED,
                        })
                    }
                    Gesture::Undecided => GestureStep::None,
                }
            }
            _ => GestureStep::None,
        }
    }

    /// Update the tracked positions of the fingers in `motion_event`, keeping the set in step with
    /// the glass (adding fingers that appeared, dropping the ones that left).
    fn sync_touch_points(&mut self, motion_event: &android_activity::input::MotionEvent<'_>) {
        let seen: Vec<(i32, PhysicalPosition<f64>)> = motion_event
            .pointers()
            .map(|pointer| {
                (
                    pointer.pointer_id(),
                    PhysicalPosition {
                        x: pointer.x() as f64,
                        y: pointer.y() as f64,
                    },
                )
            })
            .collect();
        if seen.len() == self.touch_points.len() {
            // Same fingers: just refresh their positions.
            self.touch_points = seen;
            return;
        }
        // The finger count changed mid-gesture (e.g. a third finger landed): keep the ones we know
        // and adopt the new ones, so the midpoint stays meaningful for the fingers we do track.
        for (id, position) in seen {
            match self.touch_points.iter_mut().find(|(known, _)| *known == id) {
                Some((_, known_position)) => *known_position = position,
                None => self.touch_points.push((id, position)),
            }
        }
    }

    /// The midpoint of exactly two tracked fingers, if both are known.
    fn touch_midpoint(&self) -> Option<PhysicalPosition<f64>> {
        let [(_, a), (_, b)] = self.touch_points.as_slice() else {
            return None;
        };
        Some(PhysicalPosition {
            x: (a.x + b.x) / 2.0,
            y: (a.y + b.y) / 2.0,
        })
    }

    /// The distance between exactly two tracked fingers, if both are known.
    fn touch_spread(&self) -> Option<f64> {
        let [(_, a), (_, b)] = self.touch_points.as_slice() else {
            return None;
        };
        Some(((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt())
    }

    fn handle_input_event<F>(
        &mut self,
        android_app: &AndroidApp,
        event: &InputEvent<'_>,
        callback: &mut F,
    ) -> InputStatus
    where
        F: FnMut(event::Event<T>, &RootAEL),
    {
        let mut input_status = InputStatus::Handled;
        match event {
            InputEvent::MotionEvent(motion_event) => {
                let window_id = window::WindowId(WindowId);
                let device_id = event::DeviceId(DeviceId(motion_event.device_id()));

                // Upstream winit turned *every* motion action into a touch event and dropped
                // everything else (`_ => None // TODO mouse events`), so a connected mouse
                // behaved like a fingertip: no right click, no wheel scrolling, no hover, and
                // egui flipped into its touch-screen interpretation as soon as the mouse
                // moved. Events from pointer-class devices (mouse/trackball/touchpad by
                // source, or anything whose tool is a mouse) are translated to real winit
                // pointer events here; fingers and styluses keep the upstream touch path.
                //
                // What the device is comes from its source *and* from the tool it was used
                // with: Android describes a device by a bitmask of source flags, and a
                // touchpad's is a combination of two of them (`SOURCE_MOUSE |
                // SOURCE_TOUCHPAD`), so matching the whole value against one constant
                // matches nothing at all — and a device that matches nothing is taken for a
                // finger on the glass, which is how a trackpad's scroll ended up dragging
                // the document around instead of scrolling it.
                let tool_type = if motion_event.pointer_count() > 0 {
                    motion_event.pointer_at_index(0).tool_type()
                } else {
                    ToolType::Unknown
                };
                let device = device_kind::classify(
                    u32::from(motion_event.source()),
                    match tool_type {
                        ToolType::Unknown => device_kind::Tool::Unknown,
                        ToolType::Finger => device_kind::Tool::Finger,
                        ToolType::Stylus => device_kind::Tool::Stylus,
                        ToolType::Mouse => device_kind::Tool::Mouse,
                        _ => device_kind::Tool::Other,
                    },
                );
                let pointer_class = device.pointer;
                // A touchpad is a pointing device whose tool is a finger: its scroll is
                // smooth and continuous, where a mouse's is a number of notches.
                let touchpad = device.touchpad;

                // A hardware touchpad's gesture: a two-finger slide or a pinch. Android
                // reports it as a motion event with a classification and a pointer that
                // walks across the screen as the gesture goes on — the pointer is not a
                // finger on the glass (it starts at the cursor and is moved by the
                // gesture), so the event is a scroll or a zoom and never a press, a drag
                // or a cursor move.
                let pad_gesture = match motion_event.classification() {
                    Classification::TwoFingerSwipe | Classification::MultiFingerSwipe => {
                        PadGesture::Scroll
                    }
                    Classification::Pinch => PadGesture::Pinch,
                    _ => PadGesture::None,
                };
                if pad_gesture != PadGesture::None {
                    let position = motion_event
                        .pointers()
                        .next()
                        .map(|pointer| pointer_position(&pointer));
                    match motion_event.action() {
                        MotionAction::Down | MotionAction::PointerDown => {
                            // The gesture starts here: the pointer is where the cursor is.
                            self.pad_position = position;
                            self.pad_spread = None;
                        }
                        MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel => {
                            self.pad_position = None;
                            self.pad_spread = None;
                        }
                        MotionAction::Move => match pad_gesture {
                            PadGesture::Scroll => {
                                // The pointer walks with the scroll, so the distance it
                                // walked is the scroll — the same pixels, in the same
                                // direction, as a two-finger drag on the glass.
                                if let Some(current) = position {
                                    let delta = self.pad_position.map_or(
                                        PhysicalPosition { x: 0.0, y: 0.0 },
                                        |previous| PhysicalPosition {
                                            x: current.x - previous.x,
                                            y: current.y - previous.y,
                                        },
                                    );
                                    self.pad_position = Some(current);
                                    if delta.x != 0.0 || delta.y != 0.0 {
                                        trace!("Trackpad scroll {device_id:?} {delta:?}");
                                        callback(
                                            event::Event::WindowEvent {
                                                window_id,
                                                event: event::WindowEvent::MouseWheel {
                                                    device_id,
                                                    delta: event::MouseScrollDelta::PixelDelta(
                                                        delta,
                                                    ),
                                                    phase: event::TouchPhase::Moved,
                                                },
                                            },
                                            self.window_target(),
                                        );
                                    }
                                }
                            }
                            PadGesture::Pinch => {
                                // The two pointers move apart or together: how much they
                                // changed is the zoom, relative to the last step.
                                let spread = pointer_spread(&motion_event);
                                if let Some(current) = spread {
                                    let factor = self.pad_spread.map_or(1.0, |previous| {
                                        if previous > 0.0 {
                                            current / previous
                                        } else {
                                            1.0
                                        }
                                    });
                                    self.pad_spread = Some(current);
                                    if factor != 1.0 {
                                        trace!("Trackpad pinch {device_id:?} {factor:?}");
                                        callback(
                                            event::Event::WindowEvent {
                                                window_id,
                                                event: event::WindowEvent::PinchGesture {
                                                    device_id,
                                                    // The log of the factor: `egui` turns it
                                                    // back into the factor itself.
                                                    delta: factor.ln(),
                                                    phase: event::TouchPhase::Moved,
                                                },
                                            },
                                            self.window_target(),
                                        );
                                    }
                                }
                            }
                            PadGesture::None => {}
                        },
                        _ => {}
                    }
                    return input_status;
                }

                match motion_event.action() {
                    // Hover: the cursor moves without any button held. Ordered before the
                    // button/move arms so a hovering mouse never lands in the touch path.
                    MotionAction::HoverMove
                    | MotionAction::HoverEnter
                    | MotionAction::HoverExit
                        if pointer_class =>
                    {
                        let position = motion_event
                            .pointers()
                            .next()
                            .map(|pointer| pointer_position(&pointer));
                        if let Some(position) = position {
                            trace!("Hover {device_id:?} loc={position:?}");
                            if motion_event.action() == MotionAction::HoverEnter {
                                callback(
                                    event::Event::WindowEvent {
                                        window_id,
                                        event: event::WindowEvent::CursorEntered { device_id },
                                    },
                                    self.window_target(),
                                );
                            }
                            callback(
                                event::Event::WindowEvent {
                                    window_id,
                                    event: event::WindowEvent::CursorMoved {
                                        device_id,
                                        position,
                                    },
                                },
                                self.window_target(),
                            );
                            if motion_event.action() == MotionAction::HoverExit {
                                callback(
                                    event::Event::WindowEvent {
                                        window_id,
                                        event: event::WindowEvent::CursorLeft { device_id },
                                    },
                                    self.window_target(),
                                );
                            }
                        }
                    }
                    // Wheel scrolling.
                    //
                    // Chromium draws the same line in the WebView build: a mouse wheel reports
                    // notches (`DOM_DELTA_LINE`) and a trackpad reports smooth, continuous
                    // amounts in pixels (`DOM_DELTA_PIXEL`). Android normalises a mouse's
                    // wheel to ±1 per notch on the scroll axes; a touchpad reports the same
                    // axes, but continuously — and the two are told apart by the *device*, not
                    // by the size of the number: a touchpad's source is `SOURCE_TOUCHPAD`, or
                    // a mouse source whose tool is a finger.
                    //
                    // A hardware touchpad's own two-finger gesture does not come through here
                    // at all: Android reports it as a classified motion event (a pointer that
                    // walks across the screen), which is handled above.
                    //
                    // Deciding by the size of the amount instead (a threshold above which the
                    // amount was treated as pixels) made the scroll jump by a different factor
                    // as soon as a touchpad's amount grew past it, and sent a mouse's notches
                    // down the smooth path, where they fed the momentum and the page coasted
                    // on after the wheel had stopped — a scroll that felt like it happened
                    // twice.
                    MotionAction::Scroll if pointer_class => {
                        let (vertical, horizontal) = motion_event
                            .pointers()
                            .next()
                            .map(|pointer| {
                                (
                                    pointer.axis_value(Axis::Vscroll),
                                    pointer.axis_value(Axis::Hscroll),
                                )
                            })
                            .unwrap_or((0.0, 0.0));
                        if vertical != 0.0 || horizontal != 0.0 {
                            let delta = if touchpad {
                                event::MouseScrollDelta::PixelDelta(PhysicalPosition {
                                    x: f64::from(horizontal) * POINTS_PER_AXIS,
                                    y: f64::from(vertical) * POINTS_PER_AXIS,
                                })
                            } else {
                                event::MouseScrollDelta::LineDelta(horizontal, vertical)
                            };
                            trace!("Scroll {device_id:?} {delta:?}");
                            callback(
                                event::Event::WindowEvent {
                                    window_id,
                                    event: event::WindowEvent::MouseWheel {
                                        device_id,
                                        delta,
                                        phase: event::TouchPhase::Moved,
                                    },
                                },
                                self.window_target(),
                            );
                        }
                    }
                    // Secondary/middle/back/forward buttons (the primary one arrives as
                    // plain Down/Up). The cursor position is reported first so egui has a
                    // valid pointer position to interpret the button change against.
                    MotionAction::ButtonPress | MotionAction::ButtonRelease if pointer_class => {
                        let state = if motion_event.action() == MotionAction::ButtonPress {
                            event::ElementState::Pressed
                        } else {
                            event::ElementState::Released
                        };
                        if let Some(position) = motion_event
                            .pointers()
                            .next()
                            .map(|pointer| pointer_position(&pointer))
                        {
                            callback(
                                event::Event::WindowEvent {
                                    window_id,
                                    event: event::WindowEvent::CursorMoved {
                                        device_id,
                                        position,
                                    },
                                },
                                self.window_target(),
                            );
                        }
                        let button = android_mouse_button(motion_event.action_button());
                        trace!("Button {device_id:?} {state:?} {button:?}");
                        callback(
                            event::Event::WindowEvent {
                                window_id,
                                event: event::WindowEvent::MouseInput {
                                    device_id,
                                    state,
                                    button,
                                },
                            },
                            self.window_target(),
                        );
                    }
                    // Mouse-class down/up/move/cancel: pointer events, never `Touch` (a
                    // single `Event::Touch` would make egui treat every later mouse move as
                    // a finger drag on a touch screen).
                    MotionAction::Down
                    | MotionAction::PointerDown
                    | MotionAction::Up
                    | MotionAction::PointerUp
                    | MotionAction::Move
                    | MotionAction::Cancel
                        if pointer_class =>
                    {
                        let single_pointer = matches!(
                            motion_event.action(),
                            MotionAction::Down
                                | MotionAction::PointerDown
                                | MotionAction::Up
                                | MotionAction::PointerUp
                        );
                        let state = match motion_event.action() {
                            MotionAction::Down | MotionAction::PointerDown => {
                                Some(event::ElementState::Pressed)
                            }
                            MotionAction::Up | MotionAction::PointerUp => {
                                Some(event::ElementState::Released)
                            }
                            // A cancelled gesture must not leave a button logically held.
                            MotionAction::Cancel => Some(event::ElementState::Released),
                            _ => None,
                        };
                        let pointers: Box<
                            dyn Iterator<Item = android_activity::input::Pointer<'_>>,
                        > = if single_pointer {
                            Box::new(std::iter::once(
                                motion_event.pointer_at_index(motion_event.pointer_index()),
                            ))
                        } else {
                            Box::new(motion_event.pointers())
                        };

                        let mut button_sent = false;
                        for pointer in pointers {
                            let position = pointer_position(&pointer);
                            trace!("Pointer {device_id:?} loc={position:?}, pointer={pointer:?}");
                            callback(
                                event::Event::WindowEvent {
                                    window_id,
                                    event: event::WindowEvent::CursorMoved {
                                        device_id,
                                        position,
                                    },
                                },
                                self.window_target(),
                            );
                            if !button_sent {
                                if let Some(state) = state {
                                    // Android reports the primary button as plain down/up;
                                    // anything else arrives via ButtonPress/ButtonRelease above.
                                    // A PointerDown/Up can also carry a non-primary button
                                    // (right/middle click); only treat it as Left when the
                                    // primary button is actually the one pressed.
                                    if motion_event.action_button() != Button::Primary {
                                        button_sent = true;
                                    } else {
                                        button_sent = true;
                                        callback(
                                            event::Event::WindowEvent {
                                                window_id,
                                                event: event::WindowEvent::MouseInput {
                                                    device_id,
                                                    state,
                                                    button: event::MouseButton::Left,
                                                },
                                            },
                                            self.window_target(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Fingers and styluses (and any device we don't classify as a pointer):
                    // the upstream touch path, plus the two-finger gesture recognition
                    // (`recognise_touch_gesture`). A two-finger pan emits `MouseWheel` and a
                    // pinch emits `PinchGesture`, the same two things a trackpad's wheel and
                    // pinch turn into in a browser; every other gesture falls through to the
                    // touch path unchanged, so single-finger taps and drags keep working
                    // exactly as before.
                    MotionAction::Down
                    | MotionAction::PointerDown
                    | MotionAction::Up
                    | MotionAction::PointerUp
                    | MotionAction::Move
                    | MotionAction::Cancel => {
                        match self.recognise_touch_gesture(motion_event) {
                            GestureStep::Pan(delta) => {
                                trace!("Two-finger scroll {device_id:?} {delta:?}");
                                callback(
                                    event::Event::WindowEvent {
                                        window_id,
                                        event: event::WindowEvent::MouseWheel {
                                            device_id,
                                            delta: event::MouseScrollDelta::PixelDelta(delta),
                                            phase: event::TouchPhase::Moved,
                                        },
                                    },
                                    self.window_target(),
                                );
                                return input_status;
                            }
                            GestureStep::Pinch(factor) => {
                                trace!("Two-finger pinch {device_id:?} {factor:?}");
                                callback(
                                    event::Event::WindowEvent {
                                        window_id,
                                        event: event::WindowEvent::PinchGesture {
                                            device_id,
                                            // The log of the factor: `egui` turns it back into
                                            // the factor itself.
                                            delta: factor.ln(),
                                            phase: event::TouchPhase::Moved,
                                        },
                                    },
                                    self.window_target(),
                                );
                                return input_status;
                            }
                            GestureStep::None => {}
                        }
                        // Two or more fingers on the glass: the gesture belongs to the backend
                        // (a scroll or a zoom), so the fingers are not reported as touches —
                        // egui would read them as a drag, or as a pinch of its own.
                        if self.touch_points.len() >= 2 {
                            // A second finger just landed: the first finger's touch, which egui
                            // took for a pointer press, is cancelled, so the gesture does not
                            // start a text selection. The finger is still on the glass, so the
                            // pointer keeps its place: the canvas only takes the scroll while
                            // the pointer is over it.
                            if motion_event.action() == MotionAction::PointerDown {
                                if let Some((id, location)) = self.gesture_pointer.take() {
                                    callback(
                                        event::Event::WindowEvent {
                                            window_id,
                                            event: event::WindowEvent::Touch(event::Touch {
                                                device_id,
                                                phase: event::TouchPhase::Cancelled,
                                                location,
                                                id: id as u64,
                                                force: None,
                                            }),
                                        },
                                        self.window_target(),
                                    );
                                }
                            }
                            return input_status;
                        }
                        let phase = match motion_event.action() {
                            MotionAction::Down | MotionAction::PointerDown => {
                                Some(event::TouchPhase::Started)
                            }
                            MotionAction::Up | MotionAction::PointerUp => {
                                Some(event::TouchPhase::Ended)
                            }
                            MotionAction::Move => Some(event::TouchPhase::Moved),
                            MotionAction::Cancel => Some(event::TouchPhase::Cancelled),
                            _ => None,
                        };
                        if let Some(phase) = phase {
                            let pointers: Box<
                                dyn Iterator<Item = android_activity::input::Pointer<'_>>,
                            > = match phase {
                                event::TouchPhase::Started | event::TouchPhase::Ended => {
                                    Box::new(std::iter::once(
                                        motion_event.pointer_at_index(motion_event.pointer_index()),
                                    ))
                                }
                                event::TouchPhase::Moved | event::TouchPhase::Cancelled => {
                                    Box::new(motion_event.pointers())
                                }
                            };

                            for pointer in pointers {
                                let location = PhysicalPosition {
                                    x: pointer.x() as _,
                                    y: pointer.y() as _,
                                };
                                trace!(
                                    "Input event {device_id:?}, {phase:?}, loc={location:?}, \
                                     pointer={pointer:?}"
                                );
                                let event = event::Event::WindowEvent {
                                    window_id,
                                    event: event::WindowEvent::Touch(event::Touch {
                                        device_id,
                                        phase,
                                        location,
                                        id: pointer.pointer_id() as u64,
                                        force: Some(Force::Normalized(pointer.pressure() as f64)),
                                    }),
                                };
                                callback(event, self.window_target());
                            }
                        }
                    }
                    // Anything else (hover/scroll/buttons on touch-only devices, actions
                    // newer than this winit, …): nothing to translate.
                    _ => {}
                }
            }
            InputEvent::KeyEvent(key) => {
                match key.key_code() {
                    // Flag keys related to volume as unhandled. While winit does not have a way for
                    // applications to configure what keys to flag as handled,
                    // this appears to be a good default until winit
                    // can be configured.
                    Keycode::VolumeUp | Keycode::VolumeDown | Keycode::VolumeMute
                        if self.ignore_volume_keys =>
                    {
                        input_status = InputStatus::Unhandled
                    }
                    keycode => {
                        let state = match key.action() {
                            KeyAction::Down => event::ElementState::Pressed,
                            KeyAction::Up => event::ElementState::Released,
                            _ => event::ElementState::Released,
                        };

                        let key_char = keycodes::character_map_and_combine_key(
                            android_app,
                            key,
                            &mut self.combining_accent,
                        );

                        let logical_key = keycodes::to_logical(key_char, keycode);
                        let text = if state == event::ElementState::Pressed {
                            logical_key.to_text().map(smol_str::SmolStr::new)
                        } else {
                            None
                        };

                        let event = event::Event::WindowEvent {
                            window_id: window::WindowId(WindowId),
                            event: event::WindowEvent::KeyboardInput {
                                device_id: event::DeviceId(DeviceId(key.device_id())),
                                event: event::KeyEvent {
                                    state,
                                    physical_key: keycodes::to_physical_key(keycode),
                                    logical_key,
                                    location: keycodes::to_location(keycode),
                                    repeat: key.repeat_count() > 0,
                                    text,
                                    platform_specific: KeyEventExtra {},
                                },
                                is_synthetic: false,
                            },
                        };
                        callback(event, self.window_target());
                    }
                }
            }
            _ => {
                warn!("Unknown android_activity input event {event:?}")
            }
        }

        input_status
    }

    pub fn run<F>(mut self, event_handler: F) -> Result<(), EventLoopError>
    where
        F: FnMut(event::Event<T>, &event_loop::ActiveEventLoop),
    {
        self.run_on_demand(event_handler)
    }

    pub fn run_on_demand<F>(&mut self, mut event_handler: F) -> Result<(), EventLoopError>
    where
        F: FnMut(event::Event<T>, &event_loop::ActiveEventLoop),
    {
        loop {
            match self.pump_events(None, &mut event_handler) {
                PumpStatus::Exit(0) => {
                    break Ok(());
                }
                PumpStatus::Exit(code) => {
                    break Err(EventLoopError::ExitFailure(code));
                }
                _ => {
                    continue;
                }
            }
        }
    }

    pub fn pump_events<F>(&mut self, timeout: Option<Duration>, mut callback: F) -> PumpStatus
    where
        F: FnMut(event::Event<T>, &RootAEL),
    {
        if !self.loop_running {
            self.loop_running = true;

            // Reset the internal state for the loop as we start running to
            // ensure consistent behaviour in case the loop runs and exits more
            // than once
            self.pending_redraw = false;
            self.cause = StartCause::Init;

            // run the initial loop iteration
            self.single_iteration(None, &mut callback);
        }

        // Consider the possibility that the `StartCause::Init` iteration could
        // request to Exit
        if !self.exiting() {
            self.poll_events_with_timeout(timeout, &mut callback);
        }
        if self.exiting() {
            self.loop_running = false;

            callback(event::Event::LoopExiting, self.window_target());

            PumpStatus::Exit(0)
        } else {
            PumpStatus::Continue
        }
    }

    fn poll_events_with_timeout<F>(&mut self, mut timeout: Option<Duration>, mut callback: F)
    where
        F: FnMut(event::Event<T>, &RootAEL),
    {
        let start = Instant::now();

        self.pending_redraw |= self.redraw_flag.get_and_reset();

        timeout =
            if self.running && (self.pending_redraw || self.user_events_receiver.has_incoming()) {
                // If we already have work to do then we don't want to block on the next poll
                Some(Duration::ZERO)
            } else {
                let control_flow_timeout = match self.control_flow() {
                    ControlFlow::Wait => None,
                    ControlFlow::Poll => Some(Duration::ZERO),
                    ControlFlow::WaitUntil(wait_deadline) => {
                        Some(wait_deadline.saturating_duration_since(start))
                    }
                };

                min_timeout(control_flow_timeout, timeout)
            };

        let app = self.android_app.clone(); // Don't borrow self as part of poll expression
        app.poll_events(timeout, |poll_event| {
            let mut main_event = None;

            match poll_event {
                android_activity::PollEvent::Wake => {
                    // In the X11 backend it's noted that too many false-positive wake ups
                    // would cause the event loop to run continuously. They handle this by
                    // re-checking for pending events (assuming they cover all
                    // valid reasons for a wake up).
                    //
                    // For now, user_events and redraw_requests are the only reasons to expect
                    // a wake up here so we can ignore the wake up if there are no events/requests.
                    // We also ignore wake ups while suspended.
                    self.pending_redraw |= self.redraw_flag.get_and_reset();
                    if !self.running
                        || (!self.pending_redraw && !self.user_events_receiver.has_incoming())
                    {
                        return;
                    }
                }
                android_activity::PollEvent::Timeout => {}
                android_activity::PollEvent::Main(event) => {
                    main_event = Some(event);
                }
                unknown_event => {
                    warn!("Unknown poll event {unknown_event:?} (ignored)");
                }
            }

            self.cause = match self.control_flow() {
                ControlFlow::Poll => StartCause::Poll,
                ControlFlow::Wait => StartCause::WaitCancelled {
                    start,
                    requested_resume: None,
                },
                ControlFlow::WaitUntil(deadline) => {
                    if Instant::now() < deadline {
                        StartCause::WaitCancelled {
                            start,
                            requested_resume: Some(deadline),
                        }
                    } else {
                        StartCause::ResumeTimeReached {
                            start,
                            requested_resume: deadline,
                        }
                    }
                }
            };

            self.single_iteration(main_event, &mut callback);
        });
    }

    pub fn window_target(&self) -> &event_loop::ActiveEventLoop {
        &self.window_target
    }

    pub fn create_proxy(&self) -> EventLoopProxy<T> {
        EventLoopProxy {
            user_events_sender: self.user_events_sender.clone(),
            waker: self.android_app.create_waker(),
        }
    }

    fn control_flow(&self) -> ControlFlow {
        self.window_target.p.control_flow()
    }

    fn exiting(&self) -> bool {
        self.window_target.p.exiting()
    }
}

pub struct EventLoopProxy<T: 'static> {
    user_events_sender: mpsc::Sender<T>,
    waker: AndroidAppWaker,
}

impl<T: 'static> Clone for EventLoopProxy<T> {
    fn clone(&self) -> Self {
        EventLoopProxy {
            user_events_sender: self.user_events_sender.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<T> EventLoopProxy<T> {
    pub fn send_event(&self, event: T) -> Result<(), event_loop::EventLoopClosed<T>> {
        self.user_events_sender
            .send(event)
            .map_err(|err| event_loop::EventLoopClosed(err.0))?;
        self.waker.wake();
        Ok(())
    }
}

pub struct ActiveEventLoop {
    pub(crate) app: AndroidApp,
    control_flow: Cell<ControlFlow>,
    exit: Cell<bool>,
    redraw_requester: RedrawRequester,
}

impl ActiveEventLoop {
    pub fn primary_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle::new(self.app.clone()))
    }

    pub fn create_custom_cursor(&self, source: CustomCursorSource) -> CustomCursor {
        let _ = source.inner;
        CustomCursor {
            inner: PlatformCustomCursor,
        }
    }

    pub fn available_monitors(&self) -> VecDeque<MonitorHandle> {
        let mut v = VecDeque::with_capacity(1);
        v.push_back(MonitorHandle::new(self.app.clone()));
        v
    }

    #[inline]
    pub fn listen_device_events(&self, _allowed: DeviceEvents) {}

    #[cfg(feature = "rwh_05")]
    #[inline]
    pub fn raw_display_handle_rwh_05(&self) -> rwh_05::RawDisplayHandle {
        rwh_05::RawDisplayHandle::Android(rwh_05::AndroidDisplayHandle::empty())
    }

    #[inline]
    pub fn system_theme(&self) -> Option<Theme> {
        None
    }

    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::RawDisplayHandle::Android(
            rwh_06::AndroidDisplayHandle::new(),
        ))
    }

    pub(crate) fn set_control_flow(&self, control_flow: ControlFlow) {
        self.control_flow.set(control_flow)
    }

    pub(crate) fn control_flow(&self) -> ControlFlow {
        self.control_flow.get()
    }

    pub(crate) fn exit(&self) {
        self.exit.set(true)
    }

    pub(crate) fn clear_exit(&self) {
        self.exit.set(false)
    }

    pub(crate) fn exiting(&self) -> bool {
        self.exit.get()
    }

    pub(crate) fn owned_display_handle(&self) -> OwnedDisplayHandle {
        OwnedDisplayHandle
    }
}

#[derive(Clone)]
pub(crate) struct OwnedDisplayHandle;

impl OwnedDisplayHandle {
    #[cfg(feature = "rwh_05")]
    #[inline]
    pub fn raw_display_handle_rwh_05(&self) -> rwh_05::RawDisplayHandle {
        rwh_05::AndroidDisplayHandle::empty().into()
    }

    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::AndroidDisplayHandle::new().into())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct WindowId;

impl WindowId {
    pub const fn dummy() -> Self {
        WindowId
    }
}

impl From<WindowId> for u64 {
    fn from(_: WindowId) -> Self {
        0
    }
}

impl From<u64> for WindowId {
    fn from(_: u64) -> Self {
        Self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(i32);

impl DeviceId {
    pub const fn dummy() -> Self {
        DeviceId(0)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlatformSpecificWindowAttributes;

pub(crate) struct Window {
    app: AndroidApp,
    redraw_requester: RedrawRequester,
}

impl Window {
    pub(crate) fn new(
        el: &ActiveEventLoop,
        _window_attrs: window::WindowAttributes,
    ) -> Result<Self, error::OsError> {
        // FIXME this ignores requested window attributes

        Ok(Self {
            app: el.app.clone(),
            redraw_requester: el.redraw_requester.clone(),
        })
    }

    pub(crate) fn maybe_queue_on_main(&self, f: impl FnOnce(&Self) + Send + 'static) {
        f(self)
    }

    pub(crate) fn maybe_wait_on_main<R: Send>(&self, f: impl FnOnce(&Self) -> R + Send) -> R {
        f(self)
    }

    pub fn id(&self) -> WindowId {
        WindowId
    }

    pub fn primary_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle::new(self.app.clone()))
    }

    pub fn available_monitors(&self) -> VecDeque<MonitorHandle> {
        let mut v = VecDeque::with_capacity(1);
        v.push_back(MonitorHandle::new(self.app.clone()));
        v
    }

    pub fn current_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle::new(self.app.clone()))
    }

    pub fn scale_factor(&self) -> f64 {
        MonitorHandle::new(self.app.clone()).scale_factor()
    }

    pub fn request_redraw(&self) {
        self.redraw_requester.request_redraw()
    }

    pub fn pre_present_notify(&self) {}

    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, error::NotSupportedError> {
        Err(error::NotSupportedError::new())
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, error::NotSupportedError> {
        Err(error::NotSupportedError::new())
    }

    pub fn set_outer_position(&self, _position: Position) {
        // no effect
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.outer_size()
    }

    pub fn request_inner_size(&self, _size: Size) -> Option<PhysicalSize<u32>> {
        Some(self.inner_size())
    }

    pub fn outer_size(&self) -> PhysicalSize<u32> {
        MonitorHandle::new(self.app.clone()).size()
    }

    pub fn set_min_inner_size(&self, _: Option<Size>) {}

    pub fn set_max_inner_size(&self, _: Option<Size>) {}

    pub fn resize_increments(&self) -> Option<PhysicalSize<u32>> {
        None
    }

    pub fn set_resize_increments(&self, _increments: Option<Size>) {}

    pub fn set_title(&self, _title: &str) {}

    pub fn set_transparent(&self, _transparent: bool) {}

    pub fn set_blur(&self, _blur: bool) {}

    pub fn set_visible(&self, _visibility: bool) {}

    pub fn is_visible(&self) -> Option<bool> {
        None
    }

    pub fn set_resizable(&self, _resizeable: bool) {}

    pub fn is_resizable(&self) -> bool {
        false
    }

    pub fn set_enabled_buttons(&self, _buttons: WindowButtons) {}

    pub fn enabled_buttons(&self) -> WindowButtons {
        WindowButtons::all()
    }

    pub fn set_minimized(&self, _minimized: bool) {}

    pub fn is_minimized(&self) -> Option<bool> {
        None
    }

    pub fn set_maximized(&self, _maximized: bool) {}

    pub fn is_maximized(&self) -> bool {
        false
    }

    pub fn set_fullscreen(&self, _monitor: Option<Fullscreen>) {
        warn!("Cannot set fullscreen on Android");
    }

    pub fn fullscreen(&self) -> Option<Fullscreen> {
        None
    }

    pub fn set_decorations(&self, _decorations: bool) {}

    pub fn is_decorated(&self) -> bool {
        true
    }

    pub fn set_window_level(&self, _level: WindowLevel) {}

    pub fn set_window_icon(&self, _window_icon: Option<crate::icon::Icon>) {}

    pub fn set_ime_cursor_area(&self, _position: Position, _size: Size) {}

    pub fn set_ime_allowed(&self, allowed: bool) {
        if allowed {
            self.app.show_soft_input(true);
        } else {
            self.app.hide_soft_input(true);
        }
    }

    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}

    pub fn focus_window(&self) {}

    pub fn request_user_attention(&self, _request_type: Option<window::UserAttentionType>) {}

    pub fn set_cursor(&self, _: Cursor) {}

    pub fn set_cursor_position(&self, _: Position) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(
            error::NotSupportedError::new(),
        ))
    }

    pub fn set_cursor_grab(&self, _: CursorGrabMode) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(
            error::NotSupportedError::new(),
        ))
    }

    pub fn set_cursor_visible(&self, _: bool) {}

    pub fn drag_window(&self) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(
            error::NotSupportedError::new(),
        ))
    }

    pub fn drag_resize_window(
        &self,
        _direction: ResizeDirection,
    ) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(
            error::NotSupportedError::new(),
        ))
    }

    #[inline]
    pub fn show_window_menu(&self, _position: Position) {}

    pub fn set_cursor_hittest(&self, _hittest: bool) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(
            error::NotSupportedError::new(),
        ))
    }

    #[cfg(feature = "rwh_04")]
    pub fn raw_window_handle_rwh_04(&self) -> rwh_04::RawWindowHandle {
        use rwh_04::HasRawWindowHandle;

        if let Some(native_window) = self.app.native_window().as_ref() {
            native_window.raw_window_handle()
        } else {
            panic!(
                "Cannot get the native window, it's null and will always be null before \
                 Event::Resumed and after Event::Suspended. Make sure you only call this function \
                 between those events."
            );
        }
    }

    #[cfg(feature = "rwh_05")]
    pub fn raw_window_handle_rwh_05(&self) -> rwh_05::RawWindowHandle {
        use rwh_05::HasRawWindowHandle;

        if let Some(native_window) = self.app.native_window().as_ref() {
            native_window.raw_window_handle()
        } else {
            panic!(
                "Cannot get the native window, it's null and will always be null before \
                 Event::Resumed and after Event::Suspended. Make sure you only call this function \
                 between those events."
            );
        }
    }

    #[cfg(feature = "rwh_05")]
    pub fn raw_display_handle_rwh_05(&self) -> rwh_05::RawDisplayHandle {
        rwh_05::RawDisplayHandle::Android(rwh_05::AndroidDisplayHandle::empty())
    }

    #[cfg(feature = "rwh_06")]
    // Allow the usage of HasRawWindowHandle inside this function
    #[allow(deprecated)]
    pub fn raw_window_handle_rwh_06(&self) -> Result<rwh_06::RawWindowHandle, rwh_06::HandleError> {
        use rwh_06::HasRawWindowHandle;

        if let Some(native_window) = self.app.native_window().as_ref() {
            native_window.raw_window_handle()
        } else {
            tracing::error!(
                "Cannot get the native window, it's null and will always be null before \
                 Event::Resumed and after Event::Suspended. Make sure you only call this function \
                 between those events."
            );
            Err(rwh_06::HandleError::Unavailable)
        }
    }

    #[cfg(feature = "rwh_06")]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::RawDisplayHandle::Android(
            rwh_06::AndroidDisplayHandle::new(),
        ))
    }

    pub fn config(&self) -> ConfigurationRef {
        self.app.config()
    }

    pub fn content_rect(&self) -> Rect {
        self.app.content_rect()
    }

    pub fn set_theme(&self, _theme: Option<Theme>) {}

    pub fn theme(&self) -> Option<Theme> {
        None
    }

    pub fn set_content_protected(&self, _protected: bool) {}

    pub fn has_focus(&self) -> bool {
        HAS_FOCUS.load(Ordering::Relaxed)
    }

    pub fn title(&self) -> String {
        String::new()
    }

    pub fn reset_dead_keys(&self) {}
}

#[derive(Default, Clone, Debug)]
pub struct OsError;

use std::fmt::{self, Display, Formatter};
impl Display for OsError {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        write!(fmt, "Android OS Error")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonitorHandle {
    app: AndroidApp,
}
impl PartialOrd for MonitorHandle {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MonitorHandle {
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl MonitorHandle {
    pub(crate) fn new(app: AndroidApp) -> Self {
        Self { app }
    }

    pub fn name(&self) -> Option<String> {
        Some("Android Device".to_owned())
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        if let Some(native_window) = self.app.native_window() {
            PhysicalSize::new(native_window.width() as _, native_window.height() as _)
        } else {
            PhysicalSize::new(0, 0)
        }
    }

    pub fn position(&self) -> PhysicalPosition<i32> {
        (0, 0).into()
    }

    pub fn scale_factor(&self) -> f64 {
        self.app
            .config()
            .density()
            .map(|dpi| dpi as f64 / 160.0)
            .unwrap_or(1.0)
    }

    pub fn refresh_rate_millihertz(&self) -> Option<u32> {
        // FIXME no way to get real refresh rate for now.
        None
    }

    pub fn video_modes(&self) -> impl Iterator<Item = VideoModeHandle> {
        let size = self.size().into();
        // FIXME this is not the real refresh rate
        // (it is guaranteed to support 32 bit color though)
        std::iter::once(VideoModeHandle {
            size,
            bit_depth: 32,
            refresh_rate_millihertz: 60000,
            monitor: self.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VideoModeHandle {
    size: (u32, u32),
    bit_depth: u16,
    refresh_rate_millihertz: u32,
    monitor: MonitorHandle,
}

impl VideoModeHandle {
    pub fn size(&self) -> PhysicalSize<u32> {
        self.size.into()
    }

    pub fn bit_depth(&self) -> u16 {
        self.bit_depth
    }

    pub fn refresh_rate_millihertz(&self) -> u32 {
        self.refresh_rate_millihertz
    }

    pub fn monitor(&self) -> MonitorHandle {
        self.monitor.clone()
    }
}
