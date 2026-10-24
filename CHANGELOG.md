# Changelog

## v37 — 2026-10-20

### Fixed
- Two-way Storage Access Framework (SAF) auto-sync and document autosave across all 14 apps. Files
  opened via system file pickers or incoming intents (`ACTION_VIEW`/`ACTION_SEND`) are now tracked by
  path in `SaveMirror`. The periodic 2-second background tick and focus-lost (`WindowFocused(false)`) /
  exit handlers automatically flush dirty documents with paths to disk, and `SaveMirror.sync()`
  immediately streams changes back to the original SAF document URI via Android's `ContentResolver`.
- Resolved CI build failures across `vectorcraft` (`&Value` parameter type mismatch), `designcraft`,
  `gridcraft`, `deckcraft`, `effectcraft`, `filmcraft`, and `figcraft` caused by conflicting session fields.
- Restored PhotoCraft crash-recovery and periodic autosave on Android by wiring `RecoveryStore` services
  (`autosave`, `autosave_results`, `flush_autosaves`, `discard_autosave`, `recover`, and `adopt_autosave`)
  into the shell adapter, honoring the autosave interval and "Recover on launch" preferences in File Handling.
- Optimized SAF document staging using `ParcelFileDescriptor` with NIO zero-copy channel transfer
  and enabled PhotoCraft background worker jobs (`app.background_jobs = true`), drastically reducing
  file load latency on large image files such as 20MB PSDs.
- Typing on the soft keyboard lagged by up to half a second — a backspace above all: the letter
  stayed until the next scheduled frame. The keyboard's events are queued by the Activity and
  drained by the shell once per frame, but the shell's loop sleeps in `ALooper_pollOnce` while
  nothing touches the screen, and a queued event is not an input event: the loop woke only when
  egui's cursor blink came round (half a second) or the user touched the screen. Every queued
  keyboard event now wakes the loop the way an input event does — through `GameActivity.onKeyDown`,
  whose native glue calls `ALooper_wake` — with a `KEYCODE_UNKNOWN` event that winit reads as
  `Key::Unidentified` and no widget acts on. The character goes in the frame the key went down.
- A machine with no Android SDK could not build at all: the builder looked for one in
  `ANDROID_HOME` (or `~/Android/Sdk`) and stopped with "Android SDK platform directory not
  found". The builder now keeps its own SDK in `android-sdk/` beside the other build state and
  fetches it when the machine has none — the command-line tools (which carry `sdkmanager`), the
  compile platform and the build tools. An SDK that is already on the machine is used where it
  lies, and an NDK that is already installed is never fetched twice: it is a gigabyte unpacked,
  and the NDK is not tied to an SDK root.
