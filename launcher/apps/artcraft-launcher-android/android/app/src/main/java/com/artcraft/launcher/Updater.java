package com.artcraft.launcher;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;
import android.widget.ProgressBar;
import android.widget.Toast;

import androidx.core.content.ContextCompat;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Updates and installs from GitHub releases.
 *
 * <p>Two update paths:
 * <ul>
 *   <li><b>Self-update</b>: the launcher itself (from storytold/craft-launcher releases tagged
 *       android-v...).
 *   <li><b>App installs</b>: Craft apps (from MostafaXS1/artcraft-android and per-app repos).
 * </ul>
 */
final class Updater {
    private static final String TAG = "ArtCraftLauncher";
    /** The repository for launcher self-updates. */
    static final String LAUNCHER_REPO = "storytold/craft-launcher";
    private static final String LAUNCHER_API = "https://api.github.com/repos/" + LAUNCHER_REPO + "/releases?per_page=20";
    private static final Pattern LAUNCHER_TAG_VERSION = Pattern.compile("^android-v(\\d+)\\.(\\d+)\\.(\\d+)(?:-(\\d+))?$");
    private static final Pattern NAME_VERSION = Pattern.compile("^(\\d+)\\.(\\d+)\\.(\\d+)(?:-(\\d+))?(?:-android)?");
    private static final String INSTALL_ACTION = "ai.storyteller.artcraftlauncher.APP_INSTALLED";

    private final Activity activity;
    private final File filesDir;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService net = Executors.newSingleThreadExecutor();
    private boolean busy;
    private boolean receiverRegistered;

    /** A newer release: its tag, title, notes and APK. */
    private static final class Release {
        String tag;
        String title;
        String notes;
        String apkUrl;
        long apkSize;
        int[] version;
    }

    Updater(Activity activity) {
        this.activity = activity;
        this.filesDir = activity.getFilesDir();
    }

    /** Whether the startup check is on (Help > Check for Updates at Startup). */
    boolean checkAtStartup() {
        // Disabled: the Rust backend handles all release checks including launcher updates.
        // This avoids duplicate GitHub API calls that exhaust the unauthenticated rate limit.
        return false;
    }

    /**
     * Look for a newer launcher release. {@code manual}: the user asked, so say so when there is
     * none or the check fails, and offer a skipped version again.
     */
    void check(boolean manual) {
        if (busy) {
            return;
        }
        busy = true;
        if (manual) {
            toast("Checking for launcher updates…");
        }
        net.execute(() -> {
            Release r = null;
            String error = null;
            try {
                r = newestLauncher();
            } catch (Exception e) {
                Log.w(TAG, "launcher update check", e);
                error = e.getMessage();
            }
            final Release found = r;
            final String failed = error;
            main.post(() -> {
                busy = false;
                if (activity.isFinishing()) {
                    return;
                }
                if (failed != null) {
                    if (manual) {
                        toast("Couldn't check for updates: " + failed);
                    }
                } else if (found == null) {
                    if (manual) {
                        toast("ArtCraft Launcher is up to date (" + currentName() + ").");
                    }
                } else if (manual || !found.tag.equals(prefs().getString("skipped", ""))) {
                    offerLauncherUpdate(found);
                }
            });
        });
    }

    // --- Install a Craft app APK ------------------------------------------------------------

