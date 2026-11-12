# Per-app Android adapters

The builder auto-generates a thin eframe/NativeActivity wrapper when an app has the conventional
`crates/engine`, `crates/ui-egui`, public `*App` type, `Session::new()`, and `Services::default()` API.
This only supplies a native launch shell; it does not automatically implement Android document pickers,
shared storage, clipboard, audio devices, or touch-specific UI behavior.

If an app has a different constructor or needs platform-specific services, add a complete adapter at:

    adapters/<app>/Cargo.toml
    adapters/<app>/src/lib.rs
    adapters/<app>/res/...

The builder copies that adapter into `source/<app>/apps/<app>-android/` on the first preparation. Include
`[package.metadata.android]` and the NativeActivity metadata in the adapter manifest. Use relative paths
from the Android crate to the root app workspace, normally `../../crates/...`. Keep custom adapters here,
not only in `source/`, so deleting/re-fetching a source checkout does not delete your work.

To regenerate an auto-generated wrapper, first preserve any custom edits, then remove only the marked
`source/<app>/apps/<app>-android/` directory and run the build again. The builder refuses to overwrite
unmarked directories.

## Android file open/save (SAF bridge)

`adapters/photocraft` and `adapters/vectorcraft` are complete adapters that add Android Storage Access
Framework support so the apps' File ▸ Open/Save buttons actually work on-device. Each pairs a Rust
platform-service bridge (`src/lib.rs`) with a Java `NativeActivity` subclass under
`adapters/<app>/java/...` that runs the SAF pickers, stages picked documents in app-private storage,
and copies saved bytes back through `ContentResolver`. Upstream app crates are never edited.

Building these adapters needs `cargo-apk2` (Java Activity support); plain `cargo-apk` ignores
`java_sources`. The builder selects `cargo-apk2` automatically for any adapter whose manifest declares
`java_sources`, and after packaging it verifies the launcher activity and the compiled Java are in the
APK (`verify_android_bridge` in `lib/build-common.sh`).

Manifest notes for a Java-bearing adapter:

- set `java_sources = "java"` under `[package.metadata.android]`;
- name the activity (`name = ".MyActivity"`) and give it `meta_data` `android.app.lib_name`;
- declare the launcher filter **inline**, because `cargo-apk2` does not add one itself and its serde
  `flatten` rejects an `[[...activity.intent_filter]]` table:

      intent_filter = [{ actions = ["android.intent.action.MAIN"], categories = ["android.intent.category.LAUNCHER"] }]

  Without this the APK has no launcher icon and cannot be started from the home screen, even though
  the build succeeds.

## Modifier keys (Ctrl/Alt) and Android winit

On Android, `winit` 0.30 delivers `KeyboardInput` for alphanumeric keys but does not emit
`ModifiersChanged` (it discards the `KeyEvent` meta state). The physical modifier keys
(`ControlLeft/Right`, `AltLeft/Right`, `ShiftLeft/Right`) do reach egui as `Event::Key` events, so
the adapters mirror `egui-winit` by tracking those keys in a per-app `modifiers` field and patching
`raw.events` (key, pointer button, mouse wheel modifiers) and pushing `Event::ModifiersChanged` when
they change. Plain keys work as-is; shortcuts that rely on `Ctrl`/`Alt` (e.g., `Ctrl+K`) are fixed.
Text emitted alongside a modifier chord is suppressed.

## Fullscreen

Both adapters add Java lifecycle hooks in their activity subclasses: in `onCreate` and
`onWindowFocusChanged`, they enter sticky immersive mode (hide status and navigation bars). The
theme's `Theme.DeviceDefault.NoActionBar.Fullscreen` alone hides the status bar on many Android
versions; the navigation bar often remains visible until immersive mode is explicitly set.
Re-applying on window focus ensures the bars stay hidden after system gestures or returning from the
SAF picker.

## System fonts

Adapters include the shared `mod system_fonts;`, whose source lives in
`templates/android/system_fonts.rs`. The builder copies that file next to the adapter's `src/lib.rs`
when it stages the Android crate (`prepare.sh` for PhotoCraft; the generator's custom-adapter path for
VectorCraft), so the one copy stays in step with the generated launchers. The module installs the
device's `/system/fonts` (`/product/fonts`) faces as lowest-priority egui fallbacks for scripts the
app's own fonts cannot draw, and links that folder into the user font locations the app's scanners
read. See the changelog for details. An adapter's `ui` must call
`system_fonts::install_system_fonts(ui.ctx())` and its `android_main` should call
`system_fonts::install_font_env(&android_app)` before the app starts.

## Soft keyboard (IME)

Adapters use `android-activity`'s native IME directly; there is no shared `ime_bridge` module and no
hidden Java `EditText` host. While egui reports that a text field wants the keyboard, the adapter's
`raw_input_hook` calls `app.set_ime_editor_info(...)`, resets `app.set_text_input_state(...)`, and
polls `app.text_input_state().text` each frame. A change from the previous buffer becomes
Backspace/Text/Enter events pushed into `raw.events`, the same mechanism the generated launchers use.
On focus gain an adapter calls `app.show_soft_input(false)`; on focus loss it calls
`app.hide_soft_input(false)`. Cadence is a 16 ms repaint while editing, because GameActivity's text
events do not wake winit.

## Inspecting a build on-device

Both adapters log a marker at startup (`ARTCRAFT_<APP>_ANDROID_SHELL_V2_MODIFIERS_FULLSCREEN`) and
the SAF bridge logs its own marker, so a device log confirms which shell is running:

    adb logcat | grep -E 'ARTCRAFT_|PhotoCraftSAF|VectorCraftSAF'

Installed system-font fallbacks log `Android: installed system fallback font <key> (U+XXXX)`, and a
missing script is not an error (the device simply ships no font for it):

    adb logcat | grep -E 'system fallback font|no system font found'


## Coiman IDE (`adapters/coimanide`) and the Termux bridge

Coiman IDE ([ManuelCoimbra/coiman-ide](https://github.com/ManuelCoimbra/coiman-ide)) is a desktop
eframe/egui IDE whose `app` and `ui` modules live in a binary crate, and whose Git panel, Run button
and terminal run programs that only exist inside Termux. The adapter therefore has three parts:

* `prepare.sh` turns the checkout into a library plus an Android shell without editing upstream by
  hand: it writes `crates/coiman-ide/src/lib.rs` (the same modules and core re-exports as `main.rs`,
  plus `open_file`/`open_folder`), switches eframe to wgpu and the native Activity, compiles the
  desktop pty out on Android, replaces the `trash` crate (no Android platform) with a stub that
  deletes permanently, points the whole egui stack at the builder's egui fork (the same one every
  other app uses, so the Android clipboard and IME work), and hides the desktop caption buttons.
* `src/lib.rs` is the Android shell: the SAF file bridge, the system-font handling, the modifier
  keys, the soft-keyboard bar, and the Termux environment.
* `shim/termux_shim.c` is compiled twice by `prepare.sh` into `extra-jniLibs/arm64-v8a/`:
  `libcoiman_termux_shim.so` (tools) and `libcoiman_termux_shell.so` (the interactive terminal).
  Android extracts those with the executable bit, and the shell runs them like ordinary programs.

### How a command reaches Termux

Android gives an app no way to run another app's binaries, so the shell talks to Termux through its
official `RUN_COMMAND` service:

1. `quiet_command` (every tool Coiman runs: git, cargo, python, the language servers, the Run
   button) and `which` (its "is this installed?" probe) are patched in `coiman-core` to route the
   allowlisted tools to the shim.
2. The shim writes a request into the app's own files directory and waits.
3. `TermuxBroker` (in the Activity) sees the request and sends
   `com.termux.RUN_COMMAND` to `com.termux.app.RunCommandService` with a generated `run.sh`.
4. Termux runs the script; the shim relays stdin, stdout, stderr and the exit code, so the caller
   sees an ordinary child process.

Everything the two apps exchange lives in `/sdcard/Documents/CoimanTermux`: Termux cannot read an
app's own external files directory, so shared storage is the only place both can write.

Two details are specific to Android:

* **Shared storage has no named pipes** (`mkfifo` is refused there). The stdin bridge is therefore
  a regular file: the shim appends what it receives to `<scratch>/in`, and the generated script
  feeds that file into a FIFO it creates inside Termux's own temporary directory.
* **Shared storage caches file attributes**, so the script reads the heartbeat's *contents* (a
  timestamp) rather than the file's modification time.

### What the user needs on the device

* Termux installed, opened once, with `allow-external-apps` enabled (Termux's own setting).
* The `RUN_COMMAND` permission granted (the app asks for it on first start).
* `pkg install util-linux` in Termux for the `script` command, which gives the interactive terminal
  a real pty. Without it the terminal still works, but without job control.
* Storage access for Termux (`termux-setup-storage`), so it can reach the project folder.

### The soft keyboard on a tablet

Android 15 and later keep the window edge to edge: the keyboard is drawn over the app instead of
resizing it. Coiman keeps its terminal and its other panels at the bottom of the window, so the
shell does what an app that resizes for the keyboard does, for this app only:

* the Activity caches the keyboard's height on the UI thread (`imeInsetBottom`) and wakes the
  shell when it changes, so the app repaints without waiting for the next input event;
* the shell shortens egui's screen by that height, so the whole layout — the editor above it and
  the terminal panel at the bottom — moves above the keyboard;
* the editor, the terminal and the search popups publish an IME output while they have focus, so
  the keyboard can follow the text. A tap in the editor or the terminal brings the keyboard back
  after the user dismissed it;
* the bottom bar (Coiman's status bar) carries a keyboard **icon** at its right end, followed by
  the mode the app is in. Tapping it opens a menu with the three modes — **On** (the keyboard
  stays up and comes back when it is dismissed), **Off** (the app never opens the keyboard by
  itself) and **Auto** (the keyboard follows the text: it opens for the editor, the terminal and
  the real text fields, and goes away when none of them is focused) — and with "Show now" and
  "Hide now". The mode is kept in Coiman's own settings folder, so it survives a restart. The
  button replaces the old back-button toggle: the back button belongs to the app again (a popup,
  which is a real text field, still closes on it).

### The text bar (Cut, Copy, Paste, Delete)

Coiman's editor and its terminal draw their own text, so neither of them is a `TextEdit` and the
builder's text bar (which looks for a `TextEdit`'s selection) had nothing to show. They now
publish their selection: the editor and the terminal write it into egui's own `TextEditState` (so
the bar's test finds it) and the rectangle it is at into `coiman_core::selection` (so the bar sits
next to the text instead of at the top of the screen).

### The themes

The themes the user has in VS Code are bundled with the app (`adapters/coimanide/themes`, 23 of
them, without Breeze Dark and Breeze Light). Coiman can read VS Code color themes already; it
finds them in the VS Code extensions folder or in `<config>/themes`, so `prepare.sh` copies the
bundled ones into the staged app's assets, the Activity unpacks them, and the shell copies them
into Coiman's config folder, where its theme picker lists them.

### Asking for Termux

When Termux is not installed, the Activity asks for it with a dialog like the builder's update
window: **Download** (opens Termux's page on F-Droid), **Ignore once** and **Always ignore** (the
last one is remembered in the app's settings).

### Inspecting the bridge on-device

The broker logs to logcat, and the bridge keeps a log of its sessions in the shared folder:

    adb logcat | grep CoimanTermux
    adb shell cat /sdcard/Documents/CoimanTermux/coiman-termux.log