- The first launch on a phone-class screen did not raise the interface to 150% in most apps, only in
  PhotoCraft. The launcher asks the Activity whether the device is a phone-class screen
  (`smallestScreenWidthDp < 600`, Android's own `sw600dp` boundary) over JNI, and the shared
  Activity had no such method: the call failed, the launcher took the answer for "unknown", and the
  scale was left alone. The method is in the shared Activity now, and FigCraft and CadCraft — which
  do have an interface scale, but had no entry in the builder's list of apps that raise it — are in
  it too. LightCraft has no interface scale at all and is left alone.
- A hardware trackpad's two-finger scroll did nothing at all. The backend told a mouse, a touchpad
  and a trackball apart by matching the device's whole source against one constant, but Android
  describes a device by a *bitmask*: a touchpad is `SOURCE_MOUSE | SOURCE_TOUCHPAD` (0x0010200a),
  which matches no constant, so the touchpad was taken for a finger on the glass and its scroll
  dragged the document around instead of scrolling it. The source is now read bit by bit (with the
  tool as the second hint, as Android's own documentation prescribes: a touchpad's events normally
  report only `SOURCE_MOUSE`, and `TOOL_TYPE_FINGER` is what tells it apart from a mouse). The
  touchpad's gesture is also read from the right place: Android reports it as a classified motion
  event whose pointer walks across the screen as the gesture goes on, and the distance it walked is
  the scroll — the same pixels, in the same direction, as a two-finger drag on the glass. A
  two-finger pinch is reported as a zoom, the way a browser reports it. The classification is a
  separate, tested function (`vendor/winit/src/platform_impl/android/device_kind.rs`, with a host
  test in `tests/android_device_kind.rs`).
- The soft keyboard rose over the document while it was being scrolled or drawn on. Android shows
  the keyboard whenever the focused text field asks for it, and the canvas — holding the caret —
  asked on every frame, so a two-finger scroll or a pen stroke raised it over the very page the
  user was working on. A scroll, a pan or a drawing gesture now surrenders the keyboard for the
  frame and asks the shell to put it away; the shell reads that request after the frame ran and
  calls the Activity's `imeHide`. A drag that belongs to a gesture no longer takes focus either,
  and on Android the canvas takes focus only from an explicit tap, so nothing raises the keyboard
  before the user touches the page. WordCraft only (see `lib/scrolling.sh`, `lib/ime.sh` and the
  launcher template in `lib/build-common.sh`).
- Backspace on the soft keyboard sometimes erased two characters. Some input methods report one
  press twice — through `deleteSurroundingText` and through a synthesised `sendKeyEvent` — and the
  second report, forwarded to the host view, reached winit as another real Backspace. The two
  reports of one press are now recognised as a pair (different methods, microseconds apart) and
  the second is dropped, while a quick double-tap — two presses of the same method — still erases
  twice; the synthesised key pair is consumed at the host view so it never reaches the native
  queue.
- A desktop window (Samsung DeX and the other freeform modes) cut the top off the interface. A
  desktop window carries a title bar of its own, which the system draws over the top of the window,
  and its inset is a *consuming* one: the system hands it to the app instead of applying it to the
  content, so edge to edge the app's interface was drawn under that title bar and the first row of
  it was hidden behind it. The content view is now padded by the title bar's height, which moves
  the native surface (and the interface on it) below it.
- A black bar along the top of a desktop window, which stayed there after the window was dragged
  back to normal. The padding that keeps the interface below the title bar above was read once —
  when the window took focus — and cached, but the platform reports that inset unreliably while a
  window is being resized: it hardcodes the height to the full frame during a drag
  (`InsetsSource.calculateInsets`) and only serves the caption source to *floating* windows
  (`InsetsPolicy`), so a window that had been floating kept being handed the old height after it
  was returned to fullscreen. The cached padding therefore stuck, and the strip it reserved showed
  the theme's black window background. The padding is driven by the window's own inset *dispatches*
  now, so it is recomputed in both directions, and the reserved strip is painted in the app's own
  theme colour, so even the one frame the system takes to settle is invisible.
- The two-finger pan scrolled the document the wrong way round. The content follows the fingers, but
  the backend reported the movement of the two fingers' midpoint with the opposite sign, so a slide
  up moved the document down.
- A mouse's wheel and a trackpad's scroll were told apart by the size of the amount instead of by
  the device. Android normalises a mouse's wheel to ±1 per notch and reports it on the scroll axes;
  a touchpad reports the same axes continuously, and only the device says which it is (the
  touchpad's own source, or a mouse whose tool is a finger). The size-based guess made a trackpad's
  scroll change speed the moment its amount grew past the threshold, and sent a mouse's notches
  down the smooth path now and then, where they fed the momentum and the page coasted on after the
  wheel had stopped — a scroll that felt like it happened twice. A mouse's notches are line deltas
  (one notch, one step, no coast) and a trackpad's are pixel deltas (smooth, and they coast).
- Save As and Export in WordCraft aborted the app the moment the system picker appeared. The
  picker was run on the frame thread, and that thread *is* the event-loop pump: while it was
  parked the app never received Android's `Suspended` lifecycle event, so the renderer never tore
  down its wgpu/Vulkan surface. On resume Android hands the app a new `ANativeWindow`, and creating
  a surface for a window that still had a live `VkSurfaceKHR` aborts the process with
  `AndroidSurface failed: ERROR_NATIVE_WINDOW_IN_USE_KHR` (confirmed in logcat; it killed the app
  on every save and export). WordCraft has an asynchronous dialog API for exactly this
  (`Services::file_dialog`, `#94`/`#511`), so the launcher now shows every dialog — Open,
  Insert/Change Picture, Add Font, recipients, Save As, Export, Print-to-PDF — through it on a
  worker thread while the frame loop keeps pumping; the app picks the answer up on a later frame.
  A new `saf::Bridge::pick_blocking_worker` waits on the worker without the frame-thread
  `NonblockingCallbacks` (the frame thread is alive now, so the glue's lifecycle callbacks complete
  on their own and input stays with winit's own iterator).
- WordCraft's Export opened the save dialog **twice**, and the first destination stayed a 0-byte
  file. The launcher set `Services::download`, the *browser's* save mechanism: with it set,
  `file.saveAs`/`file.exportPdf`/`file.exportPng` ignore the picked path and ask a second dialog
  for the bytes, while the first picker had already been opened for a path. Android saves like the
  desktop — the engine writes the picked path and `SaveMirror` commits that staged file to the
  chosen SAF document — so `download` (and `open_async`, which competed for the same answer queue)
  are no longer set, leaving exactly one dialog per action.
- A file handed to a **running** app was not opened until the user touched the screen. The system
  hands an opened file to the app as an intent; the Activity stages it and queues its private
  path, and the Rust shell drains that queue from `eframe::App::logic` — but egui runs a frame
  only when something asks it to, so an app sitting untouched never ran the frame that would
  have opened the file. The file appeared in the app's Recent list only after the next touch
  (the intent's own copy is queued, not lost), and the editor stayed on the previous document.
  The launcher now watches for it: a background thread asks the Activity how many files are
  waiting — a new `intentPending()` that counts the queue without taking anything off it — and
  asks egui for a repaint when there is one, so the file opens on its own. The file itself is
  still opened on the frame thread, in the same place as before. The watcher stops when the app
  shuts down, before Android releases the Activity's Java object, so it can never call into a
  dead Activity.
- Drag and drop now works in the PhotoCraft and VectorCraft adapters as well. The two custom
  adapters carry their own Activity, so the drop support that the generated launchers got (a
  window-wide `OnDragListener`, the staged copy, the persistable read/write grant) was missing
  there: a file dragged onto those apps was ignored. Both adapters now have the same drop path,
  and a dropped file lands in the same queue as a shared one, so it opens the same way.
- The drop listener now takes the drag's *enter* as well as its drop. The framework keeps a
  drag with the view that accepted the enter and hands the drop to that same view, so
  declining the enter swallowed the drop before it arrived — the file was never staged. The
  listener also asks for a persistable **write** grant first and falls back to a read grant:
  a dropped document is saved back to the place it came from, and the staged copy remembers
  the source URI, so a source that offered write access should get it.
- The PhotoCraft and VectorCraft adapters could not read the device's storage at all. Every other
  app declares `READ`/`WRITE_EXTERNAL_STORAGE` and "All files access" and asks for it once on the
  first launch; the two custom adapters declared none of it and never asked, so anything outside
  the app's own directories — the shared storage the file browsers work in, and the font folders
  the shell links — was unreadable there. They now carry the same permissions and the same
  one-time ask.
- On an app installed before this version, no font at all could be linked into the folders the
  apps' scanners read. That build linked each of those folders as a whole (`.fonts` →
  `/system/fonts`), and the leftover still reads as an existing folder, so it was kept — while
  every link made *inside* it failed silently with `EROFS`, its entries living on the read-only
  system partition, and no user font could ever show up there. The leftover is now recognised and
  replaced with the real folder of links on the first start. (The folder configuration the
  fontconfig readers get is rewritten on every start for the same reason: the list of folders
  grows — the user's own was added with this version — and a stale file would hide it from them.)
- The soft keyboard never came back after the back key had dismissed it. Android takes the back
  key as "hide the keyboard" and tells the app nothing, so the app kept believing the keyboard was
  up, and the next edit — a tap in the text, a caret moved by key — went unseen by the keyboard,
  which stayed down until the app was restarted. The Activity now watches the keyboard's own
  window insets and knows when it was dismissed; the shell reports every edit it sees, and the
  keyboard returns with it. The back key still hides the keyboard, and it stays hidden until the
  document is worked on again.
- The build could stop without saying why. A command that failed on its own — rather than one the
  script handled with `|| true`, an `if` or a `case` — ended the run with nothing but the last
  progress line in the log. The failing line is now named, so the log always says what went wrong.
- A build whose source checkout was incomplete failed with a message that pointed at the wrong
  thing. `git clone` can be interrupted — a killed build, or a full disk — after it has written
  the commit and the origin but before it has written the whole tree; Git then reports a perfectly
  normal origin and commit while most of the crates are missing from disk. The builder reused that
  checkout as if it were sound, and the run ended with "this app needs an app-specific adapter
  under adapters/<app>/", which is not what had happened. The checkout is now verified before it is
  reused, an incomplete one is named as such and re-cloned, and the clone itself goes into a
  scratch directory that is only moved into place once the tree has been checked — so an
  interrupted clone can never become the checkout a later build reuses. A clone that runs out of
  disk is now reported as such before it starts, and the space it needs is freed by name.

### Added
- The fonts in the user's own `Fonts` folder, treated like the device's own. `/storage/emulated/0/Fonts`
  (and `/sdcard/Fonts`, and a subfolder of either) is already where a user can drop faces; its files
  are now linked next to the system faces in the folder the apps' font scanners read and listed in
  the font configuration, so WordCraft, GridCraft, DeckCraft, DesignCraft, VectorCraft, PdfCraft,
  FigCraft, PhotoCraft and EffectCraft find them the way they find an installed one. The scanner
  folders are one real folder holding a link per file, with the other names
  (`$HOME/.local/share/fonts`, `$XDG_DATA_HOME/fonts`) linked *to* it: only a per-file folder can
  carry the device's faces and the user's together, and every name resolving to one folder is what
  keeps a scanner that reads two of them from listing every family twice. The links are kept
  current while the app runs — the folder is looked at every couple of seconds and re-linked only
  when something changed — so a font added later, or the "all files access" being granted after the
  app started (which is how the ask plays out: the launcher opens the settings screen, and the user
  answers while the app is already running), takes effect without relaunching. The shell logs what
  it found (`Android: user fonts: N file(s) …`), and says so in plain words when the folder cannot
  be read, since that is the one reason a font dropped there does not show up.

### Changed
- The interface size a phone-class screen gets on its first launch is now 85% for the canvas apps
  (GridCraft, DeckCraft, DesignCraft and FigCraft) and 75% for VectorCraft, instead of 150% for
  every app: those apps draw their work on a canvas, and a larger interface on a phone takes the
  room the drawing needs. The document apps (WordCraft, PdfCraft, CadCraft, SoundCraft,
  EffectCraft and FilmCraft) keep 150%, which is what a ribbon and a menu want.
- The apps ask for "All files access" again on every launch until it is granted. The ask used to
  happen once, and a dismissal was remembered as an answer that lasted, so an app installed before
  the ask existed never asked at all and could not read the device's storage. The permission is
  what the apps need for the shared storage (the font folders the shell links, the files they open
  and save), so the ask is repeated while it is missing and stops for good once it is granted.
- WordCraft, GridCraft and DeckCraft now start on their own homepage instead of an empty document.
  WordCraft shows Word's File › Home start screen (Blank document, the templates and Recent) and
  previously opened "The Open Studio Handbook", the sample document, which looked like it had
  loaded someone else's file; GridCraft and DeckCraft open the File tab's Backstage Home the way
  their desktop apps do. The generated launcher reaches each app's own public entry point right
  after the settings load — `WordApp::start_screen()` for WordCraft (its launch-with-no-document
  path, `#766`/`#1129`) and `open_backstage(Page::Home)` for the other two — so nothing the loader
  applied can overwrite it. In WordCraft, Esc or the back arrow then leaves an empty window and a
  document is opened only when the user picks Blank document, a template, or a file.

### Added
- Rust logging is installed on Android. The launchers (and the PhotoCraft/VectorCraft adapters)
  never called `android_logger::init_once`, so every `log::error!`/`log::info!` from the app, winit
  and eframe — including eframe's fatal `Exiting because of error: …` — vanished on device and only
  the hardest crashes (a raw panic) were visible. Now the whole log stream lands in logcat under the
  app's own tag.
- First-launch UI scaling for small screens. On a phone-class screen the desktop-sized interface is
  uncomfortably small, so the **first** launch of each app now raises its UI scale to 150 %. The
  decision uses Android's own form-factor test — `smallestScreenWidthDp < 600`, the same `sw600dp`
  boundary the platform and Play Store use to bucket tablets — exposed by a new
  `isPhoneDevice()` Activity method and read once over JNI (`device_is_phone`). It is the *shortest*
  screen edge in density-independent pixels, so it is stable across rotation, and it describes
  screen size rather than density (dpi): a large low-density tablet is not misread as a phone. Each
  app's scale key and representation differ (int percent, choice string, `f32` factor,
  `InterfaceSize` percentage), so the bootstrap is spelled out per app in
  `UI_SCALE_BOOTSTRAP` (`lib/build-common.sh`) and in the PhotoCraft/VectorCraft adapters. It runs
  only while the app's own settings file is absent, so a user who changes the scale later keeps
  their choice; tablets, and every later launch, are left exactly as the user set them. Covered:
  photocraft, vectorcraft, effectcraft, filmcraft, wordcraft, designcraft, deckcraft, soundcraft,
  gridcraft, pdfcraft. LightCraft, FigCraft and CadCraft persist no interface scale and are
  untouched. `GENERATOR_VERSION` 32 → 33 regenerates every launcher (35 for the file-dialog and
  blank-document changes above).
- The last unbounded lifecycle wait in the vendored `android_app_glue` is now bounded like the other
  three: `android_app_free`'s wait for `android_app->destroyed` (`ARTCRAFT_NONBLOCKING_CALLBACKS`)
  could stall the Java main thread during a pick. It still records `APP_CMD_DESTROY`; only the wait
  is skipped.
- Two-finger scrolling on touch screens, rebuilt the way a browser does it. A phone or tablet has no
  wheel, so a two-finger drag is the only way to scroll, and the gesture has to be *recognised* the
  way Chromium recognises a trackpad's: a two-finger slide scrolls, a two-finger pinch zooms. The
  vendored winit backend decides the gesture once, inside a deadzone, and sticks to it: a slide that
  moves the fingers' midpoint further than 12 physical pixels scrolls, a gesture that changes the
  distance between the fingers further than 36 pixels (and much further than it moves the midpoint)
  zooms, and anything in between does nothing at all — that deadzone is what keeps a scroll from
  turning into a slow zoom halfway through. The gesture is then reported the way a platform reports
  a trackpad's: the slide as continuous pixel deltas, applied 1:1 in physical pixels and 1.5× faster
  than the fingers move, and the pinch as a zoom factor. The fingers' touches are withheld while the
  gesture runs, and the first finger's touch is cancelled when the second lands, so egui never reads
  the same two fingers as a pinch of its own (which is what turned every scroll into a slow zoom)
  and the gesture never starts a text selection. The vendored egui-winit releases the pointer when
  that cancellation arrives but keeps the pointer's place, so the canvas still knows the pointer is
  over it. WordCraft gets an app-side guard (applied to its own source at build time) that ignores
  a press — and drops any drag already begun — while the gesture is in progress; it now watches for
  the backend's cancellation rather than egui's multi-touch state, which the backend keeps clear on
  purpose. All of this lives in `lib/scrolling.sh` and the vendored backends; comment out the one
  line in `lib/build-common.sh` that calls the script to build without the app-side guard for
  testing. It currently covers WordCraft only (the PdfCraft guard was removed at the owner's
  request).
- The flick's coast on Android. The app synthesises the momentum for the platforms that send none:
  macOS and Windows send their own, and so does Chromium on the web (which is why the web build
  must not make one as well). Android's touchscreen sends no momentum of its own for a two-finger
  pan, so the builder's `lib/scrolling.sh` no longer excludes Android: a flick coasts to a stop
  now, the way it does in a browser. Only the smooth deltas feed it, so a mouse's notches still
  land in one step.
- The Android system back button does something. winit's Android backend consumes
  `KEYCODE_BACK` (it reports the key as handled), so Android's own back handling never ran and
  the button reached the apps only as egui's `Key::BrowserBack` — which almost none of them
  listen for: the button did nothing at all. The shell now re-labels a back press as **Escape**,
  the key every app already treats as "close the dialog / cancel / go back", so the button gets
  its usual meaning with no app change. When the app consumed that Escape (a dialog closed, a
  panel went back) nothing else happens; when nothing consumed it, the app finishes, which is
  what back means on Android. The finish runs on the Java main thread (`Activity.finish()`),
  and the Activity's own `onDestroy` ends the process as it already did.
- Every app checks GitHub for a newer builder release and offers the download. Each APK carries
  the builder's own version (currently 0.2.0), and a few seconds after launch a background
  thread asks GitHub for the repository's latest release; when that version is newer, the user
  is told and offered the new APK. The check is unobtrusive — off the main thread, after a
  delay, and skipped once a version has been offered — and any failure (no network, a rate
  limit, an unexpected API shape) is logged and ignored. The shared checker is one Java class
  (`templates/android/UpdateCheck.java`) that the builder installs next to every launcher's
  Activity, so the generated launchers and the per-app adapters share it; the apps need the
  `INTERNET` permission for it.
- The back button no longer throws unsaved work away. On the desktop, closing a WordCraft window
  with unsaved changes asks first ("Do you want to quit without saving unsaved changes?"), and
  the Android back button now runs the very same check: the app's own `close_requested` opens its
  own prompt, and the app only ends once the user confirms ("Don't save", or the save finished).
  Apps without such a check close as they are. The prompt is the app's own, so it looks and
  behaves exactly like the desktop's.
- The apps use the fonts in the device's `Fonts` folder. A face dropped in
  `/storage/emulated/0/Fonts` (or `/sdcard/Fonts`) is now found by the apps' own font scanners —
  the ones WordCraft, GridCraft, DeckCraft, DesignCraft, VectorCraft and PdfCraft use for
  document and palette fonts — and by the interface's per-script fallback, so a script the device
  has no face for can be covered by a font the user supplied. The folder is read once the app has
  been granted "all files access", which the launcher already asks for. The scanner folders now
  hold a link per font file rather than a link to one folder, because they have to carry the
  device's faces and the user's at the same time; a face the user supplied replaces a system file
  of the same name.
- PdfCraft can use the device's own fonts as the interface fallback, as it does on the desktop.
  Its fallback face is an *installed* font, found through the fontconfig configuration the
  launcher writes, so the configuration now lists the device's font folders and the user's own
  one; it is rewritten on every start, so a configuration written by an earlier build is not
  left behind. The face itself is never embedded or shipped — it is read at runtime, like on the
  desktop.

### Added (in progress — hover not yet reaching winit)
- An Android pointer-icon bridge in the vendored `egui-winit`: `pointer_icon.rs` translates the
  `egui::CursorIcon` the interface resolves each frame into an `android.view.PointerIcon` and
  pushes it to the Activity's `InputEnabledSurfaceView` over JNI, so a mouse could show the
  native I-beam over text, a pointing hand over buttons, and resize arrows over splitters.
  winit's Android `set_cursor` is a no-op, so without this the icon never reaches the screen.
  The Activity handle is published once by each launcher (`android_activity.rs`) because
  `ndk-context` only carries the *Application*, and `Activity.getWindow()`/`startActivity()` are
  Activity-only. Verified on-device: the bridge runs and sets the icon on the surface view.
- A native link opener in the vendored `egui-winit`: `open_url.rs` opens a URL with an
  `ACTION_VIEW` intent on the Activity. egui's link widgets emit an `OpenUrl` command that
  `egui-winit` forwards to `webbrowser::open`, but that is compiled only under the `links`
  feature, which the ArtCraft apps leave off — so on Android a clicked link did nothing. The
  app's own `open_url` service is likewise unset on Android, so every link path ends here.

### Fixed
- PdfCraft refused Arabic text on Android ("The PDF text fonts can't draw “ا” (U+0627)"), while
  the desktop build accepted it. The apps that embed the shared craft-fonts (WordCraft, PdfCraft)
  read them from a `CRAFT_FONTS_DIR` build input, and the Android build never provided one: it
  only made an empty directory where the checkout was expected, so the apps were built with no
  face of their own for a script the device may not have — and the PDF text writer, which shapes
  Arabic with that face, refused the text instead of writing it. The builder now fetches the
  craft-fonts checkout (like the apps' own sources, and keeping a local pin) and points the
  build at it, so the same faces the desktop release embeds are embedded here. The APKs grow by
  the faces they now carry (PdfCraft +25 MB, WordCraft +42 MB, which also embeds the Chinese
  interface face).
- PdfCraft's last-resort interface face — an *installed* font, found on the desktop by name —
  was never found on Android, because the device ships the same scripts under other names: its
  Arabic face is the Naskh family and the Indic scripts are variable fonts, while the app looks
  for `NotoSansArabic-Regular.ttf` and the static Indic faces. The launcher now offers the
  device's faces under those names in the font configuration, so the app finds the face it wants
  (the log line `interface font fallback: …` says which). The interface's own per-script fallback
  gained the variable-font files, so the Indic scripts (Hindi, Telugu, Sinhala, Tamil) are drawn
  by the device's own faces instead of blank boxes.
- CADCraft could not open any drawing on Android. The engine keeps its file formats out on
  purpose — it stays I/O-agnostic and the *host* installs them as global hooks (the DXF/DWG/SVG
  reader and writer, the plotter, and the entity codec AutoLISP's `entget`/`entmake`/`entmod`
  use). The desktop host does that (see `apps/cadcraft/src/main.rs`); the Android launcher did
  not, and `cadcraft-io` was not even among its dependencies, so the engine had no file formats at
  all: every open failed with *"invalid parameters for 'open': file formats are not available in
  this build"*. The launcher now installs the same hooks and links the crate that provides them.
  A DXF and a real Autodesk DWG both open and draw, and Save As writes a valid DXF (AC1015) to
  the chosen document.
- **An app died when it came back from the Android file picker.** Picking a file parks the frame
  thread (it is the event-loop pump), so the app's window is torn down and recreated while it
  waits; the queued "window destroyed / window created" events are then handled in one go, and the
  paint that follows finds a surface whose window no longer exists. egui-wgpu's answer to a lost
  surface is to *recreate* it, and on Android that means asking Vulkan for a second surface for a
  window that still has the first — which Vulkan refuses (`ERROR_NATIVE_WINDOW_IN_USE_KHR`), and
  wgpu turns that refusal into a panic (`expect("AndroidSurface failed")`), so the whole process
  went down. The builder now ships a patched egui-wgpu: the lost surface is released *before* a
  new one is made, the device is waited for so the old Vulkan handle is really gone, and on
  Android the dead surface is simply dropped — `set_window` runs for the current window every
  frame, so the next frame makes a fresh surface for the window that is there now. CADCraft
  (DXF/DWG), WordCraft and every other app survive the picker round trip now.
- WordCraft's file dialog could not be built against the current upstream source. The answer
  type changed from a single optional path (`Result<Option<String>, String>`) to the list of
  picked paths, empty when the user cancelled (`Result<Vec<String>, String>`), so that "Add
  Font…" can pick several files. The launcher's picker now answers in whichever shape the
  checked-out source declares, and the request it sends no longer names the open variants, so
  it keeps building against a source that has one open request or several.
- EffectCraft could not import or export at all: every import said "media import is not available in
  this build" and every export "export is not available in this build". The generated launcher
  started a bare `Session::new()`, which has no media importer, no Render Queue exporter, no
  expression engine and no scripting; the desktop app instead builds `effectcraft_host::session()`.
  The launcher now depends on `effectcraft-host` and constructs `EffectcraftApp::new(
  effectcraft_host::session())`, so its import and export paths work. `effectcraft-host` compiles
  for `aarch64-linux-android`; the APK grows (64 MB → 87 MB) because the media decoders and the
  export encoders are now linked in. (This was mis-diagnosed earlier as a blocking-picker ANR; the
  picker itself was fine — the staged file even landed in `media/` — but the session had nothing to
  import it with.)
- SoundCraft's launcher icon came out as a white background with a nearly all-black bird, instead
  of the white bird on teal it should be. Two bugs in `lib/app_icons.py` combined. (1) Its
  `icon.svg` draws the artwork inside a rounded-square clip (transparent corners), and `magick` was
  rasterising the SVG onto an opaque white canvas: those corners came back white, which then won the
  brand-colour vote, so the adaptive background layer was generated `#FFFFFF` and the legacy mipmaps
  carried white corner wedges; `_raster`/`_write_legacy` now pass `-background none`. (2) The
  adaptive foreground is the artwork with its background flood-filled away, seeded at the image
  edges — but SoundCraft's bird runs off the bottom and right edges (its paths reach `520,520` and
  `530,540`, past the 512 viewBox), so the flood reached the *cream body* and erased it, leaving only
  the dark outline. `_strip_background` now identifies the background colour (the most common opaque
  border colour) and clears a flooded pixel only when it actually *was* that colour, so a flood that
  bleeds into edge-touching artwork leaves the artwork intact. Result: background `#14A9C4`, a cream
  `#EFE9DC` bird on it, and transparent legacy corners. Other apps are unchanged (their artwork does
  not touch the edge); the white-corner fix still applies to every rounded-square icon.

### Known gap
- The pointer icon still does not change on a real mouse hover. `apply_cursor` (and
  `on_cursor_moved`) are never reached, so hover is not arriving at winit even though the
  Activity's window receives it (the framework logs `updatePointerIcon`). GameActivity attaches
  an `OnGenericMotionListener` to the `InputEnabledSurfaceView`, but the view appears not to
  receive hover; to be continued.

## v36 — 2026-10-19

### Fixed
- The API-33 guard on `AMotionEvent_getActionButton` is back in the vendored
  `android-activity` crate. It had been reverted to the committed crate during the mouse
  input rollback, which restored the direct `ndk_sys` reference to a function Android only
  added in 13 — so native-activity builds again died at launch on Android 12L and below with
  `cannot locate symbol "AMotionEvent_getActionButton"` (the report came from gridcraft,
  0.8.0). The call is resolved once through `libc::dlsym` against `RTLD_DEFAULT` and cached
  in a `OnceLock`; a device without the symbol reports an unknown button, exactly as the
  v33 fix behaved. GameActivity builds never read the symbol (the button arrives from the
  Java event), so nothing changes there.

### Changed
- One more mouse-wheel fix, in the other backend: the GameActivity pointer struct only
  carries the axes the app asks for, and only X/Y are on by default, so the scroll wheels
  read as zero and no `MouseWheel` was ever emitted. `EventLoop::new` now enables
  `Axis::Vscroll` and `Axis::Hscroll` before the loop starts (lock-safe, unlike a call from
  inside the event callback), which restores wheel scrolling.
- Rebuilt gridcraft, photocraft, vectorcraft and designcraft. All four launch cold with no
  `UnsatisfiedLinkError`, and `readelf` finds zero references to `AMotionEvent_getActionButton`
  in the shipped libraries.

## v35 — 2026-10-19

### Fixed
- The nine apps that had not been rebuilt since the v33 symbol fix still refused to start on
  Android 12L and below, dying at launch with "cannot locate symbol
  AMotionEvent_getActionButton". Their shipped APKs predated the guard (and, on those trees,
  the GameActivity migration), so lifting them onto the current builder was the whole fix —
  every build re-vendors the patched `android-activity` crate fresh. All nine, plus the four
  already-current gate apps rebuilt in the same pass, now carry **zero** references to the
  symbol, checked with `readelf` against the libraries inside the shipped APKs.

### Restored
- Pointer-icon support is back — the v34 removal is undone — now living in the shared vendored
  `egui-winit`, which is where the hook always belonged: each build vendors that one crate, so
  a per-app copy is just a build artifact the next build overwrites, which is exactly how the
  five wired copies were quietly dropped in the first place. A mouse or trackpad over any app
  again gets the interface's cursor shape (text I-beam over text, resize arrows on an edge,
  grabbing hand while dragging, and so on). Every app was rebuilt with it, so all thirteen
  APKs carry the bridge.

## v34 — 2026-10-19

### Fixed
- Light/dark following never worked on a device. v31's per-frame poll reads
  `Configuration.uiMode`, which is a Java **`int`**, but the JNI field lookup asked for a
  `byte` — so `GetFieldID` failed with `NoSuchFieldError` on every frame and the theme read
  always came back `None`, leaving the interface on its start-up theme. The field is now read
  with its real signature (`I`), so an app opened in dark mode is dark from the first frame,
  and switching the phone's appearance while the app runs re-themes it live.
- PhotoCraft's canvas fell back to the CPU compositor and Help › System Info reported no
  adapter ("unknown"). The adapter never handed eframe's render state to the app. It now
  requests the GPU renderer explicitly (Vulkan first, GLES as fallback), widens the device to
  the adapter's own texture and buffer limits like the desktop build, records the adapter for
  System Info, and starts the wgpu canvas — a device that reports a CPU adapter still falls
  back honestly.

### Removed
- The pointer-icon experiment (forwarding egui's cursor shape to Android's system pointer
  icon over JNI). It had already been dropped from the shared vendored `egui-winit`, but the
  five app trees vendored while it lived (vectorcraft, lightcraft, effectcraft, designcraft,
  soundcraft) still carried it wired — those copies are back to the committed crate, and the
  duplicated empty "Mouse and trackpad" heading it left in the README is gone too.
  *(Reverted in v35: the hook now sits in the shared vendored crate, so no build can drop it
  again.)*

## v33 — 2026-10-19

### Fixed
- The apps no longer refuse to start on Android 12L and below, dying at launch with a
  "cannot locate symbol AMotionEvent_getActionButton" link error. The mouse/scroll input
  path reads the button a press or release refers to through the NDK function
  `AMotionEvent_getActionButton`, which Android only added in API 33 (Android 13). The
  direct reference kept that symbol in the library's dynamic table, so on every older
  device the dynamic linker refused to load the whole app before a single frame was
  drawn. The call is now resolved once through `dlsym` — the same way the platform
  resolves it — and devices without it report an unknown button: no button identity was
  reported there before either (the field cannot exist on those versions), so click
  handling behaves exactly as it did, and every other input field keeps working.
  GameActivity builds were never affected — that backend receives the button from the
  Java event, which exists since API 23 — but the guard covers the vendored
  android-activity crate's native-activity path too, and all shipped APKs now carry no
  reference to the symbol at all (verified with `readelf`).

## v32 — 2026-10-19

### Changed
- Every app now runs its Android lifecycle through GameActivity (`androidx.games:games-activity`
  4.4.0) instead of a framework NativeActivity, and the Gradle Android Plugin assembles the APK
  instead of cargo-apk. GameActivity is the Activity Google maintains for native apps (NativeActivity
  receives no updates), and its Java half is what hosts the soft keyboard's input connection against
  a real view — the decor-view path never did that reliably. The Rust side of android-activity gains
  the matching `game-activity` backend, into which the builder's earlier patches were ported: the
  sync-picker ANR fix now bounds all three of the glue's JVM-thread waits (window handoff,
  activity-state handoff and save-state — each a two-second timed wait), so a blocking file pick
  still cannot freeze the UI thread's callbacks into an ANR; `show_soft_input`/`hide_soft_input`
  prefer the Activity's imeShow/imeHide bridge before falling through to the stock calls; and
  `set_nonblocking_callbacks` — the switch the SAF bridge flips around each pick — is exported for
  the new backend too. The three Java activities extend GameActivity accordingly and gain the
  kill-on-finish guard GameActivity needs: its destroy handshake waits for a native loop that a
  stopped eframe can no longer service, so a finishing activity takes the process down with it
  after waiting out a bounded grace period (a non-finishing destroy — the configuration-change
  path — stays on the normal flow).
