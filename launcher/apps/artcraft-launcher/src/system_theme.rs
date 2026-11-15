//! The desktop appearance portal supplies the theme that winit cannot read on Wayland.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use zbus::blocking::{Proxy, connection::Builder};
use zbus::zvariant::{OwnedValue, Value};

const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";

#[derive(Default)]
pub struct Monitor(Arc<AtomicU8>);

impl Monitor {
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let monitor = Self::default();
        let state = Arc::downgrade(&monitor.0);
        if let Err(e) = std::thread::Builder::new().name("system-theme".into()).spawn(move || {
            let publish = |scheme| {
                if let Some(state) = state.upgrade()
                    && state.swap(scheme, Ordering::Relaxed) != scheme
                {
                    log::debug!("desktop appearance: {:?}", theme(scheme));
                    wake();
                }
            };
            // Retry after the portal goes away, backing off to every 5 minutes where there's none.
            let mut retry = Duration::from_secs(5);
            while state.strong_count() > 0 {
                match watch(&publish) {
                    Ok(()) => retry = Duration::from_secs(5),
                    Err(e) => log::debug!("desktop appearance portal: {e}"),
                }
                publish(0);
                std::thread::sleep(retry);
                retry = (retry * 2).min(Duration::from_secs(300));
            }
        }) {
            log::warn!("couldn't monitor the system theme: {e}");
        }
        monitor
    }

    pub fn apply(&self, input: &mut egui::RawInput) {
        if let Some(theme) = theme(self.0.load(Ordering::Relaxed)) {
            input.system_theme = Some(theme);
        }
    }
}

fn theme(scheme: u8) -> Option<egui::Theme> {
    match scheme {
        1 => Some(egui::Theme::Dark),
        2 => Some(egui::Theme::Light),
        _ => None,
    }
}

fn scheme(value: OwnedValue) -> u8 {
    // Read returns an extra variant wrapper; ReadOne and SettingChanged do not.
    let value = match Value::from(value) {
        Value::Value(value) => *value,
        value => value,
    };
    match u32::try_from(value) {
        Ok(1) => 1,
        Ok(2) => 2,
        _ => 0,
    }
}

fn watch(publish: &impl Fn(u8)) -> zbus::Result<()> {
    let connection = Builder::session()?.method_timeout(Duration::from_secs(3)).build()?;
    let proxy = Proxy::new(&connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings")?;
    // Subscribe before reading so a theme switch during startup is not missed.
    let changes = proxy.receive_signal_with_args("SettingChanged", &[(0, APPEARANCE), (1, COLOR_SCHEME)])?;
    let initial: OwnedValue = proxy.call("ReadOne", &(APPEARANCE, COLOR_SCHEME)).or_else(|_| proxy.call("Read", &(APPEARANCE, COLOR_SCHEME)))?;
    publish(scheme(initial));
    for message in changes {
        let (namespace, key, value): (String, String, OwnedValue) = message.body().deserialize()?;
        if namespace == APPEARANCE && key == COLOR_SCHEME {
            publish(scheme(value));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a session bus and a desktop portal with an explicit color preference"]
    fn running_desktop_portal_reaches_egui() {
        let (send, receive) = std::sync::mpsc::channel();
        let monitor = Monitor::start(move || {
            let _ = send.send(());
        });
        assert!(receive.recv_timeout(Duration::from_secs(10)).is_ok());
        let mut input = egui::RawInput::default();
        monitor.apply(&mut input);
        assert!(input.system_theme.is_some());
        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::System);
        ctx.begin_pass(input.clone());
        assert_eq!(Some(ctx.theme()), input.system_theme);
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn portal_values_include_no_preference_and_unknown_values() {
        for (value, expected) in [(0, None), (1, Some(egui::Theme::Dark)), (2, Some(egui::Theme::Light)), (3, None), (u32::MAX, None)] {
            assert_eq!(theme(scheme(OwnedValue::from(value))), expected);
        }
        assert_eq!(scheme(OwnedValue::from(true)), 0);
    }

    #[test]
    fn legacy_read_unwraps_the_extra_variant() {
        let value = Value::Value(Box::new(Value::U32(2)));
        assert_eq!(value.try_to_owned().map(scheme).ok(), Some(2));
    }

    #[test]
    fn portal_changes_preserve_explicit_choices_and_unknown_falls_back_to_winit() {
        let monitor = Monitor::default();
        let ctx = egui::Context::default();
        for preference in [egui::ThemePreference::System, egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(preference);
            for scheme in [1, 2, 0] {
                monitor.0.store(scheme, Ordering::Relaxed);
                let mut input = egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() };
                monitor.apply(&mut input);
                let expected = match preference {
                    egui::ThemePreference::Light => egui::Theme::Light,
                    egui::ThemePreference::Dark => egui::Theme::Dark,
                    egui::ThemePreference::System => theme(scheme).unwrap_or(egui::Theme::Dark),
                };
                ctx.begin_pass(input);
                assert_eq!(ctx.theme(), expected);
                ctx.end_pass().textures_delta.clear();
            }
        }
    }
}
