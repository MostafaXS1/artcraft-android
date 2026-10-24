package com.artcraft.photocraft;

import android.Manifest;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.SystemClock;
import android.provider.OpenableColumns;
import android.provider.Settings;
import android.text.InputType;
import android.util.DisplayMetrics;
import android.util.Log;
import android.view.DragEvent;
import android.view.Display;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;

import com.google.androidgamesdk.GameActivity;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/** GameActivity subclass that supplies Android SAF open/save results to the Rust UI shell. */
public final class PhotoCraftActivity extends GameActivity {
    private static final String TAG = "PhotoCraftSAF";
    private static final int REQ_OPEN = 0x5043;
    private static final int REQ_SAVE = 0x5044;
    private static final int REQ_PERMISSIONS = 0x5045;
    private static final String PREFS = "photocraft_saf_uri_map";
    private final ConcurrentLinkedQueue<String> results = new ConcurrentLinkedQueue<>();
    private final Map<Integer, Long> requestIds = new HashMap<>();
    private final Map<Long, String> suggestedNames = new HashMap<>();
    private final ExecutorService io = Executors.newSingleThreadExecutor();

    // Soft-keyboard host. GameActivity, like the NativeActivity it replaces, has no editable
    // view for the system IME, so the keyboard opens unreliably and delivers no text. This
    // tiny, invisible but focusable EditText has a real InputConnection, so the keyboard
    // shows/dismisses on request and every commit/composition arrives here where imePoll()
    // hands it to the Rust shell as egui IME events.
    private final ConcurrentLinkedQueue<String> imeEvents = new ConcurrentLinkedQueue<>();
    private EditText imeView;
    // True between imeShow() and imeHide(); suppresses a deferred show that a quick hide overtook.
    private volatile boolean imeWanted = false;
    // Some IMEs report one backspace press twice, through two different calls:
    // `deleteSurroundingText(1,0)` and `sendKeyEvent(KEYCODE_DEL)`. Both arrive back to back, in
    // the same IME dispatch — microseconds apart — while two deliberate taps are always further
    // apart (the finger has to lift and press again). So only a *pair of different methods* within
    // `IME_DELETE_PAIR_MS` is treated as a repeat; the second call is dropped, and the same method
    // called twice is never dropped (a quick double-tap still erases twice). Both timestamps are
    // remembered because an IME may send either order.
    private static final long IME_DELETE_PAIR_MS = 30L;
    private volatile long lastImeDeleteSurroundingAt = 0L;
    private volatile long lastImeDeleteKeyAt = 0L;
    // Key codes an IME synthesised through `sendKeyEvent` and which the host view must therefore
    // consume instead of letting them bubble up to the native input queue (see the host's
    // `onKeyDown`/`onKeyUp`). Guarded by itself: `sendKeyEvent` runs on the IME's thread and the
    // key dispatch on the main thread.
    private final java.util.Set<Integer> imeKeySynthesized = java.util.Collections.synchronizedSet(new java.util.HashSet<>());

    // Documents the system asked this app to open (an ACTION_VIEW/ACTION_SEND it is registered for):
    // the private path of each staged copy, drained one per call by the Rust shell through
    // intentPoll() and opened exactly like a file chosen in the picker.
    private final ConcurrentLinkedQueue<String> intentPaths = new ConcurrentLinkedQueue<>();

    // Desktop-style drag and drop: dragging a file out of the Files app (or a browser) onto this
    // window in split-screen lands it in the same intentPaths queue as a shared file, so the Rust
    // shell opens it through the unchanged intentPoll() path - one document per drop, pictures
    // landing on the open page wherever the app's inbox inserts them.
    private final View.OnDragListener dropListener = new View.OnDragListener() {
        @Override
        public boolean onDrag(View view, DragEvent event) {
            try {
                switch (event.getAction()) {
                    case DragEvent.ACTION_DRAG_STARTED:
                        // Accept only drags that actually carry files; returning false here keeps
                        // text/other drags free for whatever else wants them.
                        return !collectDragUris(event.getClipData()).isEmpty();
                    case DragEvent.ACTION_DRAG_ENTERED:
                    case DragEvent.ACTION_DRAG_LOCATION:
                    case DragEvent.ACTION_DRAG_EXITED:
                        // Take the enter as well: the framework keeps a drag with the view that
                        // accepted the enter and hands the drop to that same view, so declining
                        // the enter here would swallow the drop before it arrives.
                        return true;
                    case DragEvent.ACTION_DROP: {
                        List<Uri> uris = collectDragUris(event.getClipData());
                        if (uris.isEmpty()) return false;
                        // The drag grant is temporary; keep it if the source offered a persistable
                        // one, then copy on the I/O thread while the drop access still holds.
                        for (Uri uri : uris) persistDroppedPermission(uri);
                        stageDroppedFiles(uris);
                        return true;
                    }
                    default:
                        // ENDED (the drag ended) and anything else: nothing to do.
                        return false;
                }
            } catch (Throwable error) {
                Log.e(TAG, "Could not handle a dragged file", error);
                return false;
            }
        }
    };