- Because GameActivity extends AppCompatActivity, it refuses to inflate under the framework-only
  theme every app used (the launch would die with "You need to use a Theme.AppCompat theme"). The
  generated manifest now points at `Theme.Artcraft`, a generated AppCompat theme that keeps
  `windowFullscreen` — the flag the edge-to-edge code builds on — plus a black first frame before
  the Rust UI draws.
- The build itself: cargo-ndk compiles the native library into a `jniLibs` tree, and a generated
  single-module Gradle project (Gradle 8.11.1 wrapper, AGP 8.7.3) packages it with the Activity,
  the resources and a freshly generated `AndroidManifest.xml` — parsed from the same Cargo metadata
  tables cargo-apk2 read, so intent filters, permissions and activity attributes carry over one to
  one, and libraries are stored uncompressed and page-aligned for direct loading. The APKs keep the
  exact `versionCode` formula, `applicationId` and keystore of the cargo-apk era, so the in-app
  Updater accepts them as ordinary updates of the same app; signing moved from cargo-apk
  environment variables into Gradle's signingConfigs, fed the same builder key through the
  environment at build time. Launchers also request the GPU renderer explicitly — Vulkan first,
  GLES as the fallback for devices without usable Vulkan — and the adapter's real texture and
  buffer limits (the values the desktop apps use), so wide and tall documents stay on the GPU
  compositor instead of falling to egui's 8192 px CPU path. The generator version moves to **31**
  so every generated launcher regenerates.
