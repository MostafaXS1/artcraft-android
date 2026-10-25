# ArtCraft Android Builder
## Go to [Releases](https://github.com/MostafaXS1/artcraft-android/releases/) to download the apps.

![ArtCraft Android](artcraft-android-1.png)

Build locally signed, 64-bit ARM (arm64) Android APKs from the official Storytold repositories for the ArtCraft apps.

The builder provides a repeatable path from an existing app repository to an installable Android package. It includes release and debug builds, optional app metadata overrides, a shared file-association registry, and persistent app-specific adapters for projects that need more than the generic launcher.

Project links: [ArtCraft](https://getartcraft.com/) · [The Crafting Apps](https://getartcraft.com/apps)

## Quick start

Run the scripts from any directory. Each script resolves its own location and switches to the project directory before building.

```sh
bash build.sh -pdfcraft          # Optimized release APK
bash build-debug.sh -pdfcraft    # Debug APK; larger and slower
```

Replace `-pdfcraft` with any supported app name.

## Supported apps

```text
photocraft vectorcraft filmcraft lightcraft pdfcraft effectcraft
designcraft figcraft wordcraft soundcraft deckcraft cadcraft gridcraft
subcraft
```

App names are passed as command-line flags, such as `-photocraft` or `-vectorcraft`.

## Options

| Option | Description |
| --- | --- |
| `-stable` | Build against latest stable release tag of the upstream app (default). |
| `-main`, `-latest` | Build against the rolling upstream `main` branch. |
| `-tag=<tag>` | Build against a specific git tag (e.g. `-tag=v0.12.1`). |
| `-branch=<branch>` | Build against a specific git branch. |
| `-appname=<name>` | Override the Android display name. By default, the app's built-in title is used. |
| `-packagename=<pkg>` | Override the Android package name. By default, the package is `com.artcraft.<app>`. |
| `-appicon=<path>` | Use a custom launcher icon PNG. By default, the app's own icon is used. |
| `-h`, `--help` | Show command help. |

## Examples

```sh
# Build PhotoCraft
bash build.sh -photocraft

# Override the display name
bash build.sh -photocraft -appname=notPhotoshop

# Override the Android package name
bash build.sh -photocraft -packagename=com.adobe.photoshop

# Use a custom launcher icon
bash build.sh -photocraft -appicon=/path/to/icon.png

# Set multiple options
bash build.sh -photocraft \
  -appname=MyApp \
  -packagename=com.example.myapp \
  -appicon=icon.png
```

## Build outputs

| Path | Contents |
| --- | --- |
| `output/<app>-<version>.apk` | Optimized release APK, versioned from the app's own source manifest (e.g. `output/vectorcraft-0.12.0.apk`) |
| `output/<app>-<version>-debug.apk` | Debug APK |
| `source/<app>/` | Source checkout and generated GameActivity crate |
| `build/<app>/target/` | Isolated Cargo build output for the app |
| `logs/` | Build logs for diagnosis |
| `keys/artcraft-builder.jks` | Persistent local signing keystore |

Build output is separated by app so that Cargo artifacts and source changes do not need to be shared between targets.

## Source checkout behavior

The builder is designed to preserve local work. It does not delete an existing source checkout, reset it, or pull over local changes. Subsequent builds reuse the existing checkout and any modifications made to the Android port.

Update app sources deliberately with Git after preserving your changes. Review the diff before switching versions or incorporating upstream changes.

A checkout is reused only when it is complete. A clone that was interrupted — by a killed build, or by a full disk — leaves a checkout whose Git metadata looks normal while most of the crates are missing from disk; the builder reports that as an incomplete checkout and asks for it to be moved aside, rather than building from it. New clones are written to a scratch directory and moved into place only once they have been checked, so an interrupted clone is never mistaken for a usable checkout.

### The shared craft-fonts

The apps that embed the shared faces (WordCraft, PdfCraft) read them from the
[craft-fonts](https://github.com/storytold/craft-fonts) checkout, an optional build input they
take through `CRAFT_FONTS_DIR`. The builder fetches that checkout next to the apps' own sources
(`source/craft-fonts`, kept like them, so a local pin is not overwritten) and points the build at
it, so the same faces the desktop release embeds are embedded here — the Arabic, Thai, Georgian
and Japanese ones, without which the PDF text writer refuses text it cannot shape ("The PDF
text fonts can't draw …"). The APKs grow by the faces they now carry. A checkout that is present
but has no `fonts/manifest.txt` is left alone, and the apps build without the faces.

## Build pipeline

Each build compiles the app's Rust code with [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) into a `jniLibs` tree and packages it with the Gradle Android Plugin (AGP 8.7.3 on the bundled Gradle 8.11.1 wrapper). The launcher runs as a GameActivity — Google's maintained Activity for native apps, whose Java half also hosts the soft keyboard input connection — so the APK's Java side links `androidx.games:games-activity` and AppCompat alongside the platform SDK.

The Gradle project, `AndroidManifest.xml` and the `Theme.AppCompat` theme are regenerated from the app's own Cargo metadata on every build (see `lib/android_project.py`). Signing uses the persistent builder keystore (`keys/artcraft-builder.jks`) through Gradle's signing configs, and `versionCode` follows the same formula as earlier toolchains, so update installs stay compatible.

Gradle needs a JDK in the 17–23 range. The builder uses the first one it finds on the machine and, when there is none, downloads Temurin JDK 21 into `.cache/jdk/` once. The signing keystore itself keeps being created with the system `keytool`.

The Android SDK is found the same way: the builder uses the first one that has the compile platform and the build tools — `ANDROID_HOME`, `ANDROID_SDK_ROOT`, its own `android-sdk/`, `~/Android/Sdk`, `/opt/android-sdk` — and when the machine has none, it fetches one into `android-sdk/` (the command-line tools, the platform and the build tools, with the licences accepted). The NDK is looked for across every SDK on the machine and is only downloaded when there is none at all: it is a gigabyte unpacked, and it does not have to live in the SDK root. `ARTCRAFT_SDK_DIR`, `ARTCRAFT_TARGET_SDK`, `ARTCRAFT_BUILD_TOOLS_VERSION` and `ARTCRAFT_NDK_VERSION` override the choices.

## Porting model and adapter boundary

The generic launcher targets native Rust/egui applications that expose `Session`, `Services`, and a public `*App` type from `crates/engine` and `crates/ui-egui`.

Some applications need an app-specific adapter rather than the generic startup path. This can apply to apps with:

- Specialized startup requirements, such as audio or media backends.
- Services that are not available through the default launcher setup.
- Dependencies that do not compile for Android without changes.

A failed build is recorded under `logs/`. Use [`adapters/README.md`](adapters/README.md) to add a persistent adapter for an app. A successful APK build confirms that the Android launch shell compiled; it does not, by itself, establish that every app workflow is ready for mobile use. Verify file access, save and export behavior, audio, clipboard, lifecycle handling, and touch interaction on a device.

## Runtime integration

### Preferences and recovery files

Each launcher persists the app's preferences under `config` in the app's private storage and restores them at startup. The path helpers an app writes through — `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and the app's own config folder — are pointed at the same location, so app-written autosave and recovery files are stored there as well. `HOME` and `XDG_DATA_HOME` are the one exception: they are the roots the font scanners read (see [System fonts](#system-fonts)) and stay on internal storage, the only place the links to `/system/fonts` can be created.

That storage is the app-specific external directory, `/storage/emulated/0/Android/data/com.artcraft.<app>/files`, rather than the internal `/data/user/0/<pkg>/files`. External storage is reachable from a phone file manager and over USB/MTP, which makes settings, autosaves and exports findable; internal storage is invisible without root. The internal directory is the fallback when external storage is unavailable (and is always used for the font environment, which needs links external storage cannot hold). Android 11+ hides `Android/data` from the on-device document picker, so use USB/MTP or `adb` to browse it.

A document opened through Storage Access Framework (SAF) is staged as `<files>/media/<document name>`, so the working copy the app edits is easy to find and the same document always maps to the same path. If that name is already taken by another document, a ` (2)`-style suffix is added rather than sharing the file (a name whose staged copy has since been deleted is free again — no suffix for a cleared document that comes back). When the app lets go of a document — the recent list is cleared, an entry is removed, or the capped list ages it out — and no open document shows it, its staged copy under `media/` is deleted; the original still lives at its SAF location, and the `uri:` mapping is kept so re-opening it reuses the same path. A one-time sweep, half a minute after start, also clears out staged copies that nothing refers to any more (leftovers from an older session or a crash).

PhotoCraft and VectorCraft save through their adapters, which use the same external location.

### System fonts

The launcher uses Android's system fonts alongside each app's embedded fonts. It installs device font faces under `/system/fonts` and `/product/fonts` as the lowest-priority entry of **every** egui font family an app binds — re-checked each frame, so a family an app creates later (GridCraft draws with its own named `ui`/`cell`/`fnt:…` families rather than egui's default `Proportional`) still receives the fallback. It then links the font directory into the user font locations read by app-level scanners:

- `$HOME/.fonts`
- `$HOME/.local/share/fonts`
- `$XDG_DATA_HOME/fonts`

A minimal `fonts.conf` provides the font configuration. The links and that file live in the **internal** private directory (`/data/user/0/<pkg>/files`), not the external one: Android's emulated external storage refuses `symlink`, so building them there leaves `$HOME/.fonts` and friends missing and the scanners find no system face — the reason non-Latin scripts broke under v23. `HOME` and `XDG_DATA_HOME` therefore point at the internal directory even though the rest of the app's data is external. This supports scripts including CJK, Arabic, Hebrew, Indic scripts, Thai, Khmer, and Myanmar when suitable fonts are present on the device. Existing app rendering is left unchanged where the app already has glyph coverage. Exact coverage depends on the fonts included by the device manufacturer.

The user's own fonts are found too: a face dropped in `/storage/emulated/0/Fonts` (or `/sdcard/Fonts`, or a subfolder of either) is picked up by the apps' own font scanners and can serve the interface's per-script fallback, so a script the device has no face for can still be drawn. The folder is read once the app has been granted "all files access", which the launcher asks for on first launch — every app asks, the PhotoCraft and VectorCraft adapters included (they previously neither declared the permission nor asked for it, so they could not read it at all). Only one of the three locations above is a real folder, holding a link per font file — a folder can carry the device's faces and the user's together one file at a time — and the other two are links *to* it, so a scanner reading any of them, or two as WordCraft does, reaches one folder however many names it reads and lists nothing twice. An app installed before this change still holds the older single link to `/system/fonts` in each place, which reads as an existing folder yet can hold no links of its own (they would all be made inside the read-only system partition); the first start after the update recognises that leftover and replaces it. A face the user supplied replaces a system file of the same name.

The links are kept current while the app runs: the folder is looked at every couple of seconds and re-linked when something changed, so a font added later — or the "all files access" being granted after the app started, which is how the ask actually plays out — is picked up without having to relaunch the app first. What the apps' font *lists* show is their own business: most of them build the list once per run, so a newly visible face shows up in the list after the app's next start, or right away through the app's own "load system fonts" action where it has one.

PdfCraft's interface fallback — an *installed* face, read at runtime and never embedded — is found the same way, through the font configuration the launcher writes. That configuration also offers the device's faces under the names a desktop system's fonts are known by, because the device ships the same scripts under other names: its Arabic face is the Naskh family and the Indic scripts are variable fonts, while an app looking for an installed face by name expects `NotoSansArabic-Regular.ttf` and the static Indic faces.

### Soft keyboard and text input

The Android soft keyboard is wired into every app. Each Activity installs an invisible, focusable `EditText`; its `InputConnection` controls keyboard visibility and forwards text commits, composition updates, deletions, and Enter to the Rust shell. The shell converts these into egui input-method events for the currently focused text field.

The keyboard overlays the fullscreen native surface rather than resizing it.

Because the events are queued in the Activity and read by the shell once per frame, every queued event also wakes the shell's loop: the loop otherwise sleeps in `ALooper_pollOnce` until something touches the screen, so a backspace would wait for the next repaint (egui's cursor blink, up to half a second) before the character went away. The wake goes through `GameActivity.onKeyDown` — whose native glue calls `ALooper_wake` — carrying a `KEYCODE_UNKNOWN` event, which winit reports as `Key::Unidentified` and no widget acts on.

### Edge-to-edge display

Every app draws under the status and navigation bars, with the bars transparent and their icons (clock, battery, navigation) matched to the app's current light or dark theme — switched per frame, so a light document never sits under white-on-white status icons, on Android 9 and 10 included. Selection handles and the long-press toolbar are suppressed over the native surface, where they otherwise appear on every touch with no text field to belong to.

### Launcher icons

Every app ships a proper adaptive launcher icon: the app's brand colour as a solid background layer, the artwork (its solid background flood-filled away, so the silhouette is the artwork itself) as the foreground inside the launcher's visible safe zone, and the same artwork as the `<monochrome>` layer Android 13+ uses for Material You themed icons — instead of tinting a whole flat square the way a plain PNG does. The flat density icons remain for file managers and other non-adaptive surfaces. The artwork comes from the app's own vector icon where it has one, otherwise its largest own PNG; a `-appicon` override goes through the same pipeline. Both adapters get the identical treatment from the same shared generator (`lib/app_icons.py`).

### Light and dark theme

Apps follow the phone's own appearance setting — opened in dark mode, the interface is dark from the first frame, and switching the phone to light or dark while the app runs re-themes it live, system-bar icons included (the Android side of what the web builds read from `prefers-color-scheme`). The in-app Appearance setting still lets you pin light or dark explicitly; system is its default.

### Interface size on a phone

The apps are laid out for a tablet, so on a phone-class screen (Android's own `sw600dp` boundary: the shortest screen edge under 600 density-independent pixels) the first launch sets an interface size that suits a phone — 150% for the document apps, whose ribbon and menus need the room, and 85% (75% in VectorCraft) for the canvas apps, which draw their work on the screen. It happens once, on the first launch only: after that the app's own size setting is yours, and the app never touches it again. Tablets are left at the size the system gives them.

### Mouse and trackpad

A connected mouse or trackpad gets hover, scroll wheel, and secondary click (the interface's context menu).

A mouse's wheel turns in notches (one notch, one step, and no coasting afterwards), while a trackpad's scroll is smooth and coasts when you flick it — the device itself says which it is, not the size of the amount. A hardware trackpad's own two-finger slide scrolls the document 1:1 in pixels and its two-finger pinch zooms it, the way a browser does it: Android reports the gesture as a classified motion event whose pointer walks across the screen, and the distance it walked is the scroll.

### Touch scrolling and pinch-zoom

A phone or tablet has no wheel, so a two-finger slide scrolls the document and a two-finger pinch zooms it, the way a browser does it. The gesture is recognised once, inside a deadzone: a slide that moves the fingers more than a finger's width scrolls, a pinch that changes the distance between the fingers zooms, and anything in between does nothing at all — so a scroll never turns into a zoom halfway through. A slide tracks the fingers 1:1 in pixels (a little faster, so a long passage is quick to cross) and a flick coasts to a stop, the way it does in a browser. The gesture never starts a text selection, and it never zooms by accident.

### The system back button

Android's back gesture and back key reach the apps. winit's Android backend consumes the key (it reports it as handled), so Android's own back handling never ran and the button arrived as egui's `Key::BrowserBack`, which almost no app listens for — it did nothing at all. The launcher now re-labels a back press as **Escape**, the key every app already treats as "close the dialog / cancel / go back", so the button gets its usual meaning without any app change. If the app consumed that Escape, the button did its job; if nothing consumed it, the app finishes, which is what back means on Android.

Before the app ends, it gets the last word: the apps that guard against losing work run the same check their desktop window-close button runs, so a document with unsaved changes opens the app's own "Do you want to quit without saving unsaved changes?" prompt instead of the app vanishing. The prompt is the app's own, so it looks and behaves exactly like the desktop's.

### Updates

Every APK carries the builder's own version. A few seconds after launch, a background thread asks GitHub for the latest release of the builder's repository; when that version is newer than the one in the app, the user is told and offered the download of the new APK. The check never blocks the app, is skipped once a version has been offered, and any failure (no network, a rate limit, an unexpected API shape) is logged and ignored. The apps need the `INTERNET` permission for it, which the builder adds to every launcher.

### Drag and drop

Files can be dragged out of the Files app (or a browser) and dropped onto the window — handy in split-screen next to another app. A dropped file is copied into app-private storage off the UI thread and opens through the same path a file shared through the share sheet uses, so it opens even while the app is sitting untouched. Persistable access is requested when the source offers it, since drag grants are otherwise temporary; write access is asked for first, because a dropped document is saved back to the place it came from.

### Printing

WordCraft prints through Android's native print system: the print flow offers the system print dialog, which reaches every printer the device knows — Wi-Fi, network, and cloud print services — with printer, paper size, duplex, color, and page range chosen in the dialog itself (Android keeps printer enumeration there; apps cannot list printers). The app's page range, copies, and collation are already applied to the PDF it generates, and the system spooler receives that PDF unchanged. Job feedback — submitted or failed — appears where the desktop spooler's would.

### Open-with and share-sheet integration

Every app is registered with Android's **Open with** and share sheets for the file types it handles. The current associations include:

| File types | Apps |
| --- | --- |
| PDF | PdfCraft, DesignCraft, VectorCraft |
| Word documents | WordCraft |
| Spreadsheets | GridCraft |
| Presentations | DeckCraft |
| Images and camera raw | PhotoCraft, LightCraft |
| Video and audio | FilmCraft, SoundCraft |
| CAD drawings | CADCraft |
| App-specific project formats | The corresponding app |

File associations are centralized in [`lib/file_associations.py`](lib/file_associations.py) and applied to launcher and adapter manifests to keep them consistent. Extensions Android's MIME database knows are matched by the type it actually reports (for example `.psd` is `image/x-photoshop`, not `image/vnd.adobe.photoshop`), and every extension is also matched by file name so that a provider which stamps a generic `application/octet-stream` still offers the app. Those by-name elements pair the escaped-dot pattern with `android:host="*"` — Android only evaluates a `pathPattern` when the element also declares an authority, and without one a scheme-plus-path element matches every `file://`/`content://` URI — and the same pattern is repeated in a second, VIEW-only element declaring `mimeType="*/*"`, because a filter that declares a MIME type only matches intents that carry one. An app is therefore offered for its own extensions and types and for nothing else.

When a file is opened, shared, or passed through a link, the launcher copies it into app-private storage off the UI thread and opens it through the same path used by the app's own file picker. This is supported while the app is already running as well as on initial launch, and the file opens straight away: egui only draws a frame when something asks it to, so a background watcher asks the app for a repaint as soon as a file is waiting, instead of leaving it in the queue until the user happens to touch the screen.

## Automated builds & CI

A complete GitHub Actions CI/CD pipeline is provided in [`.github/workflows/build.yml`](.github/workflows/build.yml). It builds all 13 apps in parallel across an isolated matrix, compiles against stable releases, caches dependencies via sccache and Gradle, tests APK integrity, and automatically publishes GitHub releases when tags are pushed.

For detailed architecture, caching strategy, and workflow execution details, see [`docs/ci-workflow.md`](docs/ci-workflow.md).

## Signing and distribution

The keystore in `keys/` is intended for local builds only. A randomly generated local password is stored in `keys/.keystore-password`.

Keep the keystore and password safe and stable if you need to install later APKs as updates over an earlier build from this builder. Do not use this key for a public release. Public distribution requires a separate release-signing process and appropriate key management.

## Troubleshooting

Start with the relevant file in `logs/`. If the generic launcher cannot initialize an app or one of its dependencies does not support Android, review the adapter guidance in [`adapters/README.md`](adapters/README.md).

For device-level validation, test the workflows that matter for the target app, including opening files from other apps, saving and exporting, keyboard input, audio or media playback, lifecycle transitions, and touch interaction.

## Related projects

- [ArtCraft website](https://getartcraft.com/)
- [The Crafting Apps](https://getartcraft.com/apps)
- [Storytold repositories on GitHub](https://github.com/storytold)

