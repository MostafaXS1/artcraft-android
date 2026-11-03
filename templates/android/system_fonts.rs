//! Android system fonts, shared by every generated launcher and by the hand-written adapter
//! launchers.
//!
//! The desktop builds render every script with the fonts the operating system ships. On Android
//! those fonts live in `/system/fonts` (and `/product/fonts`), which none of the desktop font
//! helpers look at, so non-Latin text came out as blank boxes ("tofu"). This module closes that gap
//! in two ways:
//!
//! * [`install_system_fonts`] hands epaint the Android faces for the scripts the app's own fonts do
//!   not cover, as *lowest*-priority fallbacks, so they are only consulted for glyphs nothing else
//!   has. It is called every frame (cheaply) because an app that installs its own fonts replaces the
//!   whole definition set and would otherwise drop the fallbacks.
//! * [`install_font_env`] links the system font folder into the per-user locations the apps' own
//!   font scanners (`wordcraft`, `gridcraft`, `deckcraft`, `designcraft`, `vectorcraft`,
//!   `pdfcraft`, …) expect on a desktop, and writes a minimal fontconfig file for the ones that read
//!   one. This makes their document/palette rendering see the same fonts as the interface. The
//!   user's own font folder (`/storage/emulated/0/Fonts`, readable once the app has been granted
//!   "all files access") is included in both: a face the user drops there is used like any
//!   installed one. [`refresh_font_links`] keeps those links current — a font added later, or the
//!   access being granted after the app started, is picked up without a restart.
//!
//! Everything is best-effort and lazy: a missing, unreadable or already-covered font is skipped, and
//! a font is only read from disk the first time it is actually needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Directories Android keeps its system fonts in, in the order they are preferred.
const FONT_DIRS: &[&str] = &["/system/fonts", "/product/fonts"];

/// Where the user can drop fonts for the apps to use: a `Fonts` folder in the shared storage.
/// The folder is readable once the app has been granted "all files access" (the launcher asks for
/// it); without that it is simply not there, and the apps keep the fonts they have.
const USER_FONT_DIRS: &[&str] = &["/storage/emulated/0/Fonts", "/sdcard/Fonts"];

/// The names a desktop system's fonts are known by, and the script whose face stands for them.
///
/// An app that looks for an *installed* face **by name** — PdfCraft's last-resort interface font
/// does, the way it does on the desktop — finds the device's face only when it is offered under
/// that name. The device ships the same scripts under other names: its Arabic face is the Naskh
/// family, and the Indic scripts are variable fonts. The device's faces are therefore linked into
/// the font configuration under these names, and an app that knows the desktop names finds the
/// face it wants.
const DESKTOP_NAMES: &[(&str, &str)] = &[
    ("NotoSansArabic-Regular.ttf", "artcraft-arabic"),
    ("NotoSansArabicUI-Regular.ttf", "artcraft-arabic"),
    ("NotoSansDevanagari-Regular.ttf", "artcraft-devanagari"),
    ("NotoSansDevanagariUI-Regular.ttf", "artcraft-devanagari"),
    ("NotoSansTelugu-Regular.ttf", "artcraft-telugu"),
    ("NotoSansTeluguUI-Regular.ttf", "artcraft-telugu"),
    ("NotoSansSinhala-Regular.ttf", "artcraft-sinhala"),
    ("NotoSansSinhalaUI-Regular.ttf", "artcraft-sinhala"),
    ("NotoSansTamil-Regular.ttf", "artcraft-tamil"),
    ("NotoSansTamilUI-Regular.ttf", "artcraft-tamil"),
];

/// One non-Latin script worth falling back to, its probe character and the system font files that
/// draw it (best candidate first, with the face index for `.ttc` collections).
struct SystemScript {
    /// The epaint font key. Also used to tell our fallbacks from the app's own fonts.
    key: &'static str,
    /// A letter only this script uses. When the active font chain can already draw it, the script is
    /// left to the app's own fonts and no system face is installed.
    probe: char,
    /// `(file name, face index)` candidates, in preference order.
    candidates: &'static [(&'static str, u32)],
}