    /** Install an app APK from a URL (called via bridge). */
    void installApp(String appId, String apkUrl, String expectedSha256) {
        if (busy) {
            return;
        }
        busy = true;
        toast("Downloading " + appId + "…");
        net.execute(() -> {
            File apk = new File(activity.getCacheDir(), appId + ".apk");
            String error = null;
            try {
                HttpURLConnection c = open(apkUrl);
                long total = c.getContentLengthLong();
                long done = 0;
                try (InputStream in = c.getInputStream(); OutputStream out = new FileOutputStream(apk)) {
                    byte[] buf = new byte[65536];
                    for (int n; (n = in.read(buf)) > 0; ) {
                        out.write(buf, 0, n);
                        done += n;
                        if (total > 0) {
                            // Progress could be sent via bridge
                        }
                    }
                } finally {
                    c.disconnect();
                }
                // TODO: verify SHA256
            } catch (Exception e) {
                Log.w(TAG, "app download " + appId, e);
                error = e.getMessage();
                //noinspection ResultOfMethodCallIgnored
                apk.delete();
            }
            final String failed = error;
            main.post(() -> {
                busy = false;
                if (cancelled != null && cancelled[0]) {
                    return;
                }
                if (failed != null) {
                    toast("Download failed: " + failed);
                    return;
                }
                installApk(apk, appId);
            });
        });
    }

