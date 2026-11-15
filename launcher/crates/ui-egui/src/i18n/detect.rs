//! Working out the system language: the OS's preferred UI languages on the desktop, the
//! browser's on the web.

use std::sync::OnceLock;

use super::lang::{Lang, lang_from_tag};

/// The system language, worked out once.
static SYSTEM: OnceLock<Lang> = OnceLock::new();

/// Work out the system language on a background thread, so the first frame doesn't wait for the
/// locale probe. Call it at startup; a frame that needs the language before the probe finishes
/// waits for it. A no-op on wasm.
pub fn detect_system_lang_in_background() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = std::thread::Builder::new().name("locale-probe".into()).spawn(|| {
            system_lang();
        });
    }
}

/// The system language (cached). English when it can't be determined.
pub fn system_lang() -> Lang {
    // Tests drive the UI by its English labels whatever the developer's locale is.
    if cfg!(test) {
        return Lang::EN;
    }
    *SYSTEM.get_or_init(detect_system_lang)
}

/// Fix the system language before anything asks for it (integration tests, which are compiled
/// without `cfg(test)`, use it to stay independent of the developer's locale).
pub fn set_test_lang(lang: Lang) {
    let _ = SYSTEM.set(lang);
}

/// Tell the UI the system's preferred languages, most preferred first, where it can't find them
/// itself: the web shell passes the browser's (`navigator.languages`). The first supported one is
/// the system language (English if none is); call it before the first frame. Has no effect once
/// the system language is known.
pub fn set_system_locales<S: AsRef<str>>(tags: &[S]) {
    let lang = first_supported(tags.iter().map(AsRef::as_ref)).unwrap_or(Lang::EN);
    // Already set means already detected (or reported): the first answer stands.
    let _ = SYSTEM.set(lang);
}

/// The first supported language in a preference list, most preferred first.
pub(super) fn first_supported<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<Lang> {
    tags.into_iter().find_map(lang_from_tag)
}

/// `ARTCRAFT_LAUNCHER_LOCALE` overrides everything; then the OS's preferred UI languages
/// (`sys-locale`: `LANGUAGE`/`LC_*`/`LANG` on Linux and the BSDs, the preferred-languages list on
/// macOS, which apps opened from the Finder need since they inherit no `LANG`, and the user's UI
/// languages on Windows).
#[cfg(not(target_arch = "wasm32"))]
fn detect_system_lang() -> Lang {
    if let Some(l) = std::env::var("ARTCRAFT_LAUNCHER_LOCALE").ok().and_then(|v| first_supported(v.split(':'))) {
        return l;
    }
    let locales: Vec<String> = sys_locale::get_locales().collect();
    first_supported(locales.iter().map(String::as_str)).unwrap_or(Lang::EN)
}

/// The browser's languages arrive through [`set_system_locales`]; without them, English.
#[cfg(target_arch = "wasm32")]
fn detect_system_lang() -> Lang {
    Lang::EN
}