    public String pcFilesDir() { return appStorageDir().getAbsolutePath(); }

    /**
     * True when the screen is dense enough that the launch should give the app its own UI scale
     * instead of the system's. Above 600 dpi the app's own points are drawn so small that its
     * interface is hard to read, so it is scaled up; at or below that threshold — tablets in
     * particular — the app is left at Auto, the display scale Android reports.
     *
     * The test is {@link DisplayMetrics#densityDpi}, the screen's density as Android reports it
     * (160 dpi is the baseline where one dp is one pixel), including the user's Display Size
     * setting, because that is the density the app is actually drawn at.
     */
    public boolean isHighDensityDevice() {
        try {
            return getResources().getConfiguration().smallestScreenWidthDp < 600;
        } catch (Throwable error) {
            Log.w(TAG, "Could not read screen size; leaving UI scale at Auto", error);
            return false;
        }
    }

    /**
     * The app's private storage root: the app-specific external directory
     * {@code /storage/emulated/0/Android/data/<pkg>/files} whenever the device has it, so the
     * preferences, exports and the working copies of SAF documents stay reachable over USB/MTP, and
     * the internal directory otherwise.
     */
    private File appStorageDir() {
        File external = getExternalFilesDir(null);
        return external != null ? external : getFilesDir();
    }