- Gradle needs a JDK in the 17–23 range; the builder uses the first one it finds and, when there
  is none, downloads Temurin 21 into `.cache/jdk/` once (the signing keystore itself is still
  created with the system `keytool`).
- The photocraft and vectorcraft adapters now declare the same full `configChanges` set as the
  generated launchers, `uiMode` included: v31's live light/dark switching polls the theme every
  frame, but without `uiMode` in the manifest a quick-settings toggle would instead have recreated
  those two apps' activities mid-session.

### Removed
- The cargo-apk/cargo-apk2 dependency: neither tool is invoked or checked for any more.

## v31 — 2026-10-19

### Added
- The interface follows the phone's light/dark setting. Every app defaults to `Appearance`
  → system (the upstream default), which egui resolves against the OS theme it receives —
  but on Android that theme never arrived: every other platform delivers it as a winit
  `ThemeChanged` event, and Android's backend doesn't emit it, so an app opened in the
  wrong mode stayed there for the whole session. The vendored egui-winit now reads the
  Activity's `Configuration.uiMode` over JNI once per frame — the same bridge pattern the
  clipboard backend uses — and fills `RawInput::system_theme`, the identical field the
  desktop path fills, so the cold start is already correct. A quick-settings toggle then
  switches live: `uiMode` is in the manifest's `configChanges` (no restart), and the
  focus round-trip through the shade schedules the frame that carries the new value
  before the app is visible again — with the per-frame system-bar icon sync following it
  into the new theme. The mapping from the `uiMode` night bits to egui's theme is a pure
  function, unit-tested alongside the module it lives in.