/// The scripts Android ships fonts for. The probes are letters the embedded Latin faces cannot
/// draw, so a script is only loaded when it is actually missing. OEM font sets vary, so each script
/// lists the several names the same face is shipped under (AOSP `Noto*`, the older `Droid*`, and the
/// per-region CJK files).
const SYSTEM_SCRIPTS: &[SystemScript] = &[
    SystemScript {
        key: "artcraft-cjk",
        probe: '中',
        candidates: &[
            ("NotoSansCJK-Regular.ttc", 0),
            ("NotoSansCJKjp-Regular.otf", 0),
            ("NotoSansHans-Regular.otf", 0),
            ("NotoSansCJKsc-Regular.otf", 0),
            ("DroidSansFallback.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-hangul",
        probe: '가',
        candidates: &[
            ("NotoSansCJKkr-Regular.otf", 0),
            ("NotoSansKR-Regular.otf", 0),
            ("NotoSansKR-VF.ttf", 0),
            ("NotoSansCJK-Regular.ttc", 1),
            ("DroidSansFallback.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-emoji",
        probe: '😀',
        candidates: &[
            ("NotoColorEmoji.ttf", 0),
            ("NotoColorEmojiCompat.ttf", 0),
            ("NotoColorEmojiLegacy.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-arabic",
        probe: 'ا',
        candidates: &[
            ("NotoSansArabic-Regular.ttf", 0),
            ("NotoSansArabicUI-Regular.ttf", 0),
            ("NotoNaskhArabic-Regular.ttf", 0),
            ("DroidSansArabic.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-hebrew",
        probe: 'א',
        candidates: &[
            ("NotoSansHebrew-Regular.ttf", 0),
            ("NotoSansHebrewUI-Regular.ttf", 0),
            ("DroidSansHebrew-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-devanagari",
        probe: 'क',
        candidates: &[
            ("NotoSansDevanagari-Regular.ttf", 0),
            ("NotoSansDevanagariUI-Regular.ttf", 0),
            ("DroidSansDevanagari-Regular.ttf", 0),
            // Recent devices ship the Indic scripts as variable fonts only.
            ("NotoSansDevanagari-VF.ttf", 0),
            ("NotoSansDevanagariUI-VF.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-bengali",
        probe: 'ক',
        candidates: &[
            ("NotoSansBengali-Regular.ttf", 0),
            ("NotoSansBengaliUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-tamil",
        probe: 'க',
        candidates: &[
            ("NotoSansTamil-Regular.ttf", 0),
            ("NotoSansTamilUI-Regular.ttf", 0),
            // Recent devices ship the Indic scripts as variable fonts only.
            ("NotoSansTamil-VF.ttf", 0),
            ("NotoSansTamilUI-VF.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-telugu",
        probe: 'క',
        candidates: &[
            ("NotoSansTelugu-Regular.ttf", 0),
            ("NotoSansTeluguUI-Regular.ttf", 0),
            // Recent devices ship the Indic scripts as variable fonts only.
            ("NotoSansTelugu-VF.ttf", 0),
            ("NotoSansTeluguUI-VF.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-kannada",
        probe: 'ಕ',
        candidates: &[
            ("NotoSansKannada-Regular.ttf", 0),
            ("NotoSansKannadaUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-malayalam",
        probe: 'ക',
        candidates: &[
            ("NotoSansMalayalam-Regular.ttf", 0),
            ("NotoSansMalayalamUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-gujarati",
        probe: 'ક',
        candidates: &[
            ("NotoSansGujarati-Regular.ttf", 0),
            ("NotoSansGujaratiUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-gurmukhi",
        probe: 'ਕ',
        candidates: &[
            ("NotoSansGurmukhi-Regular.ttf", 0),
            ("NotoSansGurmukhiUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-oriya",
        probe: 'କ',
        candidates: &[
            ("NotoSansOriya-Regular.ttf", 0),
            ("NotoSansOriyaUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-sinhala",
        probe: 'ක',
        candidates: &[
            ("NotoSansSinhala-Regular.ttf", 0),
            ("NotoSansSinhalaUI-Regular.ttf", 0),
            // Recent devices ship the Indic scripts as variable fonts only.
            ("NotoSansSinhala-VF.ttf", 0),
            ("NotoSansSinhalaUI-VF.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-thai",
        probe: 'ก',
        candidates: &[
            ("NotoSansThai-Regular.ttf", 0),
            ("NotoSansThaiUI-Regular.ttf", 0),
            ("DroidSansThai.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-lao",
        probe: 'ກ',
        candidates: &[
            ("NotoSansLao-Regular.ttf", 0),
            ("NotoSansLaoUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-khmer",
        probe: 'ក',
        candidates: &[
            ("NotoSansKhmer-Regular.ttf", 0),
            ("NotoSansKhmerUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-myanmar",
        probe: 'က',
        candidates: &[
            ("NotoSansMyanmar-Regular.ttf", 0),
            ("NotoSansMyanmarUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-ethiopic",
        probe: 'ሀ',
        candidates: &[
            ("NotoSansEthiopic-Regular.ttf", 0),
            ("NotoSansEthiopicUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-georgian",
        probe: 'ა',
        candidates: &[
            ("NotoSansGeorgian-Regular.ttf", 0),
            ("NotoSansGeorgianUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-armenian",
        probe: 'Ա',
        candidates: &[
            ("NotoSansArmenian-Regular.ttf", 0),
            ("NotoSansArmenianUI-Regular.ttf", 0),
        ],
    },
    SystemScript {
        key: "artcraft-symbols",
        probe: '✓',
        candidates: &[
            ("NotoSansSymbols-Regular.ttf", 0),
            ("NotoSansSymbols2-Regular.ttf", 0),
        ],
    },
];

/// Whether `key` names one of our system faces for `script`: `script.key` itself, or a later
/// generation `script.key#n` (see [`install_system_fonts`]).
fn is_our_face(key: &str, script: &SystemScript) -> bool {
    key == script.key || key.strip_prefix(script.key).is_some_and(|rest| rest.starts_with('#'))
}

/// Bumped each time our faces are re-appended for families that appeared after the last install, so
/// the same face can be added again under a fresh key (`Context::add_font` skips a key it already
/// holds, which is exactly what would keep a family born later from ever receiving the fallback).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Install any missing system fallbacks into every active font family.
///
/// Called from the app's `ui` (inside an egui pass, where fonts exist) each frame: it is one cheap
/// lookup per family when everything is already in place, and it *repairs* the definition set after
/// an app replaces it wholesale (`Context::set_fonts`).
///
/// Presence in `font_data` is *not* enough to decide there is nothing to do. An app that draws with
/// its own families (`FontFamily::Name("cell-…")`, as GridCraft does) builds them in a `set_fonts`
/// call whose definitions are applied on the *next* pass, so our faces are first appended to the
/// families that already existed and `Context::add_font` then refuses to re-add them to the
/// families that appear afterwards — leaving every non-Latin glyph drawn in those families a blank
/// box. We therefore check *coverage per family* and, when one is missing a face, append it under a
/// fresh generation key. Generations share the same borrowed font bytes, and a key no family
/// references is never rasterized, so this costs no measurable memory.
pub fn install_system_fonts(ctx: &egui::Context) {
    // The user's own font folder can change while the app runs — a font added, or the app granted
    // "all files access" after it started — so the folders the apps' scanners read are kept in
    // step here (cheaply; the work happens only when something actually changed).
    refresh_font_links();
    // The families bound this pass, and, per script, those whose chain carries none of our faces.
    let (families, missing) = ctx.fonts(|view| {
        let definitions = view.definitions();
        let families: Vec<egui::FontFamily> = view.families();
        let missing: Vec<Vec<usize>> = SYSTEM_SCRIPTS
            .iter()
            .map(|script| {
                families
                    .iter()
                    .enumerate()
                    .filter(|(_, family)| {
                        !definitions
                            .families
                            .get(*family)
                            .is_some_and(|chain| chain.iter().any(|key| is_our_face(key, script)))
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        (families, missing)
    });
    if families.is_empty() || missing.iter().all(Vec::is_empty) {
        return;
    }

    let mut added = false;
    for (script, missing) in SYSTEM_SCRIPTS.iter().zip(&missing) {
        if missing.is_empty() {
            continue;
        }
        let Some(data) = script_font(script) else { continue };
        // Append as the last fallback, so an app's own faces always win and only glyphs they lack
        // come from the system face.
        let inserts = missing
            .iter()
            .map(|&index| egui::epaint::text::InsertFontFamily {
                family: families[index].clone(),
                priority: egui::epaint::text::FontPriority::Lowest,
            })
            .collect();
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        let key = format!("{}#{generation}", script.key);
        ctx.add_font(egui::epaint::text::FontInsert::new(&key, data, inserts));
        log::info!(
            "Android: added system fallback font {} (U+{:04X}) to {} families that lacked it",
            script.key,
            script.probe as u32,
            missing.len()
        );
        added = true;
    }
    if added {
        // `add_font` only queues the change; egui applies it at the start of the next pass, so ask
        // for that pass rather than waiting for the next input event.
        ctx.request_repaint();
    }
}

/// The file that holds the face for `script` on this device, and the face's index in it.
fn script_candidate(script: &'static SystemScript) -> Option<(PathBuf, u32)> {
    script.candidates.iter().find_map(|&(name, index)| {
        let path = font_path(name)?;
        Some((path, index))
    })
}

/// The font for `script`, read from disk the first time it is asked for (`None` when the device
/// ships no usable file for it).
fn script_font(script: &'static SystemScript) -> Option<egui::FontData> {
    static READ: OnceLock<Mutex<HashMap<&'static str, Option<egui::FontData>>>> = OnceLock::new();
    let mut read = READ
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = read.get(script.key) {
        return cached.clone();
    }

    let font = script_candidate(script).and_then(|(path, index)| {
        let bytes = file_bytes(&path)?;
        // Guard the face index so an out-of-range one is skipped instead of panicking epaint.
        if index >= font_face_count(bytes) {
            return None;
        }
        let mut data = egui::FontData::from_static(bytes);
        data.index = index;
        Some(data)
    });
    if font.is_none() {
        log::info!("Android: no system font found for {}", script.key);
    }
    read.insert(script.key, font.clone());
    font
}

/// The bytes of a system font file, read once per path (several scripts may name the same
/// collection) and kept for the life of the process.
fn file_bytes(path: &Path) -> Option<&'static [u8]> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<&'static [u8]>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(path) {
        return *cached;
    }

    let bytes = match std::fs::read(path) {
        Ok(bytes) if looks_like_font(&bytes) => {
            let leaked: &'static mut [u8] = Box::leak(bytes.into_boxed_slice());
            Some(&*leaked)
        }
        Ok(_) => {
            log::warn!("Skipping system font {}: not a font file", path.display());
            None
        }
        Err(error) => {
            log::warn!("Could not read system font {}: {error}", path.display());
            None
        }
    };
    cache.insert(path.to_path_buf(), bytes);
    bytes
}

/// Make `dir` a real folder that can hold links.
///
/// Builds up to v0.1.1 linked a folder as a whole (`.fonts` -> `/system/fonts`), which was enough
/// when only the device's own faces had to show up. Such a leftover has to go first: it hides the
/// user's own faces, and nothing can be linked *inside* it — its entries live on the read-only
/// system partition, so every `symlink()` there fails with `EROFS`, quietly, for every font.
/// `create_dir_all` will not help either, since a link to a folder reads as an existing folder.
/// (`unlink` removes only the link, never the folder it points at.)
#[cfg(unix)]
fn ensure_real_dir(dir: &Path) {
    if std::fs::symlink_metadata(dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(dir);
    }
    let _ = std::fs::create_dir_all(dir);
}

/// Point one of the folder names the apps read at the real folder holding the links, replacing
/// whatever the name held: an older build's link to `/system/fonts`, a real folder of links left by
/// the design this replaces, or an unrelated file.
///
/// Only the name is replaced. A folder is unlinked entry by entry — every entry it holds is a link
/// made here, so unlinking removes those links and the fonts they point at stay where they are.
#[cfg(unix)]
fn link_folder_alias(alias: &Path, target: &Path) {
    match std::fs::symlink_metadata(alias) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let _ = std::fs::remove_file(alias);
        }
        Ok(meta) if meta.is_dir() => {
            let _ = std::fs::remove_dir_all(alias);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(alias);
        }
        Err(_) => {}
    }
    let _ = std::os::unix::fs::symlink(target, alias);
}

/// Link every font file of the known font folders into `dir`, so a scanner that reads that folder
/// finds them all — the device's own faces and the ones the user dropped in their `Fonts` folder
/// alike.
///
/// A subfolder is linked as a folder, so a face the user keeps in a subfolder of their font folder
/// is found too (the apps' scanners walk into subfolders).
///
/// The user's folder is linked last, so a face they put there replaces a system file of the same
/// name: they put it there on purpose. (The interface's per-script fallback looks the other way
/// round — the system face first — because there a broken file would cost the script its
/// fallback entirely.)
///
/// Returns how many files were linked from the user's own folder.
#[cfg(unix)]
fn link_fonts(dir: &Path) -> usize {
    ensure_real_dir(dir);

    let mut user_files = 0;
    // The first failure only: it says what went wrong for the whole folder.
    let mut failed: Option<String> = None;
    // Each source once. `/sdcard/Fonts` and `/storage/emulated/0/Fonts` are the same folder, and
    // a second walk would link every file of it twice — and count them twice.
    let mut visited = std::collections::HashSet::new();
    for (index, source) in FONT_DIRS.iter().chain(USER_FONT_DIRS.iter()).enumerate() {
        let from_user = index >= FONT_DIRS.len();
        if !visited.insert(
            std::fs::canonicalize(source).unwrap_or_else(|_| PathBuf::from(source)),
        ) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(source) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name() else { continue };
            let link = dir.join(name);
            if path.is_dir() {
                // A subfolder: linked whole, so the scanners walk into it.
                let _ = std::fs::remove_file(&link);
                if let Err(error) = std::os::unix::fs::symlink(&path, &link) {
                    if failed.is_none() {
                        failed = Some(error.to_string());
                    }
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            // Replace whatever is there: the user's own fonts are linked after the system's.
            let _ = std::fs::remove_file(&link);
            match std::os::unix::fs::symlink(&path, &link) {
                Ok(()) => {
                    if from_user {
                        user_files += 1;
                    }
                }
                Err(error) => {
                    if failed.is_none() {
                        failed = Some(error.to_string());
                    }
                }
            }
        }
    }
    if let Some(error) = failed {
        log::warn!(
            "Android: a font folder could not be fully linked into {}: {error}",
            dir.display()
        );
    }
    user_files
}

/// The folders the apps' own font scanners read, where the device's faces and the user's own fonts
/// are linked. Set by [`install_font_env`], which runs before the first frame.
static FONT_LINKS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// What one of the user's own font folders looks like from in here. The refresh keys on this, so it
/// also says which of the reasons a font dropped there is missing applies: no such folder, no
/// access, or a folder whose contents changed.
#[derive(PartialEq, Eq, Clone)]
enum UserFontFolder {
    /// No such folder — the user has not made one, and there is nothing to report about that.
    Missing,
    /// There, but not readable: the app has not been granted "all files access".
    Denied,
    /// Readable, with a cheap signature of what it holds — its modification time and how many
    /// entries — which changes when a font is added, removed or renamed.
    Readable { modified: u64, entries: u64 },
}

/// The state of the user's own font folders.
fn user_font_state() -> Vec<UserFontFolder> {
    USER_FONT_DIRS
        .iter()
        .map(|dir| {
            let path = Path::new(dir);
            let entries = match std::fs::read_dir(path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return UserFontFolder::Missing;
                }
                Err(_) => return UserFontFolder::Denied,
            };
            let count = entries.flatten().count() as u64;
            let modified = std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_nanos() as u64)
                .unwrap_or(0);
            UserFontFolder::Readable {
                modified,
                entries: count,
            }
        })
        .collect()
}

/// Say what the user's own font folders gave this pass: the files linked when they could be read,
/// and the reason they could not be when they could not be. A folder the user has never made says
/// nothing — there is no access problem to report about it.
fn report_user_fonts(linked: usize, folders: &[UserFontFolder]) {
    let paths = USER_FONT_DIRS.join(", ");
    if folders
        .iter()
        .any(|folder| matches!(folder, UserFontFolder::Readable { .. }))
    {
        log::info!("Android: user fonts: {linked} file(s) in {paths}");
    } else if folders
        .iter()
        .any(|folder| matches!(folder, UserFontFolder::Denied))
    {
        log::info!(
            "Android: no access to the user's font folder ({paths}); the app needs \"all files access\" to use the fonts kept there"
        );
    }
}

/// The last seen state of the user's font folders, and when the folders were last looked at.
struct FontLinkState {
    state: Vec<UserFontFolder>,
    checked: std::time::Instant,
}

/// The remembered state, shared by the linker and the per-frame check.
fn font_link_state() -> &'static Mutex<Option<FontLinkState>> {
    static STATE: OnceLock<Mutex<Option<FontLinkState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Remember the state of the user's font folders, so the next check knows whether anything changed.
fn remember_font_state(state: Vec<UserFontFolder>) {
    let mut slot = font_link_state().lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(FontLinkState {
        state,
        checked: std::time::Instant::now(),
    });
}

/// Re-link the font folders when the user's own font folder changed, or when it became readable.
///
/// The launcher asks for "all files access" on the app's first launch, and the user answers in the
/// settings screen — after the app has already started, and after the fonts were linked for the
/// first time. Without this the user's own fonts would only appear after a restart, so the
/// folders are looked at again every couple of seconds and re-linked when something changed.
///
/// Called from [`install_system_fonts`], which every app runs each frame; the check itself is
/// throttled, and the re-linking only happens when the folder really changed.
pub fn refresh_font_links() {
    let Some(targets) = FONT_LINKS.get() else { return };
    let now = std::time::Instant::now();
    {
        let slot = font_link_state().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = slot.as_ref() {
            if now.duration_since(last.checked) < std::time::Duration::from_secs(2) {
                return;
            }
        }
    }
    let state = user_font_state();
    {
        let mut slot = font_link_state().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = slot.as_mut() {
            if last.state == state {
                last.checked = now;
                return;
            }
        }
    }
    let mut linked = 0;
    for target in targets {
        linked = link_fonts(target);
    }
    report_user_fonts(linked, &state);
    remember_font_state(state);
}

/// Link the device's faces into `dir` under the names a desktop system's fonts are known by, so
/// an app that looks for an installed face by name finds the face it wants (see
/// [`DESKTOP_NAMES`]).
#[cfg(unix)]
fn link_desktop_names(dir: &Path) {
    ensure_real_dir(dir);
    for (name, script_key) in DESKTOP_NAMES {
        let Some(script) = SYSTEM_SCRIPTS.iter().find(|script| script.key == *script_key) else {
            continue;
        };
        let Some((source, _index)) = script_candidate(script) else { continue };
        let link = dir.join(name);
        let _ = std::fs::remove_file(&link);
        let _ = std::os::unix::fs::symlink(&source, &link);
    }
}

/// Where a system font file lives, if the device ships it — or if the user dropped a face of
/// that name in their own font folder. The device's own faces come first: they are known to be
/// good, and a file the user put there may be damaged.
fn font_path(name: &str) -> Option<PathBuf> {
    FONT_DIRS
        .iter()
        .chain(USER_FONT_DIRS.iter())
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
}

/// Whether `bytes` starts like a font epaint can parse. `Context::add_font` panics on data it cannot
/// parse, so a damaged or unexpected system file is skipped rather than allowed to crash the app.
fn looks_like_font(bytes: &[u8]) -> bool {
    matches!(
        bytes.first_chunk::<4>(),
        Some([0x00, 0x01, 0x00, 0x00]) // TrueType outlines
            | Some([b'O', b'T', b'T', b'O']) // CFF/OpenType outlines
            | Some([b't', b't', b'c', b'f']) // font collection
            | Some([b't', b'r', b'u', b'e']) // legacy TrueType
    )
}

/// How many faces a font file holds (`1` for a single-face font, the collection's count for a
/// `.ttc`).
fn font_face_count(bytes: &[u8]) -> u32 {
    if bytes.starts_with(b"ttcf") {
        bytes
            .get(8..12)
            .and_then(|count| <[u8; 4]>::try_from(count).ok())
            .map(u32::from_be_bytes)
            .unwrap_or(1)
    } else {
        1
    }
}

/// Make Android's system fonts visible to the app's own font scanners.
///
/// Their desktop code looks under the user's home (`$HOME/.fonts`, `$HOME/.local/share/fonts`) and
/// the XDG data directory (`$XDG_DATA_HOME/fonts`), none of which exist on Android. Linking every
/// font file the device has — and every file in the user's own font folder — into a real folder
/// under those names, and writing a minimal `fonts.conf` for the fontconfig readers, is enough for
/// the existing scanners, without editing any app.
///
/// The links can only be created on **internal** storage: Android's emulated external storage
/// (`/storage/emulated/0/...`, a FUSE/sdcardfs mount) refuses `symlink` with `EPERM`. Creating them
/// under the external app directory therefore left `$HOME/.fonts` and friends missing, and the
/// scanners found no system face for Arabic, Hebrew, CJK and the other non-Latin scripts (blank
/// boxes). Fonts are not user data, so this environment stays in the internal private directory even
/// though the settings, working copies and exports live on external storage.
pub fn install_font_env(app: &android_activity::AndroidApp) {
    let Some(base) = app.internal_data_path() else { return };
    let home = base.join("home");
    let _ = std::fs::create_dir_all(home.join(".local/share"));

    #[cfg(unix)]
    {
        // One real folder of per-file links, with the other two folder names the apps read as
        // links *to* it.
        //
        // A folder can only be linked to one target, so the device's own faces and the user's
        // have to live together in one real folder. `$HOME/.local/share/fonts` and
        // `$XDG_DATA_HOME/fonts` are links to that folder, so a scanner may read any of them — or
        // two, as WordCraft reads two — and still reach one place: every name resolves to a single
        // folder, which is what the per-path de-duplication the scanners do keys on, so nothing is
        // catalogued twice. That is the shape the original build had (each of those names a link to
        // `/system/fonts`), and the point of this one is that it also has room for the user's own
        // folder, which a link to a read-only system folder could never have.
        let root = home.join(".fonts");
        let state = user_font_state();
        let linked = link_fonts(&root);
        for alias in [home.join(".local/share/fonts"), base.join("fonts")] {
            link_folder_alias(&alias, &root);
        }
        // Linked again when the user's own font folder changes or becomes readable (see
        // [`refresh_font_links`]), so the folder to link into is remembered here.
        let _ = FONT_LINKS.set(vec![root]);
        report_user_fonts(linked, &state);
        remember_font_state(state);
    }

    let conf_dir = base.join("fontconfig");
    let _ = std::fs::create_dir_all(conf_dir.join("cache"));
    let conf = conf_dir.join("fonts.conf");
    // The faces under their desktop names, in a folder of their own: an app that looks for an
    // installed face by name (PdfCraft's last-resort interface font) finds the device's face
    // there, under the name it knows.
    let named_dir = conf_dir.join("faces");
    #[cfg(unix)]
    {
        let _ = std::fs::create_dir_all(&named_dir);
        link_desktop_names(&named_dir);
    }
    // Written on every start, not only the first: the list of folders grows (the user's own font
    // folder was added later), and a stale file would keep the apps from seeing the new ones.
    let dirs: String = FONT_DIRS
        .iter()
        .chain(USER_FONT_DIRS.iter())
        .map(|dir| format!("<dir>{dir}</dir>"))
        .chain(std::iter::once(format!("<dir>{}</dir>", named_dir.display())))
        .collect();
    let cache = conf_dir.join("cache");
    let xml = format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
         <fontconfig>\n<cachedir>{}</cachedir>\n{dirs}\n</fontconfig>\n",
        cache.display()
    );
    let _ = std::fs::write(&conf, xml);
    log::info!("Android: font folders: {}", dirs.replace("<dir>", "").replace("</dir>", ", "));

    // SAFETY: `android_main` runs before the app starts any worker thread, so no other thread is
    // reading the environment here. `HOME`/`XDG_DATA_HOME` are the roots the apps' font scanners
    // read, so they are pointed at the internal links above *unconditionally*: the app-data redirect
    // (`install_persist_env`) may already have chosen external storage for the settings and caches,
    // and external storage cannot hold these links.
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &base);
        std::env::set_var("FONTCONFIG_FILE", &conf);
        std::env::set_var("FONTCONFIG_PATH", &conf_dir);
    }
}
