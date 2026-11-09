package com.artcraft.vectorcraft;

import android.Manifest;
import android.content.ClipData;
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
import android.util.DisplayMetrics;
import android.util.Log;
import android.view.DragEvent;
import android.view.ActionMode;
import android.view.Display;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;

import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import android.app.NativeActivity;
import android.content.Context;
import android.content.res.Configuration;
import android.os.Looper;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import java.util.concurrent.atomic.AtomicInteger;

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
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * NativeActivity subclass that supplies Android Storage Access Framework (SAF) open/save results to
 * the Rust UI shell for VectorCraft.
 *
 * <p>Documents the user picks are staged as ordinary files under the app's external {@code media}
 * directory ({@code Android/data/<pkg>/files/media}, with internal storage only as a fallback) so the
 * Rust side can read them with the standard file API. For a save, the staged file's path is remembered
 * against the chosen SAF URI, and {@link #vcCommitIfMapped(String)} copies the staged bytes back
 * through ContentResolver after every write.
 */
public final class VectorCraftActivity extends NativeActivity {
    private static final String TAG = "VectorCraftSAF";
    private static final int REQ_OPEN = 0x5643;
    private static final int REQ_SAVE = 0x5644;
    private static final int REQ_PERMISSIONS = 0x5645;
    private static final String PREFS = "vectorcraft_saf_uri_map";

    private final ConcurrentLinkedQueue<String> results = new ConcurrentLinkedQueue<>();
    private final Map<Integer, Long> requestIds = new HashMap<>();
    private final Map<Long, String> suggestedNames = new HashMap<>();
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private int captionBarPadding = 0;
    private boolean lightTheme = false;
    private View.OnApplyWindowInsetsListener chainedInsetsListener;

    private final ConcurrentLinkedQueue<String> imeQueue = new ConcurrentLinkedQueue<>();
    private ArtCraftInputView inputView;

    private void wakeNativeLoop() {
        try {
            com.artcraft.ime.ArtCraftIme.wake();
        } catch (Throwable ignored) {}
        Runnable r = () -> {
            try {
                dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_UNKNOWN));
                dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_UNKNOWN));
            } catch (Throwable ignored) {}
        };
        if (Looper.myLooper() == Looper.getMainLooper()) {
            r.run();
        } else {
            runOnUiThread(r);
        }
    }

    private static boolean isFromSoftKeyboard(KeyEvent event) {
        if (event == null) return true;
        if ((event.getFlags() & KeyEvent.FLAG_SOFT_KEYBOARD) != 0) return true;
        if (event.getDeviceId() <= 0) return true;
        InputDevice device = event.getDevice();
        if (device == null || device.isVirtual()) return true;
        return (event.getSource() & InputDevice.SOURCE_KEYBOARD) != InputDevice.SOURCE_KEYBOARD;
    }

    public void onImeCommit(String text) {
        if (text == null || text.isEmpty()) return;
        imeQueue.add(text);
        wakeNativeLoop();
    }

    public void onImeDelete() {
        imeQueue.add("\b");
        wakeNativeLoop();
    }

    public void onImeEnter() {
        imeQueue.add("\n");
        wakeNativeLoop();
    }

    public String pollIme() {
        return imeQueue.poll();
    }

    public void imeShow() {
        runOnUiThread(() -> {
            try {
                if (inputView != null) {
                    inputView.setVisibility(View.VISIBLE);
                    inputView.setFocusable(true);
                    inputView.setFocusableInTouchMode(true);
                    inputView.requestFocus();
                    InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                    if (imm != null) {
                        imm.restartInput(inputView);
                        imm.showSoftInput(inputView, 0);
                    }
                }
            } catch (Throwable ignored) {}
        });
    }

    public void imeHide() {
        runOnUiThread(() -> {
            try {
                InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    View decor = getWindow() != null ? getWindow().getDecorView() : null;
                    if (decor != null) {
                        imm.hideSoftInputFromWindow(decor.getWindowToken(), 0);
                    }
                }
                if (inputView != null) {
                    inputView.clearEditable();
                    inputView.clearFocus();
                    inputView.setFocusable(false);
                    inputView.setFocusableInTouchMode(false);
                    inputView.setVisibility(View.GONE);
                }
                Window window = getWindow();
                if (window != null) {
                    View content = window.getDecorView().findViewById(android.R.id.content);
                    if (content instanceof ViewGroup) {
                        ViewGroup vg = (ViewGroup) content;
                        for (int i = 0; i < vg.getChildCount(); i++) {
                            View child = vg.getChildAt(i);
                            if (child != inputView) {
                                child.setFocusable(true);
                                child.setFocusableInTouchMode(true);
                                child.requestFocus();
                                break;
                            }
                        }
                    }
                }
            } catch (Throwable ignored) {}
        });
    }

    private void installInputView() {
        try {
            Window window = getWindow();
            if (window == null) return;
            View decor = window.getDecorView();
            ViewGroup content = decor.findViewById(android.R.id.content);
            if (content != null && inputView == null) {
                inputView = new ArtCraftInputView(this);
                content.addView(inputView, new ViewGroup.LayoutParams(1, 1));
                inputView.setVisibility(View.GONE);
                inputView.setFocusable(false);
                inputView.setFocusableInTouchMode(false);
            }
        } catch (Throwable e) {
            Log.e(TAG, "Failed to install input view", e);
        }
    }

    private final class ObservableEditable extends SpannableStringBuilder {
        @Override
        public SpannableStringBuilder replace(int start, int end, CharSequence tb, int tbstart, int tbend) {
            if (inputView != null && !inputView.suppressingWatcher) {
                int deleteCount = end - start;
                if (deleteCount > 0) {
                    for (int i = 0; i < deleteCount; i++) {
                        onImeDelete();
                    }
                }
                int insertCount = tbend - tbstart;
                if (insertCount > 0 && tb != null) {
                    String inserted = tb.subSequence(tbstart, tbend).toString();
                    if (!inserted.isEmpty()) {
                        onImeCommit(inserted);
                    }
                }
            }
            SpannableStringBuilder result = super.replace(start, end, tb, tbstart, tbend);
            if (inputView != null && !inputView.suppressingWatcher && length() > 2000) {
                inputView.suppressingWatcher = true;
                try {
                    super.delete(0, length() - 500);
                    Selection.setSelection(this, length());
                } finally {
                    inputView.suppressingWatcher = false;
                }
            }
            return result;
        }
    }

    private final class ArtCraftInputView extends View {
        private final ObservableEditable editable = new ObservableEditable();
        private boolean suppressingWatcher = false;

        public ArtCraftInputView(Context context) {
            super(context);
            setFocusable(false);
            setFocusableInTouchMode(false);
            setVisibility(View.GONE);
            setWillNotDraw(true);
        }

        public void clearEditable() {
            suppressingWatcher = true;
            try {
                editable.clear();
                Selection.setSelection(editable, 0);
            } finally {
                suppressingWatcher = false;
            }
        }

        public void deleteLastFromEditable() {
            if (editable.length() == 0) return;
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
            return isFocused() && getVisibility() == View.VISIBLE;
        }

        @Override
        public boolean onTouchEvent(MotionEvent event) {
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
                    if (event != null && !isFromSoftKeyboard(event)) {
                        return super.sendKeyEvent(event);
                    }
                    if (event != null && isFromSoftKeyboard(event)) {
                        if (event.getAction() == KeyEvent.ACTION_DOWN) {
                            if (event.getKeyCode() == KeyEvent.KEYCODE_DEL) {
                                deleteLastFromEditable();
                                onImeDelete();
                                return true;
                            } else if (event.getKeyCode() == KeyEvent.KEYCODE_ENTER) {
                                onImeEnter();
                                return true;
                            }
                        } else if (event.getAction() == KeyEvent.ACTION_UP) {
                            if (event.getKeyCode() == KeyEvent.KEYCODE_DEL || event.getKeyCode() == KeyEvent.KEYCODE_ENTER) {
                                return true;
                            }
                        }
                    }
                    return super.sendKeyEvent(event);
                }

                @Override
                public boolean performEditorAction(int actionCode) {
                    onImeEnter();
                    return true;
                }
            };
        }
    }

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

    /** App-private storage root, used by the Rust shell for preferences and scratch data. */
    public String vcFilesDir() {
        return appStorageDir().getAbsolutePath();
    }

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
     * Hide the status and navigation bars so the egui canvas owns the whole screen.
     *
     * <p>The theme's {@code windowFullscreen} flag only hides the status bar; the navigation bar
     * stays until the window is put in sticky-immersive mode, and the system restores the bars
     * after a gesture or when the SAF picker activity closes. {@code onWindowFocusChanged} re-applies
     * it when focus returns.
     */
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

    public boolean isDesktopOrFreeform() {
        try {
            if (isInMultiWindowMode()) return true;
            Configuration config = getResources().getConfiguration();
            if ((config.uiMode & Configuration.UI_MODE_TYPE_MASK) == Configuration.UI_MODE_TYPE_DESK) {
                return true;
            }
            if (config.toString().contains("semDesktopModeEnabled=1") || config.toString().contains("desk-mode")) {
                return true;
            }
            PackageManager pm = getPackageManager();
            if (pm != null && (
                    pm.hasSystemFeature("org.chromium.arc") ||
                    pm.hasSystemFeature("org.chromium.arc.device_management") ||
                    (Build.VERSION.SDK_INT >= 27 && pm.hasSystemFeature(PackageManager.FEATURE_PC))
            )) {
                return true;
            }
        } catch (Throwable ignored) {}
        return false;
    }

    public int getSafeInsetTop() {
        return captionBarPadding;
    }

    private void applyImmersiveFullscreen() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        if (isDesktopOrFreeform()) {
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

    private void applyEdgeToEdge() {
        Window window = getWindow();
        if (window == null) {
            return;
        }
        WindowCompat.setDecorFitsSystemWindows(window, false);
        try {
            window.setStatusBarColor(0x00000000);
            window.setNavigationBarColor(0x00000000);
        } catch (Throwable error) {
            Log.i(TAG, "This device does not accept transparent system bars", error);
        }
    }

    private void applyCaptionBarInsets(WindowInsets insets) {
        try {
            Window window = getWindow();
            if (window == null) {
                return;
            }
            View decor = window.getDecorView();
            View content = decor.findViewById(android.R.id.content);
            if (content == null) {
                return;
            }
            int top = 0;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsets rootInsets = insets != null ? insets : decor.getRootWindowInsets();
                if (rootInsets != null) {
                    top = rootInsets.getInsets(WindowInsets.Type.captionBar()).top;
                    if (top == 0 && isDesktopOrFreeform()) {
                        top = rootInsets.getInsetsIgnoringVisibility(
                                WindowInsets.Type.captionBar() | WindowInsets.Type.statusBars()).top;
                    }
                }
            } else if (insets != null) {
                top = insets.getSystemWindowInsetTop();
            }
            if (top == captionBarPadding) {
                return;
            }
            captionBarPadding = top;
            content.setBackgroundColor(top > 0 ? (lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E) : 0x00000000);
            content.setPadding(0, top, 0, 0);
            if (content.getLayoutParams() instanceof ViewGroup.MarginLayoutParams) {
                ViewGroup.MarginLayoutParams mlp = (ViewGroup.MarginLayoutParams) content.getLayoutParams();
                if (mlp.topMargin != top) {
                    mlp.topMargin = top;
                    content.setLayoutParams(mlp);
                    content.requestLayout();
                }
            }
        } catch (Throwable error) {
            Log.i(TAG, "Could not inset the interface below the window's title bar", error);
        }
    }

    private void installCaptionBarInsets() {
        try {
            Window window = getWindow();
            if (window == null) {
                return;
            }
            View decor = window.getDecorView();
            decor.setOnApplyWindowInsetsListener((view, insets) -> {
                applyCaptionBarInsets(insets);
                View.OnApplyWindowInsetsListener next = chainedInsetsListener;
                return next != null ? next.onApplyWindowInsets(view, insets) : insets;
            });
            decor.requestApplyInsets();
        } catch (Throwable error) {
            Log.i(TAG, "Could not watch the window's title-bar insets", error);
        }
    }

    public void setBarIconAppearance(boolean lightTheme) {
        runOnUiThread(() -> {
            try {
                this.lightTheme = lightTheme;
                if (captionBarPadding > 0) {
                    View content = getWindow() != null
                            ? getWindow().getDecorView().findViewById(android.R.id.content) : null;
                    if (content != null) {
                        content.setBackgroundColor(lightTheme ? 0xFFF5F5F5 : 0xFF1E1E1E);
                    }
                }
                Window window = getWindow();
                if (window == null) {
                    return;
                }
                WindowInsetsControllerCompat insetsController =
                        WindowCompat.getInsetsController(window, window.getDecorView());
                if (insetsController != null) {
                    insetsController.setAppearanceLightStatusBars(lightTheme);
                    insetsController.setAppearanceLightNavigationBars(lightTheme);
                }
            } catch (Throwable error) {
                Log.e(TAG, "Could not style the system-bar icons", error);
            }
        });
    }

    private void suppressSelectionHandles() {
        try {
            View decor = getWindow() != null ? getWindow().getDecorView() : null;
            if (decor != null) {
                suppressLongClick(decor);
                decor.post(() -> suppressLongClick(decor));
            }
        } catch (Throwable error) {
            Log.i(TAG, "Could not suppress selection handles", error);
        }
    }

    private void suppressLongClick(View view) {
        if (view == null) return;
        view.setLongClickable(false);
        view.setOnLongClickListener(v -> true);
        view.setOnHoverListener(null);
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int i = 0; i < group.getChildCount(); i++) {
                suppressLongClick(group.getChildAt(i));
            }
        }
    }

    private static final int SAMSUNG_PEN_DOWN = 211;
    private boolean sideButtonHeld = false;

    @Override
    public boolean dispatchTouchEvent(MotionEvent e) {
        if (e == null) return super.dispatchTouchEvent(e);
        int samsung = e.getActionMasked() - SAMSUNG_PEN_DOWN;
        sideButtonHeld = samsung >= 0 && samsung <= 3;
        if (sideButtonHeld) {
            int[] standard = {MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_MOVE, MotionEvent.ACTION_CANCEL};
            e.setAction(standard[samsung] | (e.getAction() & MotionEvent.ACTION_POINTER_INDEX_MASK));
        }

        int action = e.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) {
            int i = e.getActionIndex();
            int tool = e.getToolType(i);
            if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
                    getWindow().getDecorView().requestUnbufferedDispatch(e);
                }
            }
        }
        return super.dispatchTouchEvent(e);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        if (event != null) {
            int tool = event.getToolType(event.getActionIndex());
            if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
                    getWindow().getDecorView().requestUnbufferedDispatch(event);
                }
            }
            if (event.isFromSource(android.view.InputDevice.SOURCE_TOUCHPAD) ||
                event.isFromSource(android.view.InputDevice.SOURCE_MOUSE) ||
                event.isFromSource(android.view.InputDevice.SOURCE_STYLUS)) {
                return super.dispatchGenericMotionEvent(event);
            }
        }
        return super.dispatchGenericMotionEvent(event);
    }

    @Override
    public ActionMode onWindowStartingActionMode(ActionMode.Callback callback, int type) {
        return null;
    }

    @Override
    public ActionMode onWindowStartingActionMode(ActionMode.Callback callback) {
        return null;
    }

    @Override
    protected void onResume() {
        super.onResume();
        suppressSelectionHandles();
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        applyEdgeToEdge();
        applyImmersiveFullscreen();
        // The native surface view is attached after onCreate and can reset the window state:
        // re-apply once the first layout pass runs.
        Window window = getWindow();
        if (window != null) {
            window.getDecorView().post(this::applyImmersiveFullscreen);
        }
        installCaptionBarInsets();
        suppressSelectionHandles();
        installDropTarget();
        installInputView();
        // Once, on the first launch: the settings screen that grants the app access to the device's
        // storage (the font folders the shell links included). Answered later, outside the app, so
        // the links the shell makes at startup are refreshed when the access arrives.
        requestStoragePermission();
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
     * On a real finish, end the process instead of running NativeActivity's teardown: the game
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
            suppressSelectionHandles();
            if (inputView != null && inputView.getVisibility() == View.VISIBLE && !inputView.isFocused()) {
                inputView.requestFocus();
            }
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        applyImmersiveFullscreen();
        try {
            dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_UNKNOWN));
            dispatchKeyEvent(new KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_UNKNOWN));
        } catch (Throwable ignored) {}
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

    /** One queued picker result as JSON, or null; polled by the Rust shell every frame. */
    public String vcPollPickerResult() {
        return results.poll();
    }

    /**
     * Launch the SAF open picker. The result (staged file paths or a cancellation) is queued for
     * {@link #vcPollPickerResult()}.
     *
     * @param id       opaque request id chosen by the caller
     * @param multiple whether several files may be picked (Place…)
     * @param mime     the MIME type to filter on, or null/"*&#47;*" for every file
     */
    public void vcRequestOpen(long id, boolean multiple, String mime) {
        runOnUiThread(() -> {
            try {
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mime == null || mime.isEmpty() ? "*/*" : mime);
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                requestIds.put(REQ_OPEN, id);
                startActivityForResult(intent, REQ_OPEN);
            } catch (Exception e) {
                enqueueError(id, "open", e.toString());
            }
        });
    }

    /**
     * Launch the SAF create-document picker. The chosen document's staged path is queued; bytes
     * written to that path are copied to the chosen document by {@link #vcCommitIfMapped(String)}.
     *
     * @param id            opaque request id chosen by the caller
     * @param suggestedName the file name to suggest, including the extension
     * @param mime          the MIME type to use, or null to derive one from the name
     */
    public void vcRequestSave(long id, String suggestedName, String mime) {
        runOnUiThread(() -> {
            try {
                String name = safeName(suggestedName == null ? "Untitled.svg" : suggestedName);
                Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mime == null || mime.isEmpty() ? mimeFor(name) : mime);
                intent.putExtra(Intent.EXTRA_TITLE, name);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
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
        if (requestCode != REQ_OPEN && requestCode != REQ_SAVE) {
            return;
        }
        Long boxedId = requestIds.remove(requestCode);
        if (boxedId == null) {
            return;
        }
        final long id = boxedId;
        final String kind = requestCode == REQ_OPEN ? "open" : "save";
        if (resultCode != RESULT_OK || data == null) {
            suggestedNames.remove(id);
            enqueueCancel(id, kind);
            return;
        }
        if (requestCode == REQ_SAVE) {
            Uri uri = data.getData();
            if (uri == null) {
                enqueueCancel(id, kind);
                return;
            }
            persistPermission(uri, data);
            try {
                String name = displayName(uri);
                if (name == null || name.trim().isEmpty()) {
                    name = suggestedNames.getOrDefault(id, "Untitled.svg");
                }
                name = keepSuggestedExtension(name, suggestedNames.get(id));
                File stage = newStageFile(name, uri);
                rememberUri(stage, uri);
                enqueuePaths(id, "save", new String[] { stage.getAbsolutePath() });
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
                if (uri != null) {
                    uris.add(uri);
                }
            }
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        if (uris.isEmpty()) {
            enqueueCancel(id, kind);
            return;
        }
        for (Uri uri : uris) {
            persistPermission(uri, data);
        }
        io.execute(() -> {
            try {
                JSONArray paths = new JSONArray();
                for (Uri uri : uris) {
                    File stage = newStageFile(displayName(uri), uri);
                    copyUriToFile(uri, stage);
                    rememberUri(stage, uri);
                    paths.put(stage.getAbsolutePath());
                }
                enqueuePathsArray(id, "open", paths);
            } catch (Exception e) {
                enqueueError(id, "open", e.toString());
            }
        });
    }

    /**
     * Commit an app-private staged file to its remembered SAF URI; called by the Rust shell after
     * every write. Returns {@code "NONE"} for an ordinary private path, {@code "OK"} after a
     * successful commit, or {@code "ERROR:<message>"} when the provider refused the write.
     */
    public String vcCommitIfMapped(String path) {
        try {
            String uriText = getSharedPreferences(PREFS, MODE_PRIVATE).getString("uri:" + path, null);
            if (uriText == null) {
                uriText = uriForSibling(path);
            }
            if (uriText == null) {
                return "NONE";
            }
            Uri uri = Uri.parse(uriText);
            File srcFile = new File(path);
            if (!srcFile.exists()) {
                return "ERROR: Staged file does not exist: " + path;
            }
            OutputStream out = getContentResolver().openOutputStream(uri, "w");
            if (out == null) {
                out = getContentResolver().openOutputStream(uri, "wt");
            }
            if (out == null) {
                return "ERROR: Android document provider did not open an output stream";
            }
            try (InputStream in = new FileInputStream(srcFile); OutputStream target = out) {
                byte[] buffer = new byte[64 * 1024];
                int count;
                long total = 0;
                while ((count = in.read(buffer)) != -1) {
                    target.write(buffer, 0, count);
                    total += count;
                }
                target.flush();
                Log.i(TAG, "Committed " + total + " bytes from " + path + " to " + uri);
            }
            return "OK";
        } catch (Exception e) {
            Log.e(TAG, "Could not write a selected document", e);
            return "ERROR: Could not write the selected document: " + e.getMessage();
        }
    }

    private static String stagedStem(String path) {
        File file = new File(path);
        String name = file.getName();
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        File parent = file.getParentFile();
        return parent == null ? stem : new File(parent, stem).getAbsolutePath();
    }

    private String uriForSibling(String path) {
        String stem = stagedStem(path);
        try {
            for (Map.Entry<String, ?> entry : getSharedPreferences(PREFS, MODE_PRIVATE).getAll().entrySet()) {
                if (!entry.getKey().startsWith("uri:") || !(entry.getValue() instanceof String uriText)) continue;
                String staged = entry.getKey().substring("uri:".length());
                if (stagedStem(staged).equals(stem)) return uriText;
            }
        } catch (Exception e) {
            Log.w(TAG, "Could not look up a staged document", e);
        }
        return null;
    }

    private static String keepSuggestedExtension(String name, String suggested) {
        if (name == null || suggested == null) return name;
        int dot = name.lastIndexOf('.');
        int slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'));
        if (dot > slash + 1 && dot < name.length() - 1) return name;
        int suggestedDot = suggested.lastIndexOf('.');
        if (suggestedDot <= 0 || suggestedDot >= suggested.length() - 1) return name;
        return name + suggested.substring(suggestedDot);
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
        if (!dir.mkdirs() && !dir.isDirectory()) {
            throw new Exception("Could not create the media directory");
        }
        String safe = safeName(name == null ? "imported-file" : name);
        String stem = safe, extension = "";
        int dot = safe.lastIndexOf('.');
        if (dot > 0) {
            stem = safe.substring(0, dot);
            extension = safe.substring(dot);
        }
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        String owner = uri == null ? null : uri.toString();
        for (int attempt = 1; attempt <= 10000; attempt++) {
            String candidate = attempt == 1 ? safe : stem + " (" + attempt + ")" + extension;
            File file = new File(dir, candidate);
            String mapped = prefs.getString("uri:" + file.getAbsolutePath(), null);
            if (mapped == null || mapped.equals(owner) || !file.exists()) {
                return file;
            }
        }
        throw new Exception("Could not find a free name for the selected document in " + dir);
    }

    private static String safeName(String name) {
        String clean = name.replace('\\', '/');
        int slash = clean.lastIndexOf('/');
        if (slash >= 0) {
            clean = clean.substring(slash + 1);
        }
        clean = clean.replaceAll("[^A-Za-z0-9._() -]", "_").trim();
        if (clean.isEmpty() || clean.equals(".") || clean.equals("..")) {
            return "untitled";
        }
        return clean;
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (index >= 0) {
                    return safeName(cursor.getString(index));
                }
            }
        } catch (Exception ignored) {
        }
        String segment = uri.getLastPathSegment();
        return segment == null ? "imported-file" : safeName(segment);
    }

    private void persistPermission(Uri uri, Intent data) {
        try {
            int flags = data.getFlags()
                    & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            if (flags != 0) {
                getContentResolver().takePersistableUriPermission(uri, flags);
            }
        } catch (Exception e) {
            Log.i(TAG, "Provider did not offer a persistable grant for " + uri, e);
        }
    }

    private void rememberUri(File path, Uri uri) {
        getSharedPreferences(PREFS, MODE_PRIVATE)
                .edit()
                .putString("uri:" + path.getAbsolutePath(), uri.toString())
                .commit();
    }

    private void copyUriToFile(Uri uri, File destination) throws Exception {
        InputStream input = getContentResolver().openInputStream(uri);
        if (input == null) {
            throw new Exception("Android document provider did not open the selected file");
        }
        try (InputStream in = input; FileOutputStream out = new FileOutputStream(destination)) {
            byte[] buffer = new byte[64 * 1024];
            int count;
            while ((count = in.read(buffer)) != -1) {
                out.write(buffer, 0, count);
            }
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
        if (intent == null) {
            return;
        }
        List<Uri> uris = collectIncomingUris(intent);
        if (uris.isEmpty()) {
            return;
        }
        for (Uri uri : uris) {
            persistPermission(uri, intent);
        }
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
        try {
            results.offer(base(id, kind, "cancel").toString());
        } catch (Exception e) {
            Log.e(TAG, "Could not encode cancellation", e);
        }
    }

    private void enqueueError(long id, String kind, String message) {
        try {
            JSONObject value = base(id, kind, "error");
            value.put("message", message == null ? "Unknown Android file picker error" : message);
            results.offer(value.toString());
        } catch (Exception e) {
            Log.e(TAG, "Could not encode picker error", e);
        }
    }

    private void enqueuePaths(long id, String kind, String[] paths) {
        try {
            JSONArray array = new JSONArray();
            for (String path : paths) {
                array.put(path);
            }
            enqueuePathsArray(id, kind, array);
        } catch (Exception e) {
            if (id >= 0) {
                enqueueError(id, kind, e.toString());
            } else {
                Log.e(TAG, "Could not encode picker paths", e);
            }
        }
    }

    private void enqueuePathsArray(long id, String kind, JSONArray paths) {
        try {
            JSONObject value = base(id, kind, "ok");
            value.put("paths", paths);
            results.offer(value.toString());
        } catch (Exception e) {
            Log.e(TAG, "Could not encode picker paths", e);
        }
    }

    private static String mimeFor(String fileName) {
        String lower = fileName.toLowerCase(Locale.ROOT);
        if (lower.endsWith(".svg") || lower.endsWith(".svgz")) return "image/svg+xml";
        if (lower.endsWith(".pdf")) return "application/pdf";
        if (lower.endsWith(".png")) return "image/png";
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
        if (lower.endsWith(".webp")) return "image/webp";
        if (lower.endsWith(".gif")) return "image/gif";
        if (lower.endsWith(".tif") || lower.endsWith(".tiff")) return "image/tiff";
        if (lower.endsWith(".bmp")) return "image/bmp";
        return "application/octet-stream";
    }

    public void setSurfacePointerIcon(final int pointerIconType) {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.N) return;
        runOnUiThread(() -> {
            try {
                android.view.Window window = getWindow();
                if (window == null) return;
                android.view.View decor = window.getDecorView();
                if (decor == null) return;
                android.view.PointerIcon icon =
                        android.view.PointerIcon.getSystemIcon(this, pointerIconType);
                applyPointerIconRecursive(decor, icon);
            } catch (Throwable error) {
                android.util.Log.w(TAG, "Could not set surface pointer icon", error);
            }
        });
    }

    private static void applyPointerIconRecursive(android.view.View root, android.view.PointerIcon icon) {
        if (root == null || android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.N) {
            return;
        }
        root.setPointerIcon(icon);
        if (root instanceof android.view.ViewGroup) {
            android.view.ViewGroup group = (android.view.ViewGroup) root;
            for (int i = 0; i < group.getChildCount(); i++) {
                applyPointerIconRecursive(group.getChildAt(i), icon);
            }
        }
    }

}