## v30 — 2026-10-19

### Added
- Adaptive launcher icons for every app. Modern launchers mask icons into circles, squircles and
  rounded squares, and Android 13+ can tint them with the wallpaper's palette (Material You
  themed icons) — neither works properly with a flat square PNG, which is why the icons looked
  out of place when themed. Every app now ships the two-layer adaptive icon the platform expects:
  the brand colour as a solid `ic_launcher_background` vector, the artwork as
  `ic_launcher_foreground` with its own solid background flood-filled away from the edges (so the
  layer is the artwork silhouette on transparency, scaled to 66.56/108 of the canvas — inside the
  launcher's visible safe zone), and the same foreground registered as the `<monochrome>` layer,
  which gives themed icons a clean single-tint silhouette instead of a tinted square. The flat
  density mipmaps remain for file managers and other surfaces that still read them. The artwork
  source is the app's own vector icon where one exists (rasterised through ImageMagick), else its
  largest own PNG; an explicit `-appicon` goes through the same pipeline. A new shared module,
  `lib/app_icons.py`, owns the whole pipeline, and photocraft's adapter and the generated
  launchers (plus vectorcraft's committed `res/`) all produce identical resources from it. The
  generator version moves to **30** so every generated launcher regenerates.

### Removed
- The mouse pointer-shape experiment from the unreleased v29 work is gone again: it never showed
  the contextual shapes on real devices, so the vendored egui-winit is back to its prior state
  (the clipboard backend untouched) and mouse hover, wheel and secondary-click *events* remain
  the supported pointer surface.

## v29 — 2026-10-19

### Added
- Files can be dragged onto a running app. Dragging a file out of the Files app (or a browser)
  onto the window in split-screen opens it in the app, through the identical path a share-sheet
  delivery uses: the Activity's drag listener collects the dropped URIs, copies them into
  app-private staging off the UI thread with the same helpers, and queues the paths for the Rust
  shell's `intentPoll()`. Drags that carry no files are declined so they don't interfere with
  anything else on the window, and a persistable read grant is requested when the source offered
  one — drag grants are otherwise temporary, so without it the copy can race the end of the drag
  (the system file picker's drags do offer persistable access).
- WordCraft prints through Android's native print system. The print flow gains a printer named
  "Android print system" — the one entry a device-side printer list cannot have, because Android
  keeps enumeration inside the system print dialog so that Wi-Fi, network and cloud print
  services are all reachable. Selecting it (auto-selected on first discovery) and submitting
  opens the real dialog, where printer, paper size, duplex, color and page range live; the PDF
  WordCraft already collated — range, copies and settings baked in — is staged in the cache
  directory and served to the spooler unchanged by a `PrintDocumentAdapter` that reports the page
  count from the system's own `PdfRenderer` and streams the full document on write (a sub-range
  picked in the dialog itself falls back to the full document, which the app's own range settings
  have already shaped). The submission answers synchronously over the SAF bridge, so the app
  reports "Print job submitted." or the failure text where the desktop spooler's feedback
  appears. `templates/android/saf_bridge.rs` grew `print_document`, and the bridge marker moves
  to **V3**.

### Changed
- Apps present edge-to-edge: the interface draws behind transparent system bars, and the bar
  icons (clock, battery, navigation) switch between light and dark with the app's own theme each
  frame, so a light document no longer sits under white-on-white status icons. This is the
  platform equivalent of AndroidX's `enableEdgeToEdge`, without the AndroidX dependency:
  `Window.setDecorFitsSystemWindows(false)` with transparent bar colors on Android 11+, the
  legacy layout flags below it. The Rust shell reports the active egui theme to the Activity at
  the end of each frame (`sync_bar_appearance`; egui 0.36's `Context::theme()` is the source of
  truth, `Context::style()` no longer exists), and the Activity applies the matching
  `APPEARANCE_LIGHT_STATUS_BAR`/`APPEARANCE_LIGHT_NAVIGATION_BAR` window flags — or the legacy
  `SYSTEM_UI_FLAG_*` equivalents on Android 9 and 10, carried across the replace-style set
  instead of dropped. Selection handles and the long-press toolbar are suppressed over the
  native surface, where they otherwise appear on every touch with no text field to belong to.
- Hardware acceleration needed no intervention; this release verifies that rather than changing
  it. cargo-apk2 emits no `android:hardwareAccelerated` attribute, so the platform default — on
  for every `targetSdkVersion` of 14 and later, and every app here targets at least 28 — applies,
  and the fullscreen `DeviceDefault` theme doesn't disable it. The rendering path was always
  native (a `SurfaceView` window surface driven by wgpu); the builder has no WebView anywhere,
  so WebView-only tuning such as `offscreenPreRaster` has nothing to apply to. Glyphs are
  rasterized by epaint on the CPU as on every other egui platform and composited on the GPU
  after that.
- The generator version moves to 29, so every generated launcher regenerates on its next build
  and picks all of this up. The two adapters (PhotoCraft, VectorCraft) ship their own launcher
  sources and were untouched by the template changes.

Everything above is build-verified (each stage gated on three randomly chosen apps, including
WordCraft for the print work); on-device confirmation is pending — the test device was unplugged
during this batch.

## v26 — 2026-10-18

### Fixed
- GridCraft's text renders in Arabic, Hebrew, CJK and the other non-Latin scripts. Every launcher
  hands epaint the device's own faces (`/system/fonts`) as the last fallback of each active font
  *family*, but the fallback was appended only to the families that existed on the pass it ran, and
  a family born later was never given it: `Context::add_font` does nothing for a face whose key the
  definition set already holds, so once the face was recorded it could no longer reach a family
  introduced afterwards. Most apps draw with egui's default `Proportional` family, which exists from
  the first pass and therefore got the fallback; GridCraft draws the interface and every cell with
  its own named families (`FontFamily::Name("ui")`, `"cell"`, the per-workbook `fnt:…` roles), which
  it binds in a `set_fonts` call applied on the *next* pass — after the fallback had already been
  appended to the defaults. The shell now checks the fallback per family each frame and re-adds it
  (under a fresh key) to any family still missing it, so a family bound at any point ends up with
  the system faces as its lowest-priority entries. The change lives in the shared
  `templates/android/system_fonts.rs`, which every launcher — generated and adapter — compiles.

## v25 — 2026-10-18

### Fixed
- Arabic, Hebrew, Chinese/Japanese/Korean and the other non-Latin scripts render again. v23 moved the
  *font environment* to the external app directory along with the app's data, but Android's emulated
  external storage (`/storage/emulated/0/...`, a FUSE/sdcardfs mount) refuses `symlink`, so the links
  that expose `/system/fonts` as `$HOME/.fonts`, `$HOME/.local/share/fonts` and `$XDG_DATA_HOME/fonts`
  were never created — silently, because the failure is discarded. Every app whose font scanner reads
  those directories (which is how the apps see any system face on Android, since `/usr/share/fonts`
  does not exist there) found no font for a script its own embedded faces lack, and the interface and
  documents showed blank boxes. Verified on device: the links were missing in every installed app, and
  `symlink` succeeds under `/data/...` but fails with `Permission denied` under `/sdcard`.
- The font environment is now built on **internal** storage, where the links can be made. Fonts are
  not user data, so nothing user-visible moves: the settings/preferences/recent list keep living in
  `.../Android/data/<pkg>/files/config` and the SAF working copies in `.../files/media`, both still
  reachable over USB/MTP. Only `HOME` and `XDG_DATA_HOME` (the roots the font scanners read, plus
  per-app dotfiles and fontconfig's cache) stay internal; `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` stay
  external. The generated launchers and both adapters set `HOME`/`XDG_DATA_HOME` in one place
  (`system_fonts::install_font_env`) so the two can no longer disagree about which storage they use.

## v24 — 2026-10-17

### Changed
- Clearing the recent-files list now removes the document's staged working copy as well. Importing
  a PDF (or any file) through Storage Access Framework copies it to
  `/storage/emulated/0/Android/data/com.artcraft.<app>/files/media/<document name>`, and it used to
  stay there forever after the app forgot it. The shell now keeps a snapshot of the recent list each
  frame: when an entry leaves it — Clear Recent, the × on one entry, or the capped list aging it
  out — and no open document is showing that file, the staged copy under `media/` is deleted. Only
  direct children of the app's own `media/` directory are ever touched, so nothing else on the
  device can be lost. The `uri:` mapping in SharedPreferences is deliberately kept: re-opening the
  same document reuses the same path (the SAF original is recopied) and a save still reaches the
  document the user picked. The staged copy is a cache — the original lives at its SAF location —
  so nothing but the cache is ever lost.
- A one-time sweep, half a minute after start, deletes staged copies no longer referred to by
  anything: leftovers from before this change, or from an app Android killed mid-session. It waits
  for session restore, spares files written in the last minute (a document may be mid-import),
  skips files the recent list or an open document still names, and — like the per-entry pruning —
  only ever deletes direct children of `media/`.
- A name whose staged copy was deleted is free again: `newStageFile` reuses the path instead of
  adding a ` (2)` suffix when the previously mapped file no longer exists, so a cleared document
  that is imported again lands on its original, suffix-free path. A different, still-existing file
  with the same name keeps the suffix as before.
- Enabled for pdfCraft, PhotoCraft and VectorCraft, and for CadCraft's next build; the remaining
  generated apps switch it on one by one as their recent-list recipes are added (an app without a
  recipe simply keeps everything under `media/`, the old behaviour).

## v23 — 2026-10-17

### Changed
- Release and debug APKs are now named after the version the app's own source declares, e.g.
  `output/vectorcraft-0.12.0.apk` (debug: `output/vectorcraft-0.12.0-debug.apk`). The builder reads
  the workspace `[workspace.package] version` from the checkout — the same value cargo writes into
  the APK's `versionName` — so the file name always matches the build without anyone remembering to
  pass it in. Sources that do not pin a workspace version fall back to the launcher crate's version.
- The Android package is `com.artcraft.<app>` for all thirteen apps (the eleven generated launchers
  and the two adapters, PhotoCraft and VectorCraft). Because the package id changes, Android treats
  each build as a new app: uninstall the previous `ai.storyteller.*` installation before testing, or
  it will sit next to the new one.
- App data now lives in the app-specific external directory
  `/storage/emulated/0/Android/data/com.artcraft.<app>/files` (settings, preferences, recent files,
  autosaves, caches, the fontconfig cache and the working copy of every document opened through
  Storage Access Framework) instead of the internal `/data/user/0/<pkg>/files`,
  which a phone file manager or a connected computer cannot reach. The internal directory is kept as
  a fallback for devices without external storage. Android 11+ still hides `Android/data` from the
  on-device file picker; it is visible over USB/MTP.

### Fixed
- Two builds running at the same time no longer destroy each other. They share the launcher crate
  under `source/<app>/apps/<app>-android`: `prepare.sh` replaces it, and the other run's `javac`
  inherits its working directory from that same directory, so the second build deleted the first
  one's working directory and it died with `java.lang.Error: Properties init: Could not determine
  current working directory`. A lock now refuses the second build up front and names the build that
  holds it.
- A document opened "with" an ArtCraft app is now staged at
  `/storage/emulated/0/Android/data/com.artcraft.<app>/files/media/<document name>` (internal storage
  only as the fallback). It used to be hidden in the internal `/data/user/0/<pkg>/files/...`
  directory — invisible to a phone file manager or a connected computer — inside a random per-open
  subdirectory such as `.../files/PhotoCraft/saf/<uuid>/example.psd`, so the working copy piled up
  under meaningless names. The same document now always maps to the same path; if the name is already
  taken by a different document, a ` (2)`-style suffix is added rather than sharing the file, because
  the staged path is what maps a save back to the document the user picked.
- "Open with" now actually offers the right ArtCraft app. The intent filters were present all along,
  but many MIME types were spelled the way the IANA register names them rather than the way Android's
  MIME database does, and Android matches types literally: `.psd` is `image/x-photoshop` (not
  `image/vnd.adobe.photoshop`), `.bmp` is `image/x-ms-bmp`, `.raf` is `image/x-fuji-raf`, `.rtf` is
  `text/rtf`, `.csv` is `text/comma-separated-values`, the `.docm`/`.dotm`/`.xlsm`/`.xltm`/`.xlsb`
  macro formats use a lower-case `macroenabled`, and so on. `lib/file_associations.py` now lists the
  Android-reported type for every extension plus the registered alias, and the mapping is documented
  against AOSP's `android.mime.types`/`MimeUtils` and a live device's `MediaStore`.
- "Open with" offered every ArtCraft app for *every* file, APKs included. A `<data>` element that
  declares a scheme and a `pathPattern` but no `android:host` does not constrain the path at all:
  AOSP `IntentFilter.matchData` only performs the path check inside its `if (mDataAuthorities !=
  null)` branch, so the element degenerated into a scheme-only match and claimed every `file://`
  and `content://` URI. Path elements now also declare `android:host="*"`, which matches any
  authority while the path pattern does the real work, and the extension glob escapes its dot
  (`.*\.psd`) so it cannot match an unrelated name that merely ends in the same letters.
- Files whose extension Android does not know (`.psb`, `.cr3`, `.ora`, the apps' own project
  formats) are reported as `application/octet-stream`, which a filter that declares only a path
  pattern does not match, because a filter that declares a MIME type makes the intent have to carry
  one. A second, separate VIEW-only filter pairs the same path patterns with `mimeType="*/*"`, so a
  typed chooser entry matches while the path pattern keeps the app off unrelated files. It is
  VIEW-only because a share intent carries its file in `EXTRA_STREAM` and has no data URI, so it can
  never match a filter that declares a scheme; shares of known types are served by the type-only
  `SEND` filter.

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft

## v20 — 2026-10-17

### Added
- Every app can now be opened from another app ("Open with") or receive a shared file. Each
  launcher and both adapters declare Android `VIEW` and `SEND`/`SEND_MULTIPLE` intent filters for
  the document types that app handles — PDF with pdfcraft, designcraft and vectorcraft, Word
  documents with wordcraft, spreadsheets with gridcraft, images and camera raw with photocraft and
  lightcraft, video and audio with filmcraft and soundcraft, and so on. The table lives in one place
  (`lib/file_associations.py`) and is applied to the generated launchers and to the adapters'
  copied manifests, so the associations cannot drift. Filters cover `content://` URIs by MIME type,
  `file://`/`content://` URIs by file-name pattern (for the apps' own project formats, which have no
  registered MIME), and `http`/`https` links for browser-opened documents.
- A file handed to the app is copied into app-private staging off the UI thread and opened exactly
  like a file chosen in the app's own open picker: the async-inbox apps (designcraft, deckcraft,
  gridcraft, wordcraft) receive it in their import inbox, the others are asked to open the staged
  path through their document API (`open_path`, `import::open`, `session.open`, …). This also works
  while the app is already running, through `onNewIntent`.

### Fixed
- A transient upstream compile error on an app's `main` can no longer block the build. Before
  preparing the launcher, the builder runs a small repair step that fills in the `export_presets`
  field PhotoCraft's preset loader is missing (the regression introduced by "load each persisted
  preset set independently"). The step only edits the file while the broken text is present, so it
  is a no-op once upstream ships the fix, and the entry can then be deleted; the checkout is still
  never reset.

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft

## v19 — 2026-10-17

### Fixed
- The Android soft keyboard is now reliable in every app. Previously it appeared only sometimes,
  flashed in and out, and never delivered typed text: a bare `NativeActivity` has no editable view,
  so `InputMethodManager.showSoftInput` on the decor view was unreliable, and egui's own text input
  had no `InputConnection` to receive commits. Every Activity now installs a tiny, invisible but
  focusable `EditText` (`templates/android/SafActivity.java` and both adapter Activities) whose real
  `InputConnection` makes the keyboard show/dismiss on request and reports every
  commit/composition/delete/editor-action. A shared Rust module
  (`templates/android/ime_bridge.rs`, copied into every generated launcher and both adapters) drains
  those events each frame into egui `ImeEvent`s for the focused `TextEdit`.
- The vendored `android-activity` now forwards `show_soft_input`/`hide_soft_input` to the Activity's
  `imeShow`/`imeHide` when it provides them, instead of always targeting the decor view; and the
  vendored `egui-winit` no longer toggles `set_ime_allowed(false)+(true)` to interrupt a composition
  on Android, which mapped to a visible hide+show flash. The Activity also sets
  `SOFT_INPUT_ADJUST_NOTHING` so the keyboard overlays the fullscreen native surface instead of
  resizing or panning it.
- The keyboard host stays invisible to every other input path. It never consumes touch or hardware key
  events, so those keep flowing to the native surface and `winit` exactly as before (physical
  keyboards still work). Its Enter/newline key and editor action are translated to an egui
  `Key::Enter`, which egui applies per field: a newline in multiline fields, confirm-and-dismiss in
  single-line ones (a bare committed newline would otherwise be dropped by egui). On devices that
  ignore an immediate `showSoftInput`, the Activity retries once after focus and layout settle.

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft

## v18 — 2026-10-17

### Added
- Android system fonts are now used, alongside each app's own embedded fonts, so non-Latin scripts
  (CJK, Arabic, Hebrew, Devanagari and the other Indic scripts, Thai, Khmer, Myanmar, Georgian,
  Armenian, Ethiopic, …) render instead of appearing as blank boxes. A shared module
  (`templates/android/system_fonts.rs`, copied into every generated launcher and both adapters)
  installs the device's `/system/fonts` (`/product/fonts`) faces as *lowest*-priority epaint
  fallbacks, so they are only consulted for glyphs the app's own fonts lack. A script is skipped when
  the app can already draw it, and each face is read at most once.
- The system font folder is also linked into the per-user locations the apps' own font scanners
  expect (`$HOME/.fonts`, `$HOME/.local/share/fonts`, `$XDG_DATA_HOME/fonts`) and a minimal
  `fonts.conf` is written with `FONTCONFIG_FILE`/`FONTCONFIG_PATH`, so their document/palette
  rendering (wordcraft, gridcraft, deckcraft, designcraft, vectorcraft, pdfcraft, …) sees the same
  fonts as the interface.

### Fixed
- Generated launcher and adapter `ui` methods install the fallbacks each frame, so an app that
  replaces its font definitions with `set_fonts` (e.g. on a language switch) does not drop them.

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft

## v16 — 2026-10-16

### Added
- Settings, preferences and recent-file lists are saved on Android and restored at startup for
  every app except PhotoCraft (whose adapter already saved). Each app's own load/save calls are
  reproduced in the generated launcher and pointed at the app's private files directory
  (`<internal data>/config`), which survives restarts and app updates. Saves happen on a short
  timer, when the window loses focus (the app is being backgrounded), and at exit.
- The platform path helpers (`HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`) are
  pointed at app-private storage before the app starts, so files the apps write themselves
  (autosave, crash-recovery copies, caches) land there too.

### Fixed
- FigCraft launcher against the current `DialogFuture` picker API.
- DesignCraft launcher against the current `Services::download` signature (it now returns a
  `Result`, so a picker that fails to start is reported instead of only logged).

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft

## v13 — 2026-10-15

### Fixed
- Crash on hardware keyboard/mouse connect/disconnect (configChanges fix)
- LightCraft SAF image import (missing `.with_fs()` file hooks)
- Right/middle-click also triggering left-click (winit Android button mapping)

### Added
- Android system clipboard via JNI (vendored arboard + egui-winit)
- Ctrl+C/X/V shortcut handling in the shell
- Immersive fullscreen + Ctrl/Alt modifier-key fix (carried from v12)

### Supported apps
photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft,
designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft
