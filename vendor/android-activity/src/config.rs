use core::fmt;
use core::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use ndk::asset::AssetManager;

use ndk::configuration::{
    Configuration, Keyboard, KeysHidden, LayoutDir, NavHidden, Navigation, Orientation, ScreenLong,
    ScreenSize, Touchscreen, UiModeNight, UiModeType,
};

/// The app's asset manager, which is how the platform's current configuration is read.
static ASSET_MANAGER: AtomicPtr<ndk_sys::AAssetManager> = AtomicPtr::new(std::ptr::null_mut());

/// Remembers the app's asset manager (the `AndroidApp` keeps one for its whole life).
pub(crate) fn remember_asset_manager(manager: &AssetManager) {
    ASSET_MANAGER.store(manager.ptr().as_ptr().cast(), Ordering::Relaxed);
}

/// How long a keyboard's state is remembered before the platform is asked again. Reading the
/// configuration is a JNI call, and a keyboard comes and goes rarely.
const KEYBOARD_STATE_TTL: Duration = Duration::from_millis(200);

/// Whether a hardware keyboard is attached to the device, right now.
///
/// This is what Android itself uses to decide whether the soft keyboard is needed: a hardware
/// keyboard reports [`ndk::configuration::Keyboard::Qwerty`] and is not hidden. The on-screen
/// helpers that stand in for the soft keyboard (the Shift/Ctrl/Alt buttons, the
/// Cut/Copy/Paste/Delete bar) read this and stay away when there is a keyboard.
///
/// The configuration Android hands the app only changes when the system sends a configuration
/// change, and it does not always do that when a keyboard is plugged in or unplugged, so this
/// asks the platform instead of trusting the cached configuration.
pub fn hardware_keyboard_connected() -> bool {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    {
        let guard = match LAST.lock() {
            Ok(guard) => guard,
            Err(_) => return read_keyboard_state(),
        };
        if let Some((read_at, connected)) = *guard {
            if read_at.elapsed() < KEYBOARD_STATE_TTL {
                return connected;
            }
        }
    }

    let connected = read_keyboard_state();
    if let Ok(mut guard) = LAST.lock() {
        *guard = Some((Instant::now(), connected));
    }
    connected
}

/// The platform's current answer, read from the app's own configuration.
fn read_keyboard_state() -> bool {
    let manager = ASSET_MANAGER.load(Ordering::Relaxed);
    if manager.is_null() {
        return false;
    }
    let Some(manager) = NonNull::new(manager) else {
        return false;
    };
    // SAFETY: the pointer is the app's own asset manager, which lives as long as the process.
    let manager = unsafe { AssetManager::from_ptr(manager) };
    let config = Configuration::from_asset_manager(&manager);
    config.keyboard() == Keyboard::Qwerty && config.keys_hidden() == KeysHidden::No
}

/// A runtime-replacable reference to [`ndk::configuration::Configuration`].
///
/// # Warning
///
/// The value held by this reference **will change** with every [`super::MainEvent::ConfigChanged`]
/// event that is raised.  You should **not** [`Clone`] this type to compare it against a
/// "new" [`super::AndroidApp::config()`] when that event is raised, since both point to the same
/// internal [`ndk::configuration::Configuration`] and will be identical.
#[derive(Clone)]
pub struct ConfigurationRef {
    config: Arc<RwLock<Configuration>>,
}
impl PartialEq for ConfigurationRef {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.config, &other.config) {
            true
        } else {
            let other_guard = other.config.read().unwrap();
            self.config.read().unwrap().eq(&*other_guard)
        }
    }
}
impl Eq for ConfigurationRef {}

impl fmt::Debug for ConfigurationRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.config.read().unwrap().fmt(f)
    }
}

impl ConfigurationRef {
    pub(crate) fn new(config: Configuration) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub(crate) fn replace(&self, src: Configuration) {
        self.config.write().unwrap().copy(&src);
    }

    // Returns a deep copy of the full application configuration
    pub fn copy(&self) -> Configuration {
        let mut dest = Configuration::new();
        dest.copy(&self.config.read().unwrap());
        dest
    }
    /// Returns the country code, as a [`String`] of two characters, if set
    pub fn country(&self) -> Option<String> {
        self.config.read().unwrap().country()
    }

    /// Returns the screen density in dpi.
    ///
    /// On some devices it can return values outside of the density enum.
    pub fn density(&self) -> Option<u32> {
        self.config.read().unwrap().density()
    }

    /// Returns the keyboard type.
    pub fn keyboard(&self) -> Keyboard {
        self.config.read().unwrap().keyboard()
    }

    /// Returns keyboard visibility/availability.
    pub fn keys_hidden(&self) -> KeysHidden {
        self.config.read().unwrap().keys_hidden()
    }

    /// Returns the language, as a `String` of two characters, if a language is set
    pub fn language(&self) -> Option<String> {
        self.config.read().unwrap().language()
    }

    /// Returns the layout direction
    pub fn layout_direction(&self) -> LayoutDir {
        self.config.read().unwrap().layout_direction()
    }

    /// Returns the mobile country code.
    pub fn mcc(&self) -> i32 {
        self.config.read().unwrap().mcc()
    }

    /// Returns the mobile network code, if one is defined
    pub fn mnc(&self) -> Option<i32> {
        self.config.read().unwrap().mnc()
    }

    pub fn nav_hidden(&self) -> NavHidden {
        self.config.read().unwrap().nav_hidden()
    }

    pub fn navigation(&self) -> Navigation {
        self.config.read().unwrap().navigation()
    }

    pub fn orientation(&self) -> Orientation {
        self.config.read().unwrap().orientation()
    }

    pub fn screen_height_dp(&self) -> Option<i32> {
        self.config.read().unwrap().screen_height_dp()
    }

    pub fn screen_width_dp(&self) -> Option<i32> {
        self.config.read().unwrap().screen_width_dp()
    }

    pub fn screen_long(&self) -> ScreenLong {
        self.config.read().unwrap().screen_long()
    }

    #[cfg(feature = "api-level-30")]
    pub fn screen_round(&self) -> ScreenRound {
        self.config.read().unwrap().screen_round()
    }

    pub fn screen_size(&self) -> ScreenSize {
        self.config.read().unwrap().screen_size()
    }

    pub fn sdk_version(&self) -> i32 {
        self.config.read().unwrap().sdk_version()
    }

    pub fn smallest_screen_width_dp(&self) -> Option<i32> {
        self.config.read().unwrap().smallest_screen_width_dp()
    }

    pub fn touchscreen(&self) -> Touchscreen {
        self.config.read().unwrap().touchscreen()
    }

    pub fn ui_mode_night(&self) -> UiModeNight {
        self.config.read().unwrap().ui_mode_night()
    }

    pub fn ui_mode_type(&self) -> UiModeType {
        self.config.read().unwrap().ui_mode_type()
    }
}
