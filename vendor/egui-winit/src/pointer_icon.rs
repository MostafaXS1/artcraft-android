//! Android system pointer icon (adaptive cursor) feed.
//!
//! On the desktop, winit's `Window::set_cursor` is what turns the [`egui::CursorIcon`] the
//! interface resolved for the current frame into a real cursor: an I-beam over text, a
//! pointing hand over buttons, resize arrows over splitters, and so on. On Android that same
//! call is a no-op — the platform has no `set_cursor` to give a mouse, because a cursor there
//! is a system-drawn *pointer icon* owned by the input dispatcher, chosen per hovered view.
//!
//! So the icon egui computes never reaches the screen: a mouse or trackpad over the canvas
//! keeps the default arrow no matter what is underneath. This module closes that gap the same
//! way `system_theme` closes the light/dark one — each frame, the resolved
//! [`egui::CursorIcon`] is translated to a `android.view.PointerIcon` constant and pushed to
//! the Activity's surface view over JNI. Android then draws the native I-beam, pointing hand,
//! resize arrows, and so on, exactly as it does for a native text widget.
//!
//! The icon is only pushed when it actually changes (the caller's `current_cursor_icon`
//! dedupe already guarantees that), so a still pointer costs nothing per frame.
//!
//! Nothing else is added: the surface view keeps its own hover handling, and the pointer icon
//! applies only while the pointer is over the window. A device with no mouse never sees any
//! of this — no hover events means this module is never reached.
#![cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]

/// `android.view.PointerIcon` system icon types, as they exist since API 24.
///
/// Kept as plain `int`s rather than a binding to the Java constants so the mapping stays
/// reviewable in one place and the JNI call only has to name the type.
mod ty {
    pub const ARROW: i32 = 1000;
    pub const CONTEXT_MENU: i32 = 1001;
    pub const HAND: i32 = 1002;
    pub const HELP: i32 = 1003;
    pub const CELL: i32 = 1006;
    pub const CROSSHAIR: i32 = 1007;
    pub const TEXT: i32 = 1008;
    pub const ALIAS: i32 = 1010;
    pub const COPY: i32 = 1011;
    pub const NO_DROP: i32 = 1012;
    pub const NOT_ALLOWED: i32 = 1013;
    pub const GRAB: i32 = 1016;
    pub const GRABBING: i32 = 1017;
    pub const ZOOM_IN: i32 = 1018;
    pub const ZOOM_OUT: i32 = 1019;
    /// `TYPE_ALL_SCROLL`: the four-way move cross.
    pub const ALL_SCROLL: i32 = 1015;
    /// `TYPE_HORIZONTAL_DOUBLE_ARROW` (`ew-resize`).
    pub const HORIZONTAL_DOUBLE_ARROW: i32 = 1014;
    /// `TYPE_VERTICAL_DOUBLE_ARROW` (`ns-resize`).
    pub const VERTICAL_DOUBLE_ARROW: i32 = 1021;
    /// `TYPE_TOP_LEFT_DIAGONAL_DOUBLE_ARROW` (`nwse-resize`).
    pub const TOP_LEFT_DIAGONAL_DOUBLE_ARROW: i32 = 1022;
    /// `TYPE_TOP_RIGHT_DIAGONAL_DOUBLE_ARROW` (`nesw-resize`).
    pub const TOP_RIGHT_DIAGONAL_DOUBLE_ARROW: i32 = 1023;
}

/// The `android.view.PointerIcon` type for an [`egui::CursorIcon`].
///
/// `None` means "leave Android's pointer icon alone" — for the variants that have no sensible
/// native counterpart (`Progress`, the edge-specific resize arrows reusing the axis arrow, and
/// `None`/hidden cursors). Returning `None` rather than a guess keeps a plain arrow from
/// flickering in where the interface asked for something the platform cannot draw.
pub fn pointer_icon_type(cursor: egui::CursorIcon) -> Option<i32> {
    use egui::CursorIcon as C;
    Some(match cursor {
        C::Default => ty::ARROW,
        // Text entry: the native I-beam, the whole point of this bridge.
        C::Text | C::VerticalText => ty::TEXT,
        // Anything clickable reads as a hand, the same as `cursor: pointer` on the web.
        C::PointingHand => ty::HAND,
        C::Crosshair => ty::CROSSHAIR,
        C::ContextMenu => ty::CONTEXT_MENU,
        C::Help => ty::HELP,
        C::Cell => ty::CELL,
        C::Alias => ty::ALIAS,
        C::Copy => ty::COPY,
        C::NoDrop => ty::NO_DROP,
        C::NotAllowed => ty::NOT_ALLOWED,
        C::Grab => ty::GRAB,
        C::Grabbing => ty::GRABBING,
        C::ZoomIn => ty::ZOOM_IN,
        C::ZoomOut => ty::ZOOM_OUT,
        C::AllScroll => ty::ALL_SCROLL,
        C::Move => ty::ALL_SCROLL,
        C::ResizeHorizontal | C::ResizeColumn => ty::HORIZONTAL_DOUBLE_ARROW,
        C::ResizeVertical | C::ResizeRow => ty::VERTICAL_DOUBLE_ARROW,
        C::ResizeNwSe => ty::TOP_LEFT_DIAGONAL_DOUBLE_ARROW,
        C::ResizeNeSw => ty::TOP_RIGHT_DIAGONAL_DOUBLE_ARROW,
        // Edge-specific resizes collapse onto the matching axis arrow: Android draws no
        // north-only or south-only arrow, and an axis arrow is closer than the default.
        C::ResizeEast | C::ResizeWest => ty::HORIZONTAL_DOUBLE_ARROW,
        C::ResizeNorth | C::ResizeSouth => ty::VERTICAL_DOUBLE_ARROW,
        C::ResizeNorthEast | C::ResizeSouthWest => ty::TOP_RIGHT_DIAGONAL_DOUBLE_ARROW,
        C::ResizeNorthWest | C::ResizeSouthEast => ty::TOP_LEFT_DIAGONAL_DOUBLE_ARROW,
        // `Progress`/`Wait` would mean a spinner; `None` means no cursor at all. Android has
        // no per-view animated spinner icon, so leave the current one in place.
        C::Progress | C::Wait | C::None => return None,
    })
}

/// `View.getClass().getName()` as a Rust string, or `""` when the lookup fails.
#[cfg(target_os = "android")]
fn view_class_name(env: &mut jni::Env<'_>, view: &jni::objects::JObject<'_>) -> String {
    use jni::{jni_sig, jni_str};

    env.call_method(view, jni_str!("getClass"), jni_sig!("()Ljava/lang/Class;"), &[])
        .and_then(|v| v.l())
        .and_then(|c| {
            env.call_method(&c, jni_str!("getName"), jni_sig!("()Ljava/lang/String;"), &[])
                .and_then(|v| v.l())
        })
        .ok()
        .and_then(|n| {
            env.cast_local::<jni::objects::JString>(n)
                .ok()
                .and_then(|s| env.get_string(&s).ok().map(String::from))
        })
        .unwrap_or_default()
}

/// Push `cursor` to the Activity's GameActivity surface view as a Android pointer icon.
///
/// `Activity.getWindow()` needs the Activity, and `ndk-context` only carries the Application
/// context, so the shell publishes the real handle once at startup (`set_activity`).
///
/// Walks `android.R.id.content` down to the `InputEnabledSurfaceView` and calls
/// `View.setPointerIcon(PointerIcon.getSystemIcon(ctx, type))`. The icon has to land on the surface
/// view itself: `ViewGroup.onResolvePointerIcon` asks the hovered *child* first and only falls back
/// to a parent when the child has no icon of its own, so setting it on an ancestor is ignored.
#[cfg(target_os = "android")]
#[expect(unsafe_code)]
pub fn apply(cursor: egui::CursorIcon) {
    let Some(icon_type) = pointer_icon_type(cursor) else {
        return;
    };

    use jni::{jni_sig, jni_str, objects::JObject, JavaVM};

    let ctx = ndk_context::android_context();
    // SAFETY: AndroidApp owns the live VM pointer; `ndk-context` initialized it before `android_main`.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let Some(activity_ptr) = crate::android_activity::activity_raw() else {
        log::debug!("egui-winit Android pointer icon: Activity not published yet; skipping");
        return;
    };
    let res = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let activity_raw = activity_ptr;
        // SAFETY: the shell stored an unowned JNI global reference owned by `android-activity`;
        // borrow it, never delete it.
        let activity = unsafe { env.as_cast_raw::<JObject<'_>>(&activity_raw)? };

        // `PointerIcon.getSystemIcon(Context, int)` — one shared icon per type, so this is cheap
        // and never needs caching on our side.
        let pointer_icon_class = env.find_class(jni_str!("android/view/PointerIcon"))?;
        let icon = env
            .call_static_method(
                &pointer_icon_class,
                jni_str!("getSystemIcon"),
                jni_sig!("(Landroid/content/Context;I)Landroid/view/PointerIcon;"),
                &[
                    jni::objects::JValue::Object(&*activity),
                    jni::objects::JValue::Int(icon_type),
                ],
            )?
            .l()?;
        if icon.is_null() {
            return Ok(());
        }

        // `Window.getDecorView().findViewById(android.R.id.content)` is the most reliable way to
        // the content root: it needs no GameActivity internals, and works identically for the
        // generated shell and the hand-written adapters.
        let window = env
            .call_method(
                &*activity,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?
            .l()?;
        if window.is_null() {
            return Ok(());
        }
        let decor = env
            .call_method(
                &window,
                jni_str!("getDecorView"),
                jni_sig!("()Landroid/view/View;"),
                &[],
            )?
            .l()?;
        if decor.is_null() {
            return Ok(());
        }

        // `android.R.id.content` is a framework resource id; read it from the class rather than
        // hard-coding an int (it has never been stable across releases).
        let r_id = env.find_class(jni_str!("android/R$id"))?;
        let content_id = env
            .get_static_field(&r_id, jni_str!("content"), jni_sig!("I"))?
            .i()?;

        let content = env
            .call_method(
                &decor,
                jni_str!("findViewById"),
                jni_sig!("(I)Landroid/view/View;"),
                &[jni::objects::JValue::Int(content_id)],
            )?
            .l()?;
        if content.is_null() {
            return Ok(());
        }

        // Walk down to the view that actually receives hover: GameActivity wraps its
        // `InputEnabledSurfaceView` one level below the app-compat content view
        // (`ContentFrameLayout` → `FrameLayout` → `InputEnabledSurfaceView`), and any extra
        // views the app adds (the invisible IME host) sit as siblings, not on that path.
        // `ViewGroup.onResolvePointerIcon` asks the hovered *child* first and only falls back to
        // a parent when the child has no icon of its own, so the icon has to land on the surface
        // view itself — setting it on an ancestor is silently ignored. Descend until a view has
        // no children; that leaf is the surface view.
        let mut target = content;
        for _ in 0..8 {
            let child_count = match env
                .call_method(&target, jni_str!("getChildCount"), jni_sig!("()I"), &[])
                .and_then(|v| v.i())
            {
                Ok(n) => n,
                Err(err) => {
                    // Not a ViewGroup (or the call threw): clear the pending exception and stop.
                    let _ = env.exception_clear();
                    log::debug!("egui-winit Android pointer icon: getChildCount failed: {err}");
                    break;
                }
            };
            if child_count <= 0 {
                break;
            }
            // Prefer the child that is (or contains) a surface view; fall back to the first child.
            let mut next = None;
            for index in 0..child_count {
                let child = env
                    .call_method(
                        &target,
                        jni_str!("getChildAt"),
                        jni_sig!("(I)Landroid/view/View;"),
                        &[jni::objects::JValue::Int(index)],
                    )?
                    .l()?;
                if child.is_null() {
                    continue;
                }
                let name = view_class_name(env, &child);
                if name.contains("SurfaceView") {
                    next = Some(child);
                    break;
                }
                if next.is_none() {
                    next = Some(child);
                }
            }
            match next {
                Some(child) => target = child,
                None => break,
            }
        }

        env.call_method(
            &target,
            jni_str!("setPointerIcon"),
            jni_sig!("(Landroid/view/PointerIcon;)V"),
            &[jni::objects::JValue::Object(&icon)],
        )?;
        Ok(())
    });
    if let Err(err) = res {
        log::error!("egui-winit Android pointer icon apply failed: {err}");
    }
}

#[cfg(not(target_os = "android"))]
pub fn apply(_cursor: egui::CursorIcon) {}

#[cfg(test)]
mod tests {
    use egui::CursorIcon as C;

    use super::{pointer_icon_type, ty};

    #[test]
    fn text_is_the_native_i_beam() {
        // The headline case from the report: hovering document text.
        assert_eq!(pointer_icon_type(C::Text), Some(ty::TEXT));
        assert_eq!(pointer_icon_type(C::VerticalText), Some(ty::TEXT));
    }

    #[test]
    fn clickable_ui_is_a_hand() {
        assert_eq!(pointer_icon_type(C::PointingHand), Some(ty::HAND));
    }

    #[test]
    fn splitters_get_axis_arrows() {
        assert_eq!(
            pointer_icon_type(C::ResizeHorizontal),
            Some(ty::HORIZONTAL_DOUBLE_ARROW)
        );
        assert_eq!(
            pointer_icon_type(C::ResizeVertical),
            Some(ty::VERTICAL_DOUBLE_ARROW)
        );
        // Column/row resizes are the same gesture on the other axis.
        assert_eq!(
            pointer_icon_type(C::ResizeColumn),
            Some(ty::HORIZONTAL_DOUBLE_ARROW)
        );
        assert_eq!(pointer_icon_type(C::ResizeRow), Some(ty::VERTICAL_DOUBLE_ARROW));
    }

    #[test]
    fn diagonals_keep_their_orientation() {
        assert_eq!(
            pointer_icon_type(C::ResizeNwSe),
            Some(ty::TOP_LEFT_DIAGONAL_DOUBLE_ARROW)
        );
        assert_eq!(
            pointer_icon_type(C::ResizeNeSw),
            Some(ty::TOP_RIGHT_DIAGONAL_DOUBLE_ARROW)
        );
    }

    #[test]
    fn no_native_counterpart_is_left_alone() {
        // These would otherwise flash an arrow where the interface asked for a spinner (or
        // nothing at all), so they must not produce an icon type.
        assert_eq!(pointer_icon_type(C::Progress), None);
        assert_eq!(pointer_icon_type(C::Wait), None);
        assert_eq!(pointer_icon_type(C::None), None);
    }

    #[test]
    fn default_is_the_plain_arrow() {
        assert_eq!(pointer_icon_type(C::Default), Some(ty::ARROW));
    }
}
