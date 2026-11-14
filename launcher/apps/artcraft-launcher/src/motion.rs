//! Whether the OS asks for fewer animations: Windows' "Animation effects" switched off, macOS'
//! "Reduce motion", a Linux or BSD desktop's animations setting. The UI asks about once a second
//! while it draws (`Host::reduce_motion`), so a change shows without a restart.

/// Does the OS ask for fewer animations? Animations stay on where it can't be told.
#[cfg(windows)]
pub fn reduced() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Control Panel\Desktop")
        .and_then(|desktop| desktop.get_raw_value("UserPreferencesMask"))
        .is_ok_and(|mask| animations_off(&mask.bytes))
}

/// Windows keeps `SPI_GETCLIENTAREAANIMATION` (the switch in Settings, and what browsers follow
/// for `prefers-reduced-motion`) as bit 33 of `UserPreferencesMask`. Reading the registry stands
/// in for `SystemParametersInfoW`, which needs `unsafe`. A mask too short for the bit says nothing.
#[cfg(any(windows, test))]
fn animations_off(mask: &[u8]) -> bool {
    mask.get(4).is_some_and(|byte| byte & 0b10 == 0)
}

/// Does the OS ask for fewer animations?
#[cfg(target_os = "macos")]
pub fn reduced() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Does the desktop ask for fewer animations? What its settings portal last said: the first call
/// starts a thread that asks it and then follows its changes, as an answer over D-Bus can take a
/// while and the UI must not wait for one. Animations stay on until it has answered, and where
/// there is no portal.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn reduced() -> bool {
    use std::sync::Once;
    use std::sync::atomic::Ordering;
    static WATCH: Once = Once::new();
    WATCH.call_once(|| {
        std::thread::spawn(|| {
            if let Err(e) = portal::watch() {
                log::debug!("the desktop's animation setting can't be read: {e}");
            }
        });
    });
    portal::REDUCED.load(Ordering::Relaxed)
}

/// The XDG desktop portal's settings, which a sandboxed Flatpak can read as well.
#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use std::sync::atomic::{AtomicBool, Ordering};

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    pub static REDUCED: AtomicBool = AtomicBool::new(false);

    /// Keeps [`REDUCED`] to what the portal says, for as long as the session bus is there.
    pub fn watch() -> zbus::Result<()> {
        let bus = Connection::session()?;
        let settings = Proxy::new(&bus, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings")?;
        let changes = settings.receive_signal("SettingChanged")?;
        REDUCED.store(reduced(&settings), Ordering::Relaxed);
        for _ in changes {
            REDUCED.store(reduced(&settings), Ordering::Relaxed);
        }
        Ok(())
    }

    /// Any one of the settings desktops keep this in asking for fewer animations is enough: the
    /// portal's own `reduced-motion`, GNOME's `enable-animations` (which desktops built on GTK
    /// share), and KDE's animation speed, where 0 is "instant".
    fn reduced(settings: &Proxy<'_>) -> bool {
        let read = |namespace: &str, key: &str| {
            // `ReadOne` is the newer call; older portals only have `Read`.
            ["ReadOne", "Read"].iter().find_map(|method| settings.call::<_, _, OwnedValue>(*method, &(namespace, key)).ok())
        };
        read("org.freedesktop.appearance", "reduced-motion").is_some_and(|v| matches!(plain(&v), Value::U32(1)))
            || read("org.gnome.desktop.interface", "enable-animations").is_some_and(|v| matches!(plain(&v), Value::Bool(false)))
            || read("org.kde.kdeglobals.KDE", "AnimationDurationFactor").is_some_and(|v| match plain(&v) {
                Value::Str(factor) => factor.trim().parse::<f64>().is_ok_and(|f| f == 0.0),
                Value::F64(factor) => *factor == 0.0,
                _ => false,
            })
    }

    /// The value inside the variants the portal wraps it in (`Read` wraps it twice).
    fn plain<'v>(value: &'v Value<'v>) -> &'v Value<'v> {
        match value {
            Value::Value(inner) => plain(inner),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_animation_setting_is_bit_33_of_the_mask() {
        // Windows' default mask, then one from a machine with "Animation effects" switched off.
        assert!(!animations_off(&[0x9E, 0x1E, 0x07, 0x80, 0x12, 0x00, 0x00, 0x00]));
        assert!(animations_off(&[0x90, 0x12, 0x03, 0x80, 0x10, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn a_short_mask_leaves_animations_on() {
        assert!(!animations_off(&[]));
        assert!(!animations_off(&[0x90, 0x12, 0x03, 0x80]));
    }
}
