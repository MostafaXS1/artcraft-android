package com.artcraft.updater;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * A one-time check for a newer builder release on GitHub.
 *
 * <p>Every APK the builder produces carries the builder's own version. On launch this asks
 * GitHub for the repository's latest release; if that version is newer than the one baked in,
 * the user is told and offered the download. The check is deliberately unobtrusive: it runs on
 * a background thread a few seconds after launch, needs the {@code INTERNET} permission, and
 * never blocks the app. A version that has already been offered is not offered again, so the
 * prompt appears once per release rather than on every launch.
 *
 * <p>It is installed as a shared class next to the app's Activity (the builder copies it into
 * every launcher's Java tree) so the generated launchers and the per-app adapters can all use
 * it without duplicating the code.
 */
public final class UpdateCheck {
    private static final String TAG = "ArtCraftUpdate";

    /** The builder version baked into this APK. */
    private static final String BUILDER_VERSION = "__BUILDER_VERSION__";

    /** GitHub's "latest release" endpoint for the builder's repository. */
    private static final String API_URL = "__UPDATE_API_URL__";

    /** The page a user downloads the newer APK from. */
    private static final String PAGE_URL = "__UPDATE_PAGE_URL__";

    private static final String PREFS = "com.artcraft.updater";
    private static final String KEY_NOTIFIED = "notified_version";

    /** How long after launch the check starts, so it never competes with the app's own startup. */
    private static final long DELAY_MS = 5000L;

    private static final ExecutorService WORKER = Executors.newSingleThreadExecutor();

    private UpdateCheck() {
    }

    /**
     * Start the check for {@code activity}. Safe to call from {@code onCreate}: everything
     * happens off the main thread and the result, if any, is posted back to it.
     */
    public static void start(final Activity activity) {
        if (activity == null) {
            return;
        }
        new Handler(Looper.getMainLooper()).postDelayed(new Runnable() {
            @Override
            public void run() {
                WORKER.execute(new Runnable() {
                    @Override
                    public void run() {
                        check(activity);
                    }
                });
            }
        }, DELAY_MS);
    }

    private static void check(Activity activity) {
        try {
            String latest = fetchLatestVersion();
            if (latest == null || latest.isEmpty()) {
                return;
            }
            if (!isNewer(latest, BUILDER_VERSION)) {
                return;
            }
            if (wasNotified(activity, latest)) {
                return;
            }
            markNotified(activity, latest);
            activity.runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    showDialog(activity, latest);
                }
            });
        } catch (Throwable error) {
            // A failed check must never disturb the app: no network, a rate limit or a changed
            // API shape all land here.
            Log.i(TAG, "Update check skipped: " + error);
        }
    }

    /** The version of the latest GitHub release, or null when it cannot be read. */
    private static String fetchLatestVersion() throws Exception {
        HttpURLConnection connection = (HttpURLConnection) new URL(API_URL).openConnection();
        try {
            connection.setRequestProperty("Accept", "application/vnd.github+json");
            connection.setRequestProperty("User-Agent", "ArtCraft-Android");
            connection.setConnectTimeout(8000);
            connection.setReadTimeout(8000);
            if (connection.getResponseCode() != 200) {
                return null;
            }
            StringBuilder body = new StringBuilder();
            try (InputStream in = connection.getInputStream();
                 BufferedReader reader = new BufferedReader(new InputStreamReader(in, "UTF-8"))) {
                String line;
                while ((line = reader.readLine()) != null) {
                    body.append(line);
                }
            }
            // Releases are tagged "v0.2.0"; the builder's own version has no "v".
            String tag = new JSONObject(body.toString()).optString("tag_name", "");
            return tag.replaceFirst("^v", "").trim();
        } finally {
            connection.disconnect();
        }
    }

    /** True when {@code candidate} is a strictly newer version than {@code current}. */
    private static boolean isNewer(String candidate, String current) {
        int[] left = parseVersion(candidate);
        int[] right = parseVersion(current);
        if (left == null || right == null) {
            return false;
        }
        for (int i = 0; i < 3; i++) {
            if (left[i] != right[i]) {
                return left[i] > right[i];
            }
        }
        return false;
    }

    /** The three numeric components of a version, or null when it is not one. */
    private static int[] parseVersion(String version) {
        String[] parts = version.trim().split("[.\\-+]");
        if (parts.length < 3) {
            return null;
        }
        int[] numbers = new int[3];
        for (int i = 0; i < 3; i++) {
            try {
                numbers[i] = Integer.parseInt(parts[i].trim());
            } catch (NumberFormatException error) {
                return null;
            }
        }
        return numbers;
    }

    private static boolean wasNotified(Activity activity, String version) {
        SharedPreferences prefs = activity.getSharedPreferences(PREFS, Activity.MODE_PRIVATE);
        return version.equals(prefs.getString(KEY_NOTIFIED, ""));
    }

    private static void markNotified(Activity activity, String version) {
        SharedPreferences prefs = activity.getSharedPreferences(PREFS, Activity.MODE_PRIVATE);
        prefs.edit().putString(KEY_NOTIFIED, version).apply();
    }

    private static void showDialog(Activity activity, String version) {
        try {
            new AlertDialog.Builder(activity)
                    .setTitle("ArtCraft update available")
                    .setMessage("Version " + version + " is available. Download it to update this app.")
                    .setPositiveButton("Download", new android.content.DialogInterface.OnClickListener() {
                        @Override
                        public void onClick(android.content.DialogInterface dialog, int which) {
                            openDownloadPage(activity);
                        }
                    })
                    .setNegativeButton("Later", null)
                    .show();
        } catch (Throwable error) {
            Log.i(TAG, "Could not show the update dialog: " + error);
        }
    }

    private static void openDownloadPage(Activity activity) {
        try {
            activity.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(PAGE_URL)));
        } catch (Throwable error) {
            Log.i(TAG, "Could not open the release page: " + error);
        }
    }
}