    private void installApk(File apk, String appId) {
        if (!activity.getPackageManager().canRequestPackageInstalls()) {
            prefs().edit().putBoolean("awaiting-permission-" + appId, true).apply();
            toast("Allow ArtCraft Launcher to install apps, then come back to finish.");
            activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + activity.getPackageName())));
            return;
        }
        net.execute(() -> {
            try {
                PackageInstaller installer = activity.getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                // Install as a new package (not update)
                int id = installer.createSession(params);
                try (PackageInstaller.Session session = installer.openSession(id)) {
                    try (InputStream in = new FileInputStream(apk); OutputStream out = session.openWrite(appId + ".apk", 0, apk.length())) {
                        byte[] buf = new byte[65536];
                        for (int n; (n = in.read(buf)) > 0; ) {
                            out.write(buf, 0, n);
                        }
                        session.fsync(out);
                    }
                    main.post(this::registerReceiver);
                    Intent status = new Intent(INSTALL_ACTION).setPackage(activity.getPackageName());
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                    session.commit(PendingIntent.getBroadcast(activity, id, status, flags).getIntentSender());
                }
            } catch (Exception e) {
                Log.w(TAG, "app install " + appId, e);
                main.post(() -> toast("Install failed: " + e.getMessage()));
            }
        });
    }

    void resumePendingInstalls() {
        // The launcher asked to install an app while the "install unknown apps" permission was
        // missing: the user may have granted it while away.
        if (pendingApk != null && activity.getPackageManager().canRequestPackageInstalls()) {
            startAppInstall();
        }
        File[] apks = activity.getCacheDir().listFiles((dir, name) -> name.endsWith(".apk"));
        if (apks == null) return;
        for (File apk : apks) {
            String appId = apk.getName().replace(".apk", "");
            if (prefs().getBoolean("awaiting-permission-" + appId, false)
                    && activity.getPackageManager().canRequestPackageInstalls()) {
                prefs().edit().putBoolean("awaiting-permission-" + appId, false).apply();
                installApk(apk, appId);
            }
        }
    }

    // --- Install an app the launcher downloaded ---------------------------------------------

    private static final String APP_INSTALL_ACTION = "ai.storyteller.artcraftlauncher.APP_INSTALL_STATUS";
    private boolean appReceiverRegistered;
    /** The APK the Rust backend handed over, waiting for the permission or the system prompt. */
    private String pendingApk;
    private String pendingPkg;
    /** The bridge request to answer when the install ends; -1 once answered. */
    private long pendingBridgeId = -1;

    /**
     * Installs a downloaded APK and answers the bridge request {@code bridgeId} when the system
     * installer reports its final status. Without the "install unknown apps" permission the
     * answer says so at once, and the install starts by itself once the user grants it.
     */
    void installApkFile(String packageName, String apkPath, long bridgeId) {
        pendingApk = apkPath;
        pendingPkg = packageName;
        pendingBridgeId = bridgeId;
        if (!activity.getPackageManager().canRequestPackageInstalls()) {
            toast("Allow ArtCraft Launcher to install apps, then come back to finish.");
            answerBridge(false, "permission");
            activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + activity.getPackageName())));
            return;
        }
        startAppInstall();
    }

    /** Starts the PackageInstaller session for the pending APK, if there is one. */
    private void startAppInstall() {
        final String apkPath = pendingApk;
        final String packageName = pendingPkg;
        pendingApk = null;
        pendingPkg = null;
        if (apkPath == null || !new File(apkPath).isFile()) {
            answerBridge(false, "the downloaded file is gone");
            return;
        }
        net.execute(() -> {
            try {
                File apk = new File(apkPath);
                PackageInstaller installer = activity.getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(packageName);
                int id = installer.createSession(params);
                try (PackageInstaller.Session session = installer.openSession(id)) {
                    try (InputStream in = new FileInputStream(apk); OutputStream out = session.openWrite("app.apk", 0, apk.length())) {
                        byte[] buf = new byte[65536];
                        for (int n; (n = in.read(buf)) > 0; ) {
                            out.write(buf, 0, n);
                        }
                        session.fsync(out);
                    }
                    main.post(this::registerAppReceiver);
                    Intent status = new Intent(APP_INSTALL_ACTION).setPackage(activity.getPackageName());
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                    session.commit(PendingIntent.getBroadcast(activity, id, status, flags).getIntentSender());
                }
            } catch (Exception e) {
                Log.w(TAG, "app install " + packageName, e);
                main.post(() -> answerBridge(false, "failed: " + e.getMessage()));
            }
        });
    }

    private void registerAppReceiver() {
        if (appReceiverRegistered) {
            return;
        }
        appReceiverRegistered = true;
        ContextCompat.registerReceiver(activity.getApplicationContext(), new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        context.startActivity(confirm);
                    }
                } else if (status == PackageInstaller.STATUS_SUCCESS) {
                    answerBridge(true, null);
                } else if (status == PackageInstaller.STATUS_FAILURE_ABORTED) {
                    answerBridge(false, "aborted");
                } else {
                    String msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                    answerBridge(false, "failed: " + (msg == null ? "the installer refused it" : msg));
                }
            }
        }, new IntentFilter(APP_INSTALL_ACTION), ContextCompat.RECEIVER_NOT_EXPORTED);
    }

    /** Answers the waiting bridge request, once. */
    private void answerBridge(boolean success, String error) {
        long id = pendingBridgeId;
        pendingBridgeId = -1;
        if (id == -1) {
            return;
        }
        try {
            JSONObject a = new JSONObject().put("id", id).put("success", success);
            if (error != null) {
                a.put("error", error);
            }
            ArtCraftLauncherActivity.writeAnswer(new File(activity.getFilesDir(), "bridge"), a);
        } catch (Exception e) {
            Log.e(TAG, "install answer", e);
        }
    }

    // ---- Finding the launcher release -------------------------------------------------------

    private String currentName() {
        try {
            String n = activity.getPackageManager().getPackageInfo(activity.getPackageName(), 0).versionName;
            return n == null ? "" : n.replace("-android", "");
        } catch (Exception e) {
            return "";
        }
    }

    static int[] parse(Pattern p, String s) {
        Matcher m = p.matcher(s == null ? "" : s);
        if (!m.find()) {
            return null;
        }
        int[] v = new int[4];
        for (int i = 0; i < 4; i++) {
            String g = m.group(i + 1);
            v[i] = g == null ? 0 : Integer.parseInt(g);
        }
        return v;
    }

    static int compare(int[] a, int[] b) {
        for (int i = 0; i < 4; i++) {
            if (a[i] != b[i]) {
                return Integer.compare(a[i], b[i]);
            }
        }
        return 0;
    }

    /** The newest launcher release newer than this build, with an APK; null when there is none. */
    private Release newestLauncher() throws IOException, org.json.JSONException {
        int[] current = parse(NAME_VERSION, currentName());
        if (current == null) {
            throw new IOException("unknown installed version");
        }
        JSONArray releases = new JSONArray(new String(get(LAUNCHER_API), StandardCharsets.UTF_8));
        Release best = null;
        for (int i = 0; i < releases.length(); i++) {
            JSONObject rel = releases.getJSONObject(i);
            if (rel.optBoolean("draft")) {
                continue;
            }
            String tag = rel.optString("tag_name");
            int[] v = parse(LAUNCHER_TAG_VERSION, tag);
            if (v == null || compare(v, current) <= 0 || (best != null && compare(v, best.version) <= 0)) {
                continue;
            }
            JSONArray assets = rel.optJSONArray("assets");
            for (int j = 0; assets != null && j < assets.length(); j++) {
                JSONObject a = assets.getJSONObject(j);
                if (a.optString("name").endsWith(".apk")) {
                    Release r = new Release();
                    r.tag = tag;
                    r.title = rel.optString("name", tag);
                    r.notes = rel.optString("body", "");
                    r.apkUrl = a.getString("browser_download_url");
                    r.apkSize = a.optLong("size", -1);
                    r.version = v;
                    best = r;
                    break;
                }
            }
        }
        return best;
    }

    private HttpURLConnection open(String url) throws IOException {
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(15000);
        c.setReadTimeout(30000);
        c.setRequestProperty("User-Agent", "ArtCraft-Launcher/" + currentName());
        c.setRequestProperty("Accept", url.startsWith("https://api.github.com/") ? "application/vnd.github+json" : "application/octet-stream");
        c.setInstanceFollowRedirects(true);
        int code = c.getResponseCode();
        if (code != 200) {
            c.disconnect();
            throw new IOException("GitHub answered " + code);
        }
        return c;
    }

    private byte[] get(String url) throws IOException {
        HttpURLConnection c = open(url);
        try (InputStream in = c.getInputStream()) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[16384];
            for (int n; (n = in.read(buf)) > 0; ) {
                out.write(buf, 0, n);
            }
            return out.toByteArray();
        } finally {
            c.disconnect();
        }
    }

    // ---- Asking -----------------------------------------------------------------------------

    private void offerLauncherUpdate(Release r) {
        String name = r.tag.replace("android-v", "");
        new AlertDialog.Builder(activity, AlertDialog.THEME_DEVICE_DEFAULT_DARK)
                .setTitle("ArtCraft Launcher " + name + " is available")
                .setMessage("You have " + currentName() + ".\n\n" + summary(r.notes) + "\n\nDownload and install it now? Save your work first: the launcher closes to update.")
                .setPositiveButton("Install", (d, w) -> downloadLauncher(r))
                .setNegativeButton("Later", null)
                .setNeutralButton("Skip This Version", (d, w) -> prefs().edit().putString("skipped", r.tag).apply())
                .show();
    }

    private SharedPreferences prefs() {
        return activity.getSharedPreferences("updates", Context.MODE_PRIVATE);
    }

    /** The release notes' "New in this build" section (or their start), for the dialog. */
    static String summary(String notes) {
        String s = notes.replace("\r", "");
        int at = s.indexOf("## New in this build");
        if (at >= 0) {
            s = s.substring(s.indexOf('\n', at) + 1);
            int end = s.indexOf("\n## ");
            if (end >= 0) {
                s = s.substring(0, end);
            }
        }
        s = s.replaceAll("\\*\\*|`", "").replaceAll("(?m)^- ", "• ").replaceAll("\\n[ \\t]*(?![•\\n])", " ").trim();
        return s.length() > 900 ? s.substring(0, 900) + "…" : s;
    }

    // ---- Downloading and installing launcher ------------------------------------------------

    private boolean[] cancelled = null;

    private void downloadLauncher(Release r) {
        ProgressBar bar = new ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal);
        bar.setMax(1000);
        bar.setPadding(48, 24, 48, 0);
        cancelled = new boolean[]{false};
        AlertDialog dialog = new AlertDialog.Builder(activity, AlertDialog.THEME_DEVICE_DEFAULT_DARK)
                .setTitle("Downloading ArtCraft Launcher " + r.tag.replace("android-v", ""))
                .setView(bar)
                .setCancelable(false)
                .setNegativeButton("Cancel", (d, w) -> cancelled[0] = true)
                .show();
        busy = true;
        net.execute(() -> {
            File apk = new File(activity.getCacheDir(), "launcher-update.apk");
            String error = null;
            try {
                HttpURLConnection c = open(r.apkUrl);
                long total = r.apkSize > 0 ? r.apkSize : c.getContentLengthLong();
                long done = 0;
                try (InputStream in = c.getInputStream(); OutputStream out = new FileOutputStream(apk)) {
                    byte[] buf = new byte[65536];
                    for (int n; (n = in.read(buf)) > 0; ) {
                        if (cancelled[0]) {
                            throw new IOException("cancelled");
                        }
                        out.write(buf, 0, n);
                        done += n;
                        if (total > 0) {
                            final int p = (int) (done * 1000 / total);
                            main.post(() -> bar.setProgress(p));
                        }
                    }
                } finally {
                    c.disconnect();
                }
                if (r.apkSize > 0 && done != r.apkSize) {
                    throw new IOException("the download was incomplete");
                }
            } catch (Exception e) {
                Log.w(TAG, "launcher update download", e);
                error = e.getMessage();
                //noinspection ResultOfMethodCallIgnored
                apk.delete();
            }
            final String failed = error;
            main.post(() -> {
                busy = false;
                dialog.dismiss();
                if (cancelled[0]) {
                    return;
                }
                if (failed != null) {
                    toast("The update couldn't be downloaded: " + failed);
                    return;
                }
                installLauncherApk(apk);
            });
        });
    }

    private void installLauncherApk(File apk) {
        if (!activity.getPackageManager().canRequestPackageInstalls()) {
            prefs().edit().putBoolean("awaiting-permission-launcher", true).apply();
            toast("Allow ArtCraft Launcher to install apps, then come back to finish the update.");
            activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + activity.getPackageName())));
            return;
        }
        net.execute(() -> {
            try {
                PackageInstaller installer = activity.getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(activity.getPackageName());
                int id = installer.createSession(params);
                try (PackageInstaller.Session session = installer.openSession(id)) {
                    try (InputStream in = new FileInputStream(apk); OutputStream out = session.openWrite("launcher.apk", 0, apk.length())) {
                        byte[] buf = new byte[65536];
                        for (int n; (n = in.read(buf)) > 0; ) {
                            out.write(buf, 0, n);
                        }
                        session.fsync(out);
                    }
                    main.post(this::registerReceiver);
                    Intent status = new Intent(INSTALL_ACTION).setPackage(activity.getPackageName());
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                    session.commit(PendingIntent.getBroadcast(activity, id, status, flags).getIntentSender());
                }
            } catch (Exception e) {
                Log.w(TAG, "launcher update install", e);
                main.post(() -> toast("The update couldn't be installed: " + e.getMessage()));
            }
        });
    }

    private void registerReceiver() {
        if (receiverRegistered) {
            return;
        }
        receiverRegistered = true;
        ContextCompat.registerReceiver(activity.getApplicationContext(), new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        context.startActivity(confirm);
                    }
                } else if (status != PackageInstaller.STATUS_SUCCESS) {
                    String msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                    Log.w(TAG, "install: " + status + " " + msg);
                    if (status != PackageInstaller.STATUS_FAILURE_ABORTED) {
                        Toast.makeText(context, status == PackageInstaller.STATUS_FAILURE_CONFLICT || status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE
                                ? "The update is signed with a different key: uninstall and install by hand."
                                : "Install failed" + (msg == null ? "." : ": " + msg), Toast.LENGTH_LONG).show();
                    }
                }
            }
        }, new IntentFilter(INSTALL_ACTION), ContextCompat.RECEIVER_NOT_EXPORTED);
    }

    private void toast(String text) {
        Toast.makeText(activity, text, Toast.LENGTH_LONG).show();
    }
}