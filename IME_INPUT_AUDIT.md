# IME Input Handling Audit - Android ArtCraft Clones

**Date:** 2026-11-01  
**Source:** `/run/media/mostafa/Data/artcraft-clones/`

## Summary

The Android clones in `/run/media/mostafa/Data/artcraft-clones/` do **not** use a Java `EditText`/`InputConnection` bridge. All apps rely entirely on `android-activity` + `eframe` + `egui` IME support. The NativeActivity surface receives keyboard/IME events from the Android system through `android-activity`'s `RawInput` → `egui::Context`, and the Rust UI processes commits/composition via the app's own egui IME integration.

No `imeShow/imeHide`, no `InputConnection`, no custom `EditText` host is present in the reference clones.

## Apps Found

- `cadcraft-android`
- `effectcraft-android`
- `kaedo-wordcraft-mobile`
- `nandaje-wordcraft-android`
- `photocraft-android`
- `vectorcraft-android`

## Java Activity Pattern

All activities are `NativeActivity` subclasses with JNI callbacks for SAF/permissions, no IME code:

**CADCraft `/run/media/mostafa/Data/artcraft-clones/cadcraft-android/packaging/android/app/src/main/java/ai/storyteller/cadcraft/MainActivity.kt`**

```kotlin
class MainActivity : NativeActivity() {
    companion object {
        init { System.loadLibrary("cadcraft_android") }
        @JvmStatic external fun nativeOnStoragePermissionResult(...)
        @JvmStatic external fun nativeDeliverFile(...)
        @JvmStatic external fun nativeDeliverSafOpenFile(...)
        @JvmStatic external fun nativeDeliverSafSavePath(...)
        @JvmStatic external fun nativeDeliverSafFolderPath(...)
    }
    // ... requestStoragePermissions, openSafFilePicker, etc
}
```

Same pattern for WordCraft, PhotoCraft. No `EditText`, `InputConnection`, `imeShow/imeHide`.

## Rust Android Entry Pattern

**CADCraft `/run/media/mostafa/Data/artcraft-clones/cadcraft-android/apps/cadcraft-android/src/android_entry.rs`**

```rust
pub fn android_main(app: AndroidApp) {
    app.enable_motion_axis(...);
    permissions::init_dirs(...);
    install_jni_bridge(&app);
    file_dialog::request_storage_permissions(false);
    ...
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title("CADCraft")
            .with_drag_and_drop(true),
        android_app: Some(app),
        ...
    };
    eframe::run_native(...)
}
```

JNI callbacks for permissions, file delivery, SAF. No IME bridge.

WordCraft entry is identical: `android_activity` + `eframe::run_native` with `android_app: Some(app)`.

## IME Handling in Rust UI

### WordCraft - egui IME composition

`/run/media/mostafa/Data/artcraft-clones/nandaje-wordcraft-android/crates/ui-egui/src/canvas.rs`

```rust
pub ime_preedit: String,
pub(crate) ime_preedit_range: Option<std::ops::Range<usize>>,
pub(crate) ime_faces: Vec<std::sync::Arc<wordcraft_fonts::FontFace>>,
pub(crate) ime_reset: bool,
pub(crate) ime_interrupt_now: bool,
pub(crate) ime_interrupted_preedit: bool,
pub(crate) ime_tracker: egui::ImeContextTracker,
pub(crate) ime_viewport: Option<egui::ViewportId>,
```

`/run/media/mostafa/Data/artcraft-clones/nandaje-wordcraft-android/crates/ui-egui/src/keys/ime.rs`

```rust
app.canvas.ime_preedit.clear();
app.canvas.ime_preedit_range = None;
...
app.canvas.ime_preedit = text;
app.canvas.ime_preedit_range = active_range_chars;
```

Pre-edit drawn as overlay on canvas. Commit/reset in `crates/ui-egui/src/app/commands.rs`:

```rust
let composing_before = !self.canvas.ime_preedit.is_empty();
self.canvas.ime_preedit.clear();
self.canvas.ime_preedit_range = None;
```

Window isolation: `/run/media/mostafa/Data/artcraft-clones/nandaje-wordcraft-android/crates/ui-egui/src/windows.rs` ensures only focused window composes.

### VectorCraft - Outgoing IME producer

`/run/media/mostafa/Data/artcraft-clones/vectorcraft-android/crates/ui-egui/src/ime.rs`

Uses `egui::ime_context` helper:

```rust
use egui::ime_context::{CommittedText, ImeContextUpdate, ImeMutationStamp, ImeObservation, ImeTarget};
...
pub(crate) const SURROUNDING_LIMIT: usize = MAX_IME_SURROUNDING_TEXT_BYTES;
```

Target identity from engine:
```rust
pub(crate) fn target(session: &mut Session, view: ViewInfo) -> Option<ImeTarget> {
    session.tool_ime_target(view).map(|(document, node, session)| 
        ImeTarget::new(Id::unique(("vectorcraft-type-ime", document, node)), session))
}
```

Stamp/commit tracking:
```rust
pub(crate) fn stamp(session: &mut Session, view: ViewInfo) -> Option<ImeMutationStamp>
pub(crate) fn note_ime_apply(...)
pub(crate) fn note_application_edit(...)
```

End-of-pass writes egui `IMEOutput`:
```rust
pub(crate) fn write_output(ctx: &Context, update: Option<ImeContextUpdate>, rect: egui::Rect, cursor_rect: egui::Rect, interrupt: bool) {
    ctx.output_mut(|o| {
        o.ime = Some(IMEOutput {
            owner: update.owner,
            context_generation: update.context_generation,
            purpose: update.purpose,
            rect,
            cursor_rect,
            should_interrupt_composition: interrupt,
            reset_request: None,
            change_cause: update.change_cause,
            surrounding_text: update.surrounding_text,
        });
    });
}
```

App hooks:
`/run/media/mostafa/Data/artcraft-clones/vectorcraft-android/crates/ui-egui/src/app/ime.rs`
```rust
pub fn install_ime_reset_hooks(ctx: &egui::Context) { ime::install_hooks(ctx); }
pub fn finish_ime_logic(&mut self, ctx: &egui::Context) { ime::finish_logic(self, ctx); }
```

Type tool pre-edit: `/run/media/mostafa/Data/artcraft-clones/vectorcraft-android/crates/tools/src/text/tool.rs`
```rust
fn ime_preedit(&mut self, cx: &ToolContext, text: &str, active_chars: Option<Range<usize>>) -> Vec<Action>
```

### EffectCraft

Script UI uses `WidgetKind::EditText` as logical widget kind, not Android view:

`/run/media/mostafa/Data/artcraft-clones/effectcraft-android/crates/ui-egui/src/panels/scriptui_view.rs`
```rust
WidgetKind::EditText => { ... }
```

IME pre-edit preview stored in egui temp data:
`/run/media/mostafa/Data/artcraft-clones/effectcraft-android/crates/ui-egui/src/panels/viewer_text.rs`
```rust
fn ime_preedit_id(viewport: egui::ViewportId) -> egui::Id
ui.data_mut(|d| d.remove::<Preedit>(ime_preedit_id(viewport)));
```

## How Input Flows

1. **Soft keyboard show/hide is implicit**: When egui has keyboard focus and `pass_eligible` is true, the app writes an `IMEOutput` with `rect`/`cursor_rect`. `android-activity` maps that to Android's IME window. No explicit `imeShow/imeHide` calls exist in reference apps.

2. **Commit text**: egui receives `egui::Event::Text` / `egui::Event::Ime` from `android-activity`. For WordCraft/VectorCraft/PhotoCraft/CADCraft, Rust UI processes commit via `ime.rs` / `keys` modules and forwards to engine `Tool::ime_preedit` / commit path.

3. **Composition**: Pre-edit text kept in `canvas.ime_preedit` / `ime_preedit` tool state and painted as placeholder overlay while IME active. `ime_preedit_range` holds active caret/selection inside composition.

4. **Interaction with egui**: Apps never create Android `EditText`. Text editing performed inside egui canvas. IME context published via `ctx.output_mut(|o| o.ime = Some(IMEOutput{...}))`. `android-activity` translates that to platform IME, incoming composition/commit events fed back as `egui::RawInput`.

## Contrast with Builder's IME Bridge

The builder (`/home/mostafa/artcraft-android-builder/templates/android/SafActivity.java` and `templates/android/ime_bridge.rs`) uses a **different** approach:

- Creates invisible `EditText` with custom `InputConnectionWrapper`
- Queues events as `c` commit, `p` pre-edit, `f` finish, `d` delete, `a` action
- Rust `ime_bridge.rs` polls via JNI `imePoll()` and translates to `egui::Event::Ime`
- `imeShow()` / `imeHide()` called via JNI to show/hide soft keyboard

Reference clones **do not** use this. They rely on `android-activity` + `eframe` native IME support.

## Implications for Current Issues

The builder's custom IME bridge is likely broken or incomplete:
- IME input broken in many apps
- The reference apps work without an `EditText` bridge, using only egui's `IMEOutput`
- Possible conflicts between egui's native IME handling and the custom bridge
- The bridge may interfere with egui's own `android-activity` IME delivery

Recommendation: audit whether the builder's `ime_bridge` is necessary at all, or if egui's native IME support via `android-activity` should be used instead, matching the reference implementations.