    /**
     * Ask once, on first launch, for broad file access so the app can read and write the device's
     * own storage — the files the user opens or exports directly, and the font folders the builder
     * links for the shell (the device's own faces and the user's {@code Fonts} folder). Android
     * 11+ grants it through the "All files access" settings screen; older versions request it at
     * runtime. The ask is remembered so a refusal never nags; the grant can be changed later in
     * Settings.
     */
    /**
     * Ask the user for broad device file access ("All files access"), so the app's own file
     * browsers can read and write files beyond the ones picked through SAF, and the font
     * folders the shell links are readable.
     *
     * The ask is repeated on every launch until the user grants it: the permission is what the
     * app needs to work with the device's storage, and a dismissal is not an answer that lasts.
     * Once it is granted, nothing is asked any more.
     */
    private void requestStoragePermission() {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                if (Environment.isExternalStorageManager()) return;
                Intent intent = new Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION);
                intent.setData(Uri.parse("package:" + getPackageName()));
                startActivity(intent);
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                if (checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED) return;
                requestPermissions(
                        new String[] {
                            Manifest.permission.READ_EXTERNAL_STORAGE,
                            Manifest.permission.WRITE_EXTERNAL_STORAGE
                        },
                        REQ_PERMISSIONS);
            }
        } catch (Exception e) {
            Log.e(TAG, "Could not request file access", e);
        }
    }

    private void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.systemBars());
                controller.setSystemBarsBehavior(
                        WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            window.getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                            | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_FULLSCREEN);
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        applyImmersiveFullscreen();
        Window window = getWindow();
        if (window != null) {
            window.getDecorView().post(this::applyImmersiveFullscreen);
        }
        installDropTarget();
        // Once, on the first launch: the settings screen that grants the app access to the device's
        // storage (the font folders the shell links included). Answered later, outside the app, so
        // the links the shell makes at startup are refreshed when the access arrives.
        requestStoragePermission();
        imeSetup();
        handleIncomingFiles(getIntent());
        // Ask GitHub whether a newer builder release exists (a few seconds in, off the main
        // thread). The shared checker is installed next to this Activity by the builder.
        com.artcraft.updater.UpdateCheck.start(this);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // singleTask launch mode: opening a registered file type while the app runs lands here
        // instead of onCreate, so stage it for the shell the same way.
        setIntent(intent);
        applyImmersiveFullscreen();
        handleIncomingFiles(intent);
    }

    /**
     * On a real finish, end the process instead of running GameActivity's teardown: the game
     * glue's destroy handshake waits for the Rust thread to acknowledge the destroy, which
     * winit's Android backend doesn't do once eframe's loop has stopped -- the main thread
     * would hang there, and every later launch in this process with it, since winit's event
     * loop cannot be created twice in one process. Staged file copies get a moment to land
     * before the process goes.
     */
    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            io.shutdown();
            try {
                if (!io.awaitTermination(20, TimeUnit.SECONDS)) {
                    Log.w(TAG, "staged copies still copying after 20 s; ending anyway");
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            Log.i(TAG, "finished; ending the process");
            android.os.Process.killProcess(android.os.Process.myPid());
        }
        super.onDestroy();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            applyImmersiveFullscreen();
        }
    }

    /**
     * Install the invisible, focusable {@link EditText} that makes Android's soft keyboard usable.
     * Called from {@link #onCreate}; failures are logged and leave the app on the old (unreliable)
     * decor-view path, so a device that rejects the extra view still runs.
     */
    private void imeSetup() {
        try {
            // The IME height must not resize or pan the fullscreen native surface; the keyboard
            // simply overlays it. STATE_UNCHANGED keeps the initial hidden state under our control.
            Window window = getWindow();
            if (window != null) {
                window.setSoftInputMode(
                        WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING
                                | WindowManager.LayoutParams.SOFT_INPUT_STATE_UNCHANGED);
            }
            imeView = new EditText(this) {
                @Override
                public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
                    InputConnection base = super.onCreateInputConnection(outAttrs);
                    if (base == null) {
                        return null;
                    }
                    return new InputConnectionWrapper(base, true) {
                        @Override
                        public boolean commitText(CharSequence text, int newCursorPosition) {
                            if (text != null && text.length() > 0) {
                                queueImeEvent("c" + text);
                            }
                            return super.commitText(text, newCursorPosition);
                        }

                        @Override
                        public boolean setComposingText(CharSequence text, int newCursorPosition) {
                            queueImeEvent("p" + (text == null ? "" : text.toString()));
                            return super.setComposingText(text, newCursorPosition);
                        }

                        @Override
                        public boolean finishComposingText() {
                            queueImeEvent("f");
                            return super.finishComposingText();
                        }

                        @Override
                        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                            if (beforeLength > 0 || afterLength > 0) {
                                if (claimDelete("d")) {
                                    queueImeEvent("d" + beforeLength + "," + afterLength);
                                }
                            }
                            return super.deleteSurroundingText(beforeLength, afterLength);
                        }

                        @Override
                        public boolean performEditorAction(int actionCode) {
                            queueImeEvent("a" + actionCode);
                            return super.performEditorAction(actionCode);
                        }

                        @Override
                        public boolean sendKeyEvent(KeyEvent event) {
                            if (event.getAction() == KeyEvent.ACTION_DOWN) {
                                int code = event.getKeyCode();
                                if (code == KeyEvent.KEYCODE_DEL) {
                                    if (claimDelete("s")) {
                                        queueImeEvent("d1,0");
                                    }
                                } else if (code == KeyEvent.KEYCODE_FORWARD_DEL) {
                                    if (claimDelete("s")) {
                                        queueImeEvent("d0,1");
                                    }
                                } else if (code == KeyEvent.KEYCODE_ENTER) {
                                    queueImeEvent("a6");
                                }
                            }
                            // An IME answers some keys by sending a synthesised key event, and the
                            // wrapper forwards it to the host view. Without this the host's
                            // `onKeyDown` would return false and the event would bubble up to the
                            // Activity, into GameActivity's native queue and out through winit as a
                            // *second* deletion (a real Backspace) on top of the one queued above —
                            // the double-delete. `onKeyDown`/`onKeyUp` use the flag to consume the
                            // pair here, so neither reaches the native path.
                            imeKeySynthesized.add(event.getKeyCode());
                            return super.sendKeyEvent(event);
                        }
                    };
                }

                @Override
                public boolean onKeyDown(int keyCode, KeyEvent event) {
                    // A key the IME synthesised (see sendKeyEvent above): its deletion was already
                    // queued, so consuming it here stops it bubbling to the Activity and reaching
                    // winit as a second Backspace. Physical key events never carry the flag, so they
                    // still fall through to the native queue below and keep working.
                    if (imeKeySynthesized.contains(keyCode)) {
                        return true;
                    }
                    // Let hardware key events fall through to the native input queue (winit), exactly
                    // as they did before this view existed; the soft keyboard talks to us through the
                    // InputConnection instead. This keeps physical keyboards working.
                    return false;
                }

                @Override
                public boolean onKeyUp(int keyCode, KeyEvent event) {
                    if (imeKeySynthesized.remove(keyCode)) {
                        return true;
                    }
                    return false;
                }

                @Override
                public boolean onTouchEvent(android.view.MotionEvent event) {
                    // Purely a keyboard target: never intercept touch, which belongs to the native
                    // surface underneath.
                    return false;
                }
            };
            imeView.setBackground(null);
            imeView.setCursorVisible(false);
            imeView.setTextColor(0);
            imeView.setAlpha(0.01f);
            imeView.setSoundEffectsEnabled(false);
            imeView.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
            imeView.setImeOptions(EditorInfo.IME_FLAG_NO_FULLSCREEN);
            imeView.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
            FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(1, 1);
            params.gravity = Gravity.TOP | Gravity.START;
            addContentView(imeView, params);
        } catch (Throwable error) {
            Log.w(TAG, "Could not install the soft-keyboard host view", error);
        }
    }

    /** Show the soft keyboard by focusing the IME host view. Called by the Rust shell via JNI. */
    public void imeShow() {
        final EditText view = imeView;
        if (view == null) {
            return;
        }
        imeWanted = true;
        runOnUiThread(() -> {
            if (!imeWanted) {
                return;
            }
            try {
                view.setFocusable(true);
                view.setFocusableInTouchMode(true);
                view.requestFocus();
                InputMethodManager imm =
                        (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                if (imm != null && imm.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT)) {
                    return;
                }
                // Some devices ignore the request until focus and layout have settled; retry once on
                // the next message-loop turn (guarded so a following hide wins).
                view.post(() -> {
                    if (!imeWanted) {
                        return;
                    }
                    try {
                        InputMethodManager retry =
                                (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                        if (retry != null) {
                            retry.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT);
                        }
                    } catch (Throwable error) {
                        Log.w(TAG, "Could not show the soft keyboard", error);
                    }
                });
            } catch (Throwable error) {
                Log.w(TAG, "Could not show the soft keyboard", error);
            }
        });
    }

    /** Hide the soft keyboard and release the IME host view. Called by the Rust shell via JNI. */
    public void imeHide() {
        imeWanted = false;
        final EditText view = imeView;
        if (view == null) {
            return;
        }
        runOnUiThread(() -> {
            try {
                InputMethodManager imm =
                        (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
                }
                view.clearFocus();
                // Drop the scratch buffer between sessions so it cannot grow without bound.
                view.setText("");
            } catch (Throwable error) {
                Log.w(TAG, "Could not hide the soft keyboard", error);
            }
        });
    }

    /**
     * Queue one keyboard event for the Rust shell, and wake the native loop so the same frame
     * reads it.
     *
     * The native loop (android-activity, then winit and eframe) sleeps in ALooper_pollOnce until
     * something arrives, and from here only an input event on its own queue can wake it. A queued
     * keyboard event is not an input event: it would wait in this queue until the loop next woke
     * for another reason — the next frame timer, a touch, or egui's cursor blink, up to half a
     * second later — which is the lag felt between a backspace and the character going away.
     *
     * GameActivity.onKeyDown hands a key event to the native queue (onKeyDownNative, whose glue
     * calls ALooper_wake), so it is what wakes the loop. KEYCODE_UNKNOWN reaches winit as
     * Key::Unidentified, which no widget acts on: the app sees a wake-up and nothing else. Both
     * the press and the release go, or the key would look held down.
     */
    private void queueImeEvent(String event) {
        imeEvents.offer(event);
        try {
            super.onKeyDown(
                    KeyEvent.KEYCODE_UNKNOWN,
                    new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_UNKNOWN));
            super.onKeyUp(
                    KeyEvent.KEYCODE_UNKNOWN,
                    new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_UNKNOWN));
        } catch (Throwable error) {
            Log.w(TAG, "Could not wake the native loop for a keyboard event", error);
        }
    }

    /**
     * Whether a delete call should be handed to the shell, or dropped as the second half of one
     * press an IME reported twice.
     *
     * Some IMEs answer one backspace with two different calls — `deleteSurroundingText(1,0)` and
     * `sendKeyEvent(KEYCODE_DEL)`. They arrive back to back in the same dispatch, so a call of the
     * *other* method within {@link #IME_DELETE_PAIR_MS} of this one is the repeat and is dropped.
     * The same method twice is always allowed: two quick taps of one key are two deletions.
     */
    private boolean claimDelete(String method) {
        long now = android.os.SystemClock.uptimeMillis();
        long lastOther = "d".equals(method) ? lastImeDeleteKeyAt : lastImeDeleteSurroundingAt;
        if (lastOther != 0L && now - lastOther < IME_DELETE_PAIR_MS) {
            return false;
        }
        if ("d".equals(method)) {
            lastImeDeleteSurroundingAt = now;
        } else {
            lastImeDeleteKeyAt = now;
        }
        return true;
    }

    /**
     * Hand the Rust shell at most one queued keyboard event per call. The first character tags the
     * event ("c" commit, "p" pre-edit, "f" finish, "d" delete-with-counts, "a" editor action) and
     * the remainder is the payload; returns {@code null} when the queue is empty.
     */
    public String imePoll() {
        return imeEvents.poll();
    }

    /**
     * Hand the Rust shell the private path of one file the system asked us to open (or that another
     * app shared with us), or {@code null} when nothing is queued. Each staged copy is delivered
     * once and then opened exactly like a file chosen in the open picker.
     */
    public String intentPoll() {
        return intentPaths.poll();
    }

    /**
     * How many files the system (or another app's drop) handed us that the shell has not opened
     * yet. Asked without taking anything off the queue, so the frame that opens the file still
     * finds it there. The shell's watcher thread uses it to wake an idle app: egui runs a frame
     * only when something asks it to, so a file that arrives while the app sits untouched would
     * otherwise wait in the queue until the user happened to touch the screen.
     */
    public int intentPending() {
        return intentPaths.size();
    }

    /** Every {@code content://} URI the drag carries (text-only drags yield nothing). */
    private List<Uri> collectDragUris(ClipData clip) {
        List<Uri> uris = new ArrayList<>();
        if (clip == null) return uris;
        for (int i = 0; i < clip.getItemCount(); i++) {
            Uri uri = clip.getItemAt(i).getUri();
            if (uri != null) uris.add(uri);
        }
        return uris;
    }

    /**
     * Best-effort persistable grant for a dropped URI. Drag-and-drop only grants temporary
     * access, so a source that offered a persistable grant (the system file picker does) keeps
     * the copy below readable even after the drag ends; the rest are copied while the drop's
     * temporary grant still holds. Write access is asked for first, because a dropped document
     * is saved back to the place it came from (the staged copy remembers the source URI), and a
     * provider that only offered read access still gets the read grant.
     */
    private void persistDroppedPermission(Uri uri) {
        Exception failure = null;
        int[] grants = {
                Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                Intent.FLAG_GRANT_READ_URI_PERMISSION,
        };
        for (int flags : grants) {
            try {
                getContentResolver().takePersistableUriPermission(uri, flags);
                return;
            } catch (Exception e) {
                failure = e;
            }
        }
        Log.i(TAG, "Provider did not offer a persistable grant for " + uri, failure);
    }

    /**
     * Copy dropped files into app-private staging and queue their paths for the Rust shell,
     * mirroring the shared-file path: the copy runs off the UI thread and the shell picks each
     * path up on a later {@link #intentPoll()}.
     */
    private void stageDroppedFiles(List<Uri> uris) {
        io.execute(() -> {
            for (Uri uri : uris) {
                try {
                    File stage = newStageFile(displayName(uri), uri);
                    copyUriToFile(uri, stage);
                    rememberUri(stage, uri);
                    intentPaths.offer(stage.getAbsolutePath());
                    Log.i(TAG, "Dropped file staged: " + stage.getAbsolutePath());
                } catch (Exception e) {
                    Log.e(TAG, "Could not stage the dropped file: " + uri, e);
                }
            }
        });
    }

    /**
     * Listen for drags on the whole window: the decor, the content frame and every child of it
     * (the native surface and the keyboard host), so the drop is caught whichever view the
     * system considers to be under the pointer. Views that don't handle a drag pass it on, so
     * the same listener on all of them never conflicts.
     */
    private void installDropTarget() {
        try {
            Window window = getWindow();
            if (window == null) return;
            View decor = window.getDecorView();
            decor.setOnDragListener(dropListener);
            View content = decor.findViewById(android.R.id.content);
            if (content == null) return;
            content.setOnDragListener(dropListener);
            if (content instanceof ViewGroup) {
                ViewGroup group = (ViewGroup) content;
                for (int i = 0; i < group.getChildCount(); i++) {
                    group.getChildAt(i).setOnDragListener(dropListener);
                }
            }
        } catch (Throwable error) {
            Log.e(TAG, "Could not install the drag-and-drop target", error);
        }
    }

    public String pcPollPickerResult() { return results.poll(); }

    public void pcRequestOpen(long id, boolean multiple) {
        runOnUiThread(() -> {
            try {
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                requestIds.put(REQ_OPEN, id);
                startActivityForResult(intent, REQ_OPEN);
            } catch (Exception e) {
                enqueueError(id, "open", e.toString());
            }
        });
    }

    public void pcRequestSave(long id, String suggestedName) {
        runOnUiThread(() -> {
            try {
                String name = safeName(suggestedName == null ? "Untitled.pcraft" : suggestedName);
                Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mimeFor(name));
                intent.putExtra(Intent.EXTRA_TITLE, name);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                requestIds.put(REQ_SAVE, id);
                suggestedNames.put(id, name);
                startActivityForResult(intent, REQ_SAVE);
            } catch (Exception e) {
                enqueueError(id, "save", e.toString());
            }
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQ_OPEN && requestCode != REQ_SAVE) return;
        Long boxedId = requestIds.remove(requestCode);
        if (boxedId == null) return;
        final long id = boxedId;
        final String kind = requestCode == REQ_OPEN ? "open" : "save";
        if (resultCode != RESULT_OK || data == null) {
            suggestedNames.remove(id);
            enqueueCancel(id, kind);
            return;
        }
        if (requestCode == REQ_SAVE) {
            Uri uri = data.getData();
            if (uri == null) { enqueueCancel(id, kind); return; }
            persistPermission(uri, data);
            try {
                String name = displayName(uri);
                if (name == null || name.trim().isEmpty()) name = suggestedNames.getOrDefault(id, "Untitled.pcraft");
                File stage = newStageFile(name, uri);
                rememberUri(stage, uri);
                enqueueSave(id, stage.getAbsolutePath());
            } catch (Exception e) {
                enqueueError(id, kind, e.toString());
            } finally {
                suggestedNames.remove(id);
            }
            return;
        }

        List<Uri> uris = new ArrayList<>();
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) {
                Uri uri = clip.getItemAt(i).getUri();
                if (uri != null) uris.add(uri);
            }
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        if (uris.isEmpty()) { enqueueCancel(id, kind); return; }
        for (Uri uri : uris) persistPermission(uri, data);
        io.execute(() -> {
            try {
                JSONArray paths = new JSONArray();
                for (Uri uri : uris) {
                    File stage = newStageFile(displayName(uri), uri);
                    copyUriToFile(uri, stage);
                    rememberUri(stage, uri);
                    paths.put(stage.getAbsolutePath());
                }
                JSONObject result = base(id, "open", "ok");
                result.put("paths", paths);
                results.offer(result.toString());
            } catch (Exception e) {
                enqueueError(id, "open", e.toString());
            }
        });
    }

    /** Commit an app-private staged file to its remembered SAF URI; used by Rust after every save. */
    public String pcCommitIfMapped(String path) {
        try {
            String uriText = getSharedPreferences(PREFS, MODE_PRIVATE).getString("uri:" + path, null);
            if (uriText == null) return "NONE";
            Uri uri = Uri.parse(uriText);
            OutputStream out = getContentResolver().openOutputStream(uri, "wt");
            if (out == null) out = getContentResolver().openOutputStream(uri, "w");
            if (out == null) return "ERROR: Android document provider did not open an output stream";
            try (InputStream in = new FileInputStream(new File(path)); OutputStream target = out) {
                byte[] buffer = new byte[64 * 1024];
                int count;
                while ((count = in.read(buffer)) != -1) target.write(buffer, 0, count);
                target.flush();
            }
            return "OK";
        } catch (Exception e) {
            Log.e(TAG, "Could not write a selected document", e);
            return "ERROR: Could not write the selected document: " + e.getMessage();
        }
    }

    /**
     * The working copy of a document the user chose through SAF. It lives under {@link #appStorageDir()}
     * so a file manager or a computer finds the file the app is editing, and inside a plain
     * {@code media} directory named after the document rather than a random per-open directory, so the
     * same document always lands on the same path.
     *
     * <p>A document that is opened again reuses its staged file; a different document whose name is
     * already taken gets a {@code " (2)"}-style suffix, because the staged path is the key that maps
     * back to the picked document and two documents sharing one path would overwrite each other.
     * A name whose staged copy has since been deleted (the app prunes working copies the recent list
     * let go of) is free again: the stale {@code uri:} mapping is harmless.
     */
    private File newStageFile(String name, Uri uri) throws Exception {
        File dir = new File(appStorageDir(), "media");
        if (!dir.mkdirs() && !dir.isDirectory()) throw new Exception("Could not create the media directory");
        String safe = safeName(name == null ? "imported-file" : name);
        String stem = safe, extension = "";
        int dot = safe.lastIndexOf('.');
        if (dot > 0) { stem = safe.substring(0, dot); extension = safe.substring(dot); }
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        String owner = uri == null ? null : uri.toString();
        for (int attempt = 1; attempt <= 10000; attempt++) {
            String candidate = attempt == 1 ? safe : stem + " (" + attempt + ")" + extension;
            File file = new File(dir, candidate);
            String mapped = prefs.getString("uri:" + file.getAbsolutePath(), null);
            if (mapped == null || mapped.equals(owner) || !file.exists()) return file;
        }
        throw new Exception("Could not find a free name for the selected document in " + dir);
    }

    private static String safeName(String name) {
        String clean = name.replace('\\', '/');
        int slash = clean.lastIndexOf('/');
        if (slash >= 0) clean = clean.substring(slash + 1);
        clean = clean.replaceAll("[^A-Za-z0-9._() -]", "_").trim();
        if (clean.isEmpty() || clean.equals(".") || clean.equals("..")) return "untitled";
        return clean;
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (index >= 0) return safeName(cursor.getString(index));
            }
        } catch (Exception ignored) { }
        String segment = uri.getLastPathSegment();
        return segment == null ? "imported-file" : safeName(segment);
    }

    private void persistPermission(Uri uri, Intent data) {
        try {
            int flags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            if (flags != 0) getContentResolver().takePersistableUriPermission(uri, flags);
        } catch (Exception e) {
            Log.i(TAG, "Provider did not offer a persistable grant for " + uri, e);
        }
    }

    private void rememberUri(File path, Uri uri) {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString("uri:" + path.getAbsolutePath(), uri.toString()).apply();
    }

    private void copyUriToFile(Uri uri, File destination) throws Exception {
        try (android.os.ParcelFileDescriptor pfd = getContentResolver().openFileDescriptor(uri, "r")) {
            if (pfd != null) {
                try (FileInputStream in = new FileInputStream(pfd.getFileDescriptor());
                     FileOutputStream out = new FileOutputStream(destination)) {
                    java.nio.channels.FileChannel inChannel = in.getChannel();
                    java.nio.channels.FileChannel outChannel = out.getChannel();
                    long size = inChannel.size();
                    long transferred = 0;
                    while (transferred < size) {
                        long count = outChannel.transferFrom(inChannel, transferred, size - transferred);
                        if (count <= 0) break;
                        transferred += count;
                    }
                    return;
                }
            }
        } catch (Exception ignored) { }
        InputStream input = getContentResolver().openInputStream(uri);
        if (input == null) throw new Exception("Android document provider did not open the selected file");
        try (InputStream in = input; FileOutputStream out = new FileOutputStream(destination)) {
            byte[] buffer = new byte[256 * 1024];
            int count;
            while ((count = in.read(buffer)) != -1) out.write(buffer, 0, count);
            out.flush();
        }
    }

    /**
     * Collect the file(s) an {@code ACTION_VIEW} / {@code ACTION_SEND} intent carries: the data URI
     * (opening a registered type), the intent's {@link ClipData} (modern shares) and the legacy
     * {@code EXTRA_STREAM} extra, de-duplicated. Anything else (in particular the launcher's MAIN
     * intent) is ignored so normal launches are untouched.
     */
    private static List<Uri> collectIncomingUris(Intent intent) {
        LinkedHashSet<Uri> uris = new LinkedHashSet<>();
        String action = intent.getAction();
        if (Intent.ACTION_VIEW.equals(action)) {
            if (intent.getData() != null) uris.add(intent.getData());
        } else if (Intent.ACTION_SEND.equals(action)) {
            Uri stream = intent.getParcelableExtra(Intent.EXTRA_STREAM);
            if (stream != null) uris.add(stream);
        } else if (Intent.ACTION_SEND_MULTIPLE.equals(action)) {
            ArrayList<Uri> streams = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
            if (streams != null) uris.addAll(streams);
        } else {
            return new ArrayList<>();
        }
        ClipData clip = intent.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) {
                Uri uri = clip.getItemAt(i).getUri();
                if (uri != null) uris.add(uri);
            }
        }
        return new ArrayList<>(uris);
    }

    /**
     * Copy every file the given intent carries into app-private staging and queue its path for the
     * Rust shell. Runs off the UI thread because the copy can be large; the shell polls
     * {@link #intentPoll()} until the path appears, so a slow copy never blocks startup.
     */
    private void handleIncomingFiles(Intent intent) {
        if (intent == null) return;
        List<Uri> uris = collectIncomingUris(intent);
        if (uris.isEmpty()) return;
        for (Uri uri : uris) persistPermission(uri, intent);
        io.execute(() -> {
            for (Uri uri : uris) {
                try {
                    File stage = newStageFile(displayName(uri), uri);
                    copyUriToFile(uri, stage);
                    rememberUri(stage, uri);
                    intentPaths.offer(stage.getAbsolutePath());
                } catch (Exception e) {
                    Log.e(TAG, "Could not open the file the system handed the app: " + uri, e);
                }
            }
        });
    }

    private JSONObject base(long id, String kind, String status) throws Exception {
        JSONObject value = new JSONObject();
        value.put("id", id);
        value.put("kind", kind);
        value.put("status", status);
        return value;
    }

    private void enqueueCancel(long id, String kind) {
        try { results.offer(base(id, kind, "cancel").toString()); }
        catch (Exception e) { Log.e(TAG, "Could not encode cancellation", e); }
    }

    private void enqueueError(long id, String kind, String message) {
        try {
            JSONObject value = base(id, kind, "error");
            value.put("message", message == null ? "Unknown Android file picker error" : message);
            results.offer(value.toString());
        } catch (Exception e) { Log.e(TAG, "Could not encode picker error", e); }
    }

    private void enqueueSave(long id, String path) {
        try {
            JSONObject value = base(id, "save", "ok");
            value.put("path", path);
            results.offer(value.toString());
        } catch (Exception e) { enqueueError(id, "save", e.toString()); }
    }

    private static String mimeFor(String fileName) {
        String lower = fileName.toLowerCase(java.util.Locale.ROOT);
        if (lower.endsWith(".png")) return "image/png";
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
        if (lower.endsWith(".webp")) return "image/webp";
        if (lower.endsWith(".gif")) return "image/gif";
        if (lower.endsWith(".tif") || lower.endsWith(".tiff")) return "image/tiff";
        if (lower.endsWith(".bmp")) return "image/bmp";
        if (lower.endsWith(".svg")) return "image/svg+xml";
        return "application/octet-stream";
    }
}
