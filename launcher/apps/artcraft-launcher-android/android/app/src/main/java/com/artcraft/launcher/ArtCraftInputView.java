package com.artcraft.launcher;

import android.content.Context;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * A hidden text field that gives the Rust UI real soft-keyboard input.
 *
 * <p>winit's Android backend delivers key events but never the IME's committed text, so egui's
 * text fields stay empty however much is typed. The IME needs a focused view that implements
 * {@link #onCreateInputConnection}; this one is 1×1 px and invisible, and every edit the IME makes
 * is queued for {@link ArtCraftLauncherActivity#pollIme()} (the Rust side drains it every frame).
 */
final class ArtCraftInputView extends View {
    /** Text edits waiting for the Rust side to poll. */
    private final ConcurrentLinkedQueue<String> queue;
    /** What the IME thinks the field holds; also the source of backspace/enter handling. */
    private final ObservableEditable editable = new ObservableEditable();
    /** Set while the field is edited from our side, so the watcher doesn't report it back. */
    private boolean suppressingWatcher;

    ArtCraftInputView(Context context, ConcurrentLinkedQueue<String> queue) {
        super(context);
        this.queue = queue;
        setFocusable(false);
        setFocusableInTouchMode(false);
        setVisibility(GONE);
        setWillNotDraw(true);
    }

    void clearEditable() {
        suppressingWatcher = true;
        try {
            editable.clear();
            Selection.setSelection(editable, 0);
        } finally {
            suppressingWatcher = false;
        }
    }

    /** Drop the last character the IME left in the field (used for KEYCODE_DEL). */
    private void deleteLast() {
        if (editable.length() == 0) {
            return;
        }
        suppressingWatcher = true;
        try {
            int len = editable.length();
            editable.delete(len - 1, len);
            Selection.setSelection(editable, editable.length());
        } finally {
            suppressingWatcher = false;
        }
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return isFocused() && getVisibility() == VISIBLE;
    }

    @Override
    public boolean onTouchEvent(android.view.MotionEvent event) {
        return false;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT;
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_FULLSCREEN;
        return new BaseInputConnection(this, true) {
            @Override
            public Editable getEditable() {
                return editable;
            }

            @Override
            public boolean sendKeyEvent(KeyEvent event) {
                if (event == null || !isFromSoftKeyboard(event)) {
                    return super.sendKeyEvent(event);
                }
                if (event.getAction() == KeyEvent.ACTION_DOWN) {
                    if (event.getKeyCode() == KeyEvent.KEYCODE_DEL) {
                        deleteLast();
                        push("\b");
                        return true;
                    }
                    if (event.getKeyCode() == KeyEvent.KEYCODE_ENTER) {
                        push("\n");
                        return true;
                    }
                } else if (event.getAction() == KeyEvent.ACTION_UP
                        && (event.getKeyCode() == KeyEvent.KEYCODE_DEL || event.getKeyCode() == KeyEvent.KEYCODE_ENTER)) {
                    return true;
                }
                return super.sendKeyEvent(event);
            }

            @Override
            public boolean performEditorAction(int actionCode) {
                if (actionCode == EditorInfo.IME_ACTION_DONE
                        || actionCode == EditorInfo.IME_ACTION_GO
                        || actionCode == EditorInfo.IME_ACTION_SEND
                        || actionCode == EditorInfo.IME_ACTION_SEARCH) {
                    push("\n");
                    return true;
                }
                return super.performEditorAction(actionCode);
            }
        };
    }

    /** Whether a key event came from the soft keyboard (not a hardware keyboard). */
    private static boolean isFromSoftKeyboard(KeyEvent event) {
        if ((event.getFlags() & KeyEvent.FLAG_SOFT_KEYBOARD) != 0) {
            return true;
        }
        if (event.getDeviceId() <= 0) {
            return true;
        }
        InputDevice device = event.getDevice();
        if (device == null || device.isVirtual()) {
            return true;
        }
        return (event.getSource() & InputDevice.SOURCE_KEYBOARD) != InputDevice.SOURCE_KEYBOARD;
    }

    private void push(String text) {
        queue.add(text);
    }

    /**
     * Reports every change the IME makes to the field: committed text becomes an egui text event,
     * deletions a backspace each. Suppressed while we edit the field ourselves.
     */
    private final class ObservableEditable extends SpannableStringBuilder {
        @Override
        public SpannableStringBuilder replace(int start, int end, CharSequence tb, int tbstart, int tbend) {
            if (!suppressingWatcher) {
                int deleteCount = end - start;
                for (int i = 0; i < deleteCount; i++) {
                    push("\b");
                }
                if (tb != null && tbend > tbstart) {
                    String inserted = tb.subSequence(tbstart, tbend).toString();
                    if (!inserted.isEmpty()) {
                        push(inserted);
                    }
                }
            }
            SpannableStringBuilder result = super.replace(start, end, tb, tbstart, tbend);
            // Keep the field small: an IME that keeps composing forever must not grow it without bound.
            if (!suppressingWatcher && length() > 2000) {
                suppressingWatcher = true;
                try {
                    super.delete(0, length() - 500);
                    Selection.setSelection(this, length());
                } finally {
                    suppressingWatcher = false;
                }
            }
            return result;
        }
    }

    /** The queue is drained by {@link ArtCraftLauncherActivity#pollIme()}. */
    void showKeyboard() {
        post(() -> {
            try {
                setVisibility(VISIBLE);
                setFocusable(true);
                setFocusableInTouchMode(true);
                requestFocus();
                InputMethodManager imm = (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.restartInput(this);
                    imm.showSoftInput(this, 0);
                }
            } catch (Throwable ignored) {
                // The window may be gone; the next frame asks again.
            }
        });
    }

    /** Put the keyboard away and empty the field, so the next session starts clean. */
    void hideKeyboard() {
        post(() -> {
            try {
                InputMethodManager imm = (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
                Window window = (Window) null;
                if (getContext() instanceof android.app.Activity activity) {
                    window = activity.getWindow();
                }
                if (imm != null && window != null) {
                    imm.hideSoftInputFromWindow(window.getDecorView().getWindowToken(), 0);
                }
                clearEditable();
                clearFocus();
                setFocusable(false);
                setFocusableInTouchMode(false);
                setVisibility(GONE);
            } catch (Throwable ignored) {
                // Nothing to do.
            }
        });
    }

    /** Put the 1×1 view into the window's content view, once. */
    static ArtCraftInputView install(android.app.Activity activity, ConcurrentLinkedQueue<String> queue) {
        try {
            Window window = activity.getWindow();
            if (window == null) {
                return null;
            }
            ViewGroup content = window.getDecorView().findViewById(android.R.id.content);
            if (content == null) {
                return null;
            }
            ArtCraftInputView view = new ArtCraftInputView(activity, queue);
            content.addView(view, new ViewGroup.LayoutParams(1, 1));
            return view;
        } catch (Throwable e) {
            android.util.Log.w("ArtCraftLauncher", "couldn't install the input view", e);
            return null;
        }
    }
}