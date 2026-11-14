package com.artcraft.launcher;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ActivityNotFoundException;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.util.Pair;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;
import android.view.View;
import android.view.Window;
import android.widget.ProgressBar;
import android.widget.Toast;

import androidx.core.app.NotificationCompat;
import androidx.core.content.ContextCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.regex.Matcher;
import java.util.regex.Pattern;



/**
 * ArtCraft Launcher's Activity: GameActivity runs the Rust app (libartcraft_launcher_android.so);
 * this class adds what winit doesn't offer on Android.
 *
 * <ul>
 *   <li><b>Bridge</b>: Storage Access Framework pickers, driven by small JSON files in
 *       {@code files/bridge} (see {@code src/bridge.rs} for the protocol).
 *   <li><b>Install/Uninstall</b>: APK downloads and PackageInstaller sessions.
 *   <li><b>Updates</b>: Self-update from GitHub releases.
 * </ul>
 */
public class ArtCraftLauncherActivity extends GameActivity {
    private static final String TAG = "ArtCraftLauncher";
    /** Guards answer.json writes from this Activity and from {@link Updater}'s receiver. */
    private static final Object ANSWER_LOCK = new Object();
    private static final int REQ_OPEN = 1001;
    private static final int REQ_SAVE = 1002;
    private static final int REQ_NOTIFICATIONS = 1003;
    private static final int REQ_UNINSTALL = 1004;
    private static final long POLL_MS = 150;
    private static final String CHANNEL_DOWNLOADS = "downloads";
    private static final String CHANNEL_UPDATES = "updates";

    static {
        // GameActivity loads the library too (android.app.lib_name); loading it here first makes
        // native functions resolvable from the first touch.
        System.loadLibrary("artcraft_launcher_android");
    }

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private File bridge;
    private Updater updater;
    /** The picker request being answered (-1: none), kept across Activity recreation. */
    private long pendingId = -1;
    private String pendingName = "";
    /** The uninstall request being answered (-1: none), kept across Activity recreation. */
    private long pendingUninstallId = -1;
    private String pendingUninstallPackage = "";
    /** Soft-keyboard text, deletions and enters waiting for the Rust UI to drain. */
    private final ConcurrentLinkedQueue<String> imeQueue = new ConcurrentLinkedQueue<>();
    /** The last keyboard state asked for by Rust (`bridge/keyboard`), so it acts on changes only. */
    private String keyboardState = "";
    /** The last system theme (light/dark) to sync with Rust (`bridge/theme`). */
    private String themeState = "";
    /** The invisible field the soft keyboard types into. */
    private ArtCraftInputView inputView;

    private final Runnable poll = new Runnable() {
        @Override
        public void run() {
            try {
                checkRequest();
                checkKeyboard();
                checkTheme();
                checkSync();
                checkQuit();
                checkUpdateRequest();
                checkClipboard();
                checkNotifications();
            } catch (Exception e) {
                Log.e(TAG, "bridge", e);
            }
            handler.postDelayed(this, POLL_MS);
        }
    };

    @Override
    protected void onCreate(android.os.Bundle state) {
        super.onCreate(state);

        bridge = new File(getFilesDir(), "bridge");
        updater = new Updater(this);
        createNotificationChannels();
        requestNotificationPermission();
        if (state == null && updater.checkAtStartup()) {
            // A moment after launch, so the first frames aren't competing with it.
            handler.postDelayed(() -> updater.check(false), 4000);
        }
        if (state != null) {
            pendingId = state.getLong("pendingId", -1);
            pendingName = state.getString("pendingName", "");
            pendingUninstallId = state.getLong("pendingUninstallId", -1);
            pendingUninstallPackage = state.getString("pendingUninstallPackage", "");
        }
        hideSystemBars();
        // The soft keyboard needs a real view to type into; winit's Android backend never
        // delivers IME text, so the Rust UI drains this queue every frame.
        inputView = ArtCraftInputView.install(this, imeQueue);
        // Draw under a front camera's cutout on every Android version
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) {
            android.view.WindowManager.LayoutParams lp = getWindow().getAttributes();
            lp.layoutInDisplayCutoutMode = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R
                    ? android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                    : android.view.WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            getWindow().setAttributes(lp);
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        hideSystemBars();
        // Re-read the keyboard state after coming back: Rust keeps asking while a text field has
        // focus, and the IME should reappear with the Activity.
        keyboardState = "?";
        // Also re-check theme in case it changed while we were in background.
        themeState = "?";
        handler.removeCallbacks(poll);
        handler.post(poll);
        updater.resumePendingInstalls();
    }

    @Override
    public void onConfigurationChanged(android.content.res.Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        // System theme (light/dark) changed: tell Rust to re-apply.
        themeState = "?";
        bumpThemeGeneration();
        handler.removeCallbacks(poll);
        handler.post(poll);
    }

    @Override
    protected void onPause() {
        handler.removeCallbacks(poll);
        super.onPause();
    }

    /**
     * Back sends the launcher to the background rather than finishing it.
     */
    @Override
    public void onBackPressed() {
        moveTaskToBack(true);
    }

    /**
     * The keyboard was put away: tell the Rust side.
     */
    @Override
    public void onSoftwareKeyboardVisibilityChanged(boolean visible) {
        super.onSoftwareKeyboardVisibilityChanged(visible);
        Log.d(TAG, "onSoftwareKeyboardVisibilityChanged: visible=" + visible);
    }

    /**
     * Once the Activity is finished, end the process.
     */
    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            checkSync();
            io.shutdown();
            try {
                if (!io.awaitTermination(20, java.util.concurrent.TimeUnit.SECONDS)) {
                    Log.w(TAG, "saves still copying after 20 s; ending anyway");
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
    protected void onSaveInstanceState(android.os.Bundle out) {
        super.onSaveInstanceState(out);
        out.putLong("pendingId", pendingId);
        out.putString("pendingName", pendingName);
        out.putLong("pendingUninstallId", pendingUninstallId);
        out.putString("pendingUninstallPackage", pendingUninstallPackage);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
    }

    private void hideSystemBars() {
        androidx.core.view.WindowInsetsControllerCompat c = androidx.core.view.WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        c.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars());
        c.setSystemBarsBehavior(androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    /**
     * Called by the Rust shell once the frame's resolved cursor icon changes (see the vendored
     * egui-winit's `pointer_icon`). Walks the whole view tree, since the pointer hover state
     * only reaches the deepest focused view.
     */
    public void setSurfacePointerIcon(final int pointerIconType) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.N) return;
        runOnUiThread(() -> {
            try {
                Window window = getWindow();
                if (window == null) return;
                View decor = window.getDecorView();
                if (decor == null) return;
                android.view.PointerIcon icon =
                        android.view.PointerIcon.getSystemIcon(this, pointerIconType);
                applyPointerIconRecursive(decor, icon);
            } catch (Throwable error) {
                Log.w(TAG, "Could not set surface pointer icon", error);
            }
        });
    }

    private static void applyPointerIconRecursive(View root, android.view.PointerIcon icon) {
        if (root == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.N) {
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

    // ---- Bridge request/response -------------------------------------------------------------

    private void checkRequest() throws Exception {
        File f = new File(bridge, "request.json");
        if (pendingId != -1 || pendingUninstallId != -1 || !f.isFile()) {
            return;
        }
        Log.d(TAG, "checkRequest: processing request.json");
        String content = readText(f);
        Log.d(TAG, "checkRequest: request content: " + content);
        JSONObject r = new JSONObject(content);
        //noinspection ResultOfMethodCallIgnored
        f.delete();
        long id = r.getLong("id");
        String kind = r.getString("kind");
        Log.d(TAG, "checkRequest: id=" + id + ", kind=" + kind);
        Intent intent;
        int code;
        if ("open".equals(kind)) {
            intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, r.optBoolean("multiple", false));
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            code = REQ_OPEN;
        } else if ("save".equals(kind)) {
            pendingName = r.optString("name", "Untitled.png");
            intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType(r.optString("mime", "application/octet-stream"))
                    .putExtra(Intent.EXTRA_TITLE, pendingName);
            code = REQ_SAVE;
        } else if ("get_installed_versions".equals(kind)) {
            JSONArray packages = r.optJSONArray("packages");
            JSONObject versions = new JSONObject();
            for (int i = 0; packages != null && i < packages.length(); i++) {
                String pkg = packages.optString(i, "");
                String version = getInstalledVersion(pkg);
                versions.put(pkg, version != null ? version : JSONObject.NULL);
            }
            try {
                answer(new JSONObject().put("id", id).put("versions", versions));
            } catch (Exception e) {
                Log.e(TAG, "get_installed_versions answer", e);
            }
            return;
        } else if ("uninstall".equals(kind)) {
            String pkg = r.optString("package", "");
            pendingUninstallId = id;
            pendingUninstallPackage = pkg;
            Intent uninstall = new Intent(Intent.ACTION_DELETE, Uri.parse("package:" + pkg));
            try {
                // The answer is written once the user has decided in the system prompt.
                startActivityForResult(uninstall, REQ_UNINSTALL);
            } catch (ActivityNotFoundException e) {
                Log.e(TAG, "uninstall " + pkg, e);
                pendingUninstallId = -1;
                pendingUninstallPackage = "";
                try {
                    answer(new JSONObject().put("id", id).put("success", false));
                } catch (Exception answerError) {
                    Log.e(TAG, "uninstall answer", answerError);
                }
            }
            return;
        } else if ("open_app".equals(kind)) {
            String pkg = r.optString("package", "");
            boolean ok = openApp(pkg);
            try {
                answer(new JSONObject().put("id", id).put("success", ok));
            } catch (Exception e) {
                Log.e(TAG, "open_app answer", e);
            }
            return;
        } else if ("move_to_background".equals(kind)) {
            moveTaskToBack(true);
            try {
                answer(new JSONObject().put("id", id).put("success", true));
            } catch (Exception e) {
                Log.e(TAG, "move_to_background answer", e);
            }
            return;
        } else if ("install_apk".equals(kind)) {
            String pkg = r.optString("package", "");
            String apkPath = r.optString("apk_path", "");
            // The answer is written once the system installer reports the final status.
            updater.installApkFile(pkg, apkPath, id);
            return;
        } else if ("app_info".equals(kind)) {
            String pkg = r.optString("package", "");
            try {
                Intent info = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:" + pkg));
                info.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(info);
                answer(new JSONObject().put("id", id).put("success", true));
            } catch (Exception e) {
                Log.e(TAG, "app_info " + pkg, e);
                answer(new JSONObject().put("id", id).put("success", false));
            }
            return;
        } else {
            answerError(id, "unknown request " + kind);
            return;
        }
        pendingId = id;
        try {
            startActivityForResult(intent, code);
        } catch (ActivityNotFoundException e) {
            pendingId = -1;
            answerError(id, "no document picker on this device");
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_UNINSTALL) {
            final long uninstallId = pendingUninstallId;
            final String uninstallPackage = pendingUninstallPackage;
            pendingUninstallId = -1;
            pendingUninstallPackage = "";
            if (uninstallId == -1) {
                return;
            }
            // The system installer returned whatever the user did; the package's presence is the
            // real answer, so re-check it instead of trusting resultCode.
            boolean removed = uninstallPackage.isEmpty() || getInstalledVersion(uninstallPackage) == null;
            try {
                answer(new JSONObject().put("id", uninstallId).put("success", removed));
            } catch (Exception e) {
                Log.e(TAG, "uninstall answer", e);
            }
            return;
        }
        if (requestCode != REQ_OPEN && requestCode != REQ_SAVE) {
            return;
        }
        final long id = pendingId;
        pendingId = -1;
        if (id == -1) {
            return;
        }
        if (resultCode != Activity.RESULT_OK || data == null) {
            answerCancelled(id);
            return;
        }
        if (requestCode == REQ_OPEN) {
            final java.util.List<Uri> uris = new java.util.ArrayList<>();
            android.content.ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    uris.add(clip.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
            final int flags = data.getFlags();
            io.execute(() -> copyIn(id, uris, flags));
        } else {
            final Uri uri = data.getData();
            if (uri == null) {
                answerCancelled(id);
                return;
            }
            String provided = displayName(uri, pendingName);
            String name = savedName(provided, pendingName);
            Log.i(TAG, "save: asked for \"" + pendingName + "\", the provider named it \"" + provided + "\"; writing \"" + name + "\"");
            keepAccess(uri, Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            File dest = new File(bridge, "out/" + id + "/" + safeName(name));
            //noinspection ResultOfMethodCallIgnored
            dest.getParentFile().mkdirs();
            remember(dest, uri);
            try {
                answer(new JSONObject().put("id", id).put("path", dest.getAbsolutePath()));
            } catch (Exception e) {
                Log.e(TAG, "save answer", e);
            }
        }
    }

    /** Copy picked documents into the bridge inbox and answer with their paths. */
    private void copyIn(long id, java.util.List<Uri> uris, int flags) {
        JSONArray paths = new JSONArray();
        String error = null;
        for (Uri uri : uris) {
            boolean writable = keepAccess(uri, flags);
            File dest = new File(bridge, "inbox/" + id + "/" + safeName(nameWithExtension(uri)));
            //noinspection ResultOfMethodCallIgnored
            dest.getParentFile().mkdirs();
            try (InputStream in = getContentResolver().openInputStream(uri);
                 OutputStream out = new FileOutputStream(dest)) {
                if (in == null) {
                    throw new IOException("the provider returned no data");
                }
                copy(in, out);
                paths.put(dest.getAbsolutePath());
                if (writable) {
                    remember(dest, uri);
                } else {
                    forget(dest);
                }
            } catch (Exception e) {
                Log.e(TAG, "open " + uri, e);
                error = e.getMessage();
            }
        }
        try {
            JSONObject a = new JSONObject().put("id", id).put("paths", paths);
            if (error != null) {
                a.put("error", error);
            }
            answer(a);
        } catch (Exception e) {
            Log.e(TAG, "open answer", e);
        }
    }

    /** Keep access to an opened document across restarts; true when it may be written back. */
    private boolean keepAccess(Uri uri, int flags) {
        boolean write = (flags & Intent.FLAG_GRANT_WRITE_URI_PERMISSION) != 0;
        try {
            getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION
                    | (write ? Intent.FLAG_GRANT_WRITE_URI_PERMISSION : 0));
        } catch (SecurityException e) {
            // Not persistable: the grant lasts while the app runs, which is enough to save back.
        }
        return write;
    }

    // ---- Leaving -----------------------------------------------------------------------------

    /** File > Exit (bridge `quit`): finish once pending saves are queued (see onDestroy). */
    private void checkQuit() {
        File f = new File(bridge, "quit");
        if (!f.exists()) {
            return;
        }
        //noinspection ResultOfMethodCallIgnored
        f.delete();
        checkSync();
        finish();
    }

    /** The system clipboard for text fields. */
    private void checkClipboard() throws IOException {
        android.content.ClipboardManager cm = (android.content.ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (cm == null) {
            return;
        }
        File set = new File(bridge, "clip-set.txt");
        if (set.isFile()) {
            String text = readText(set);
            //noinspection ResultOfMethodCallIgnored
            set.delete();
            cm.setPrimaryClip(android.content.ClipData.newPlainText("ArtCraftLauncher", text));
        }
        File get = new File(bridge, "clip-get");
        if (get.exists() && get.delete()) {
            String text = "";
            android.content.ClipData clip = cm.getPrimaryClip();
            if (clip != null && clip.getItemCount() > 0) {
                CharSequence t = clip.getItemAt(0).coerceToText(this);
                if (t != null) {
                    text = t.toString();
                }
            }
            File tmp = new File(bridge, "clip.txt.tmp");
            try (OutputStream out = new FileOutputStream(tmp)) {
                out.write(text.getBytes(StandardCharsets.UTF_8));
            }
            if (!tmp.renameTo(new File(bridge, "clip.txt"))) {
                Log.w(TAG, "clipboard: rename failed");
            }
        }
    }

    /** Help > Check for Updates... (bridge `check-update`). */
    private void checkUpdateRequest() {
        File f = new File(bridge, "check-update");
        if (f.exists() && f.delete()) {
            updater.check(true);
        }
    }

    /**
     * Show or hide the soft keyboard as Rust asks (a text field gained or lost focus). The file is
     * written atomically, so a missing or unreadable file means "no change".
     */
    private void checkKeyboard() {
        File f = new File(bridge, "keyboard");
        if (!f.isFile()) {
            return;
        }
        String want;
        try {
            want = readText(f).trim();
        } catch (IOException e) {
            return;
        }
        if (want.equals(keyboardState)) {
            return;
        }
        keyboardState = want;
        if (inputView == null) {
            return;
        }
        if ("show".equals(want)) {
            inputView.showKeyboard();
        } else if ("hide".equals(want)) {
            inputView.hideKeyboard();
        }
    }

    private void checkTheme() {
        File f = new File(bridge, "theme");
        if (!f.isFile()) {
            return;
        }
        String want;
        try {
            want = readText(f).trim();
        } catch (IOException e) {
            return;
        }
        if (want.equals(themeState)) {
            return;
        }
        themeState = want;
        // Only the transient system bars (visible while swiped in) follow the UI; the app
        // itself is themed by egui from `RawInput::system_theme`. Never call
        // UiModeManager.setNightMode here: that changes Android's own light/dark setting.
        boolean dark = "dark".equals(want);
        WindowInsetsControllerCompat bars = WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        bars.setAppearanceLightStatusBars(!dark);
        bars.setAppearanceLightNavigationBars(!dark);
    }

    /**
     * Bumps a generation file the Rust worker polls. Android delivers a configuration change
     * (system light/dark, split-screen resize) here without a native input event; the worker
     * wakes egui, whose next frame re-reads the Activity and updates the theme or layout.
     */
    private void bumpThemeGeneration() {
        if (bridge == null) {
            return;
        }
        File tmp = new File(bridge, "system_theme.tmp");
        File destination = new File(bridge, "system_theme");
        try (FileOutputStream out = new FileOutputStream(tmp)) {
            out.write((System.currentTimeMillis() + ":" + getResources().getConfiguration().uiMode)
                    .getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            Log.w(TAG, "couldn't write the theme generation", e);
            return;
        }
        if (!tmp.renameTo(destination)) {
            Log.w(TAG, "couldn't publish the theme generation");
        }
    }

    // ---- Notifications -----------------------------------------------------------------------

    private void createNotificationChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return;
        }
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm == null) {
            return;
        }
        NotificationChannel downloads = new NotificationChannel(
                CHANNEL_DOWNLOADS, "Downloads", NotificationManager.IMPORTANCE_LOW);
        downloads.setDescription("App downloads and installs in progress");
        downloads.setShowBadge(false);
        nm.createNotificationChannel(downloads);
        NotificationChannel updates = new NotificationChannel(
                CHANNEL_UPDATES, "App updates", NotificationManager.IMPORTANCE_DEFAULT);
        updates.setDescription("New versions of installed apps");
        nm.createNotificationChannel(updates);
    }

    private void requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, REQ_NOTIFICATIONS);
        }
    }

    /**
     * Posts the notification files Rust wrote to {@code bridge/notifications}. A repeated id
     * updates the same notification (download progress); a {@code cancel} entry removes it.
     */
    private void checkNotifications() {
        File dir = new File(bridge, "notifications");
        File[] files = dir.listFiles((d, name) -> name.endsWith(".json"));
        if (files == null) {
            return;
        }
        NotificationManager nm = (NotificationManager) getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm == null) {
            return;
        }
        for (File f : files) {
            try {
                JSONObject n = new JSONObject(readText(f));
                //noinspection ResultOfMethodCallIgnored
                f.delete();
                String id = n.optString("id", f.getName());
                int notificationId = id.hashCode();
                if (n.optBoolean("cancel", false)) {
                    nm.cancel(notificationId);
                    continue;
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                        && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    // The permission prompt is up (or was dismissed); the next download or
                    // update will be shown once notifications are allowed.
                    continue;
                }
                String channel = n.optString("channel", CHANNEL_DOWNLOADS);
                String text = n.optString("text", "");
                boolean ongoing = n.optBoolean("ongoing", false);
                NotificationCompat.Builder builder = new NotificationCompat.Builder(this, channel)
                        .setSmallIcon(R.drawable.ic_notification)
                        .setContentTitle(n.optString("title", "ArtCraft Launcher"))
                        .setContentText(text)
                        .setStyle(new NotificationCompat.BigTextStyle().bigText(text))
                        .setContentIntent(launcherPendingIntent())
                        .setOnlyAlertOnce(true)
                        .setAutoCancel(n.optBoolean("auto_cancel", true))
                        .setOngoing(ongoing)
                        .setCategory(CHANNEL_DOWNLOADS.equals(channel)
                                ? NotificationCompat.CATEGORY_PROGRESS
                                : NotificationCompat.CATEGORY_RECOMMENDATION)
                        .setPriority(CHANNEL_DOWNLOADS.equals(channel)
                                ? NotificationCompat.PRIORITY_LOW
                                : NotificationCompat.PRIORITY_DEFAULT);
                if (n.has("progress")) {
                    int progress = Math.max(0, Math.min(100, n.optInt("progress", 0)));
                    builder.setProgress(100, progress, false);
                    builder.setSubText(progress + "%");
                }
                nm.notify(notificationId, builder.build());
            } catch (Exception e) {
                Log.e(TAG, "notification " + f, e);
                //noinspection ResultOfMethodCallIgnored
                f.delete();
            }
        }
    }

    private PendingIntent launcherPendingIntent() {
        Intent intent = new Intent(this, ArtCraftLauncherActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        return PendingIntent.getActivity(this, 0, intent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
    }

    // ---- Writing back ------------------------------------------------------------------------

    private void checkSync() {
        File[] files = new File(bridge, "sync").listFiles((dir, name) -> name.endsWith(".json"));
        if (files == null) {
            return;
        }
        for (File f : files) {
            try {
                String path = new JSONObject(readText(f)).getString("path");
                //noinspection ResultOfMethodCallIgnored
                f.delete();
                final File local = new File(path);
                final String uri = documents().getString(local.getAbsolutePath(), null);
                if (uri == null) {
                    toast("Saved inside ArtCraft Launcher only: this document can't be written back. Use File > Save As to choose where to save.");
                    continue;
                }
                io.execute(() -> copyOut(local, Uri.parse(uri)));
            } catch (Exception e) {
                Log.e(TAG, "sync " + f, e);
                //noinspection ResultOfMethodCallIgnored
                f.delete();
            }
        }
    }

    private void copyOut(File local, Uri uri) {
        try (InputStream in = new FileInputStream(local);
             OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
            if (out == null) {
                throw new IOException("the provider refused the write");
            }
            copy(in, out);
            Log.i(TAG, "saved " + local.getName() + " to " + uri);
        } catch (Exception e) {
            Log.e(TAG, "save " + uri, e);
            toast("Couldn't save " + local.getName() + ": " + e.getMessage());
        }
    }

    private SharedPreferences documents() {
        return getSharedPreferences("documents", MODE_PRIVATE);
    }

    private void remember(File local, Uri uri) {
        documents().edit().putString(local.getAbsolutePath(), uri.toString()).apply();
    }

    private void forget(File local) {
        documents().edit().remove(local.getAbsolutePath()).apply();
    }

    // ---- Helpers -----------------------------------------------------------------------------

    private String getInstalledVersion(String packageName) {
        try {
            PackageManager pm = getPackageManager();
            android.content.pm.PackageInfo info = pm.getPackageInfo(packageName, 0);
            return info.versionName;
        } catch (Exception e) {
            return null;
        }
    }

    private boolean openApp(String packageName) {
        try {
            PackageManager pm = getPackageManager();
            Intent intent = pm.getLaunchIntentForPackage(packageName);
            if (intent != null) {
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(intent);
                return true;
            }
            return false;
        } catch (Exception e) {
            Log.e(TAG, "open " + packageName, e);
            return false;
        }
    }

    /**
     * Called from Rust once per frame (see {@code egui-winit}'s Android IME bridge): the next
     * piece of soft-keyboard text, or {@code null} when the queue is empty.
     */
    public String pollIme() {
        return imeQueue.poll();
    }

    private void answerCancelled(long id) {
        try {
            answer(new JSONObject().put("id", id).put("cancelled", true));
        } catch (Exception e) {
            Log.e(TAG, "answer", e);
        }
    }

    private void answerError(long id, String error) {
        try {
            answer(new JSONObject().put("id", id).put("cancelled", true).put("error", error));
        } catch (Exception e) {
            Log.e(TAG, "answer", e);
        }
    }

    /** Write answer.json under a temporary name, then rename, so Rust never reads half of it. */
    private void answer(JSONObject a) throws IOException {
        writeAnswer(bridge, a);
    }

    /** The same, callable from {@link Updater} once the system installer reports its result. */
    static void writeAnswer(File bridgeDir, JSONObject a) throws IOException {
        synchronized (ANSWER_LOCK) {
            Log.d(TAG, "answer: writing response: " + a);
            File tmp = new File(bridgeDir, "answer.json.tmp");
            try (OutputStream out = new FileOutputStream(tmp)) {
                out.write(a.toString().getBytes(StandardCharsets.UTF_8));
            }
            if (!tmp.renameTo(new File(bridgeDir, "answer.json"))) {
                throw new IOException("couldn't publish the answer");
            }
        }
    }

    private String displayName(Uri uri, String fallback) {
        try (android.database.Cursor c = getContentResolver().query(uri, new String[]{android.provider.OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) {
                return c.getString(0);
            }
        } catch (Exception e) {
            Log.w(TAG, "display name", e);
        }
        String last = uri.getLastPathSegment();
        return last != null ? last : fallback;
    }

    private String nameWithExtension(Uri uri) {
        String name = displayName(uri, "document");
        if (!extensionOf(name).isEmpty()) {
            return name;
        }
        String type = null;
        try {
            type = getContentResolver().getType(uri);
        } catch (Exception e) {
            Log.w(TAG, "type", e);
        }
        String ext = type == null ? null : android.webkit.MimeTypeMap.getSingleton().getExtensionFromMimeType(type);
        if (ext == null && "image/vnd.adobe.photoshop".equals(type)) {
            ext = "psd";
        }
        Log.i(TAG, "open: \"" + name + "\" has no extension; type " + type + " -> " + ext);
        return ext == null ? name : name + "." + ext;
    }

    static String savedName(String provided, String requested) {
        String p = provided == null ? "" : provided.trim();
        boolean id = p.isEmpty() || p.matches("\\d+") || p.matches("[A-Za-z]+:\\d+");
        if (id) {
            return requested;
        }
        if (!extensionOf(p).isEmpty()) {
            return p;
        }
        String ext = extensionOf(requested);
        return ext.isEmpty() ? p : p + "." + ext;
    }

    private static String extensionOf(String name) {
        int dot = name.lastIndexOf('.');
        return dot > 0 && dot < name.length() - 1 ? name.substring(dot + 1).toLowerCase(java.util.Locale.ROOT) : "";
    }

    static String safeName(String name) {
        String s = name.replaceAll("[\\\\/:*?\"<>|\\p{Cntrl}]", "_").trim();
        while (s.startsWith(".")) {
            s = s.substring(1);
        }
        return s.isEmpty() ? "document" : s;
    }

    private static String readText(File f) throws IOException {
        try (InputStream in = new FileInputStream(f)) {
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            copy(in, out);
            return out.toString("UTF-8");
        }
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[1 << 16];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
    }

    private void toast(String text) {
        runOnUiThread(() -> Toast.makeText(this, text, Toast.LENGTH_LONG).show());
    }
}