package com.artcraft.coimanide;

import android.app.Activity;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.os.Environment;
import android.util.Log;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashSet;
import java.util.Set;

/**
 * Forwards Coiman IDE's tool and terminal requests to Termux.
 *
 * <p>Coiman cannot execute Termux's binaries: they belong to another app. The Rust shell therefore
 * bundles a small shim that writes a request into {@code getCacheDir()/coiman-termux/<token>} and
 * then waits for the result files. This thread turns each request into an official Termux
 * {@code RUN_COMMAND} intent:</p>
 *
 * <pre>
 *   com.termux/com.termux.app.RunCommandService
 *   action  com.termux.RUN_COMMAND
 *   extras  RUN_COMMAND_PATH, ARGUMENTS, WORKDIR, BACKGROUND, RESULT_DIRECTORY
 * </pre>
 *
 * <p>The command itself is a generated {@code run.sh} in shared storage. Termux reads it with
 * {@code bash <script>} (shared storage is mounted noexec, so the script is never executed
 * directly), runs the tool or the interactive {@code script}-wrapped shell, and writes its own
 * result files beside the shim's {@code exit}/{@code out}/{@code err} markers.</p>
 *
 * <p>Everything the two apps exchange lives in {@code /sdcard/Documents/CoimanTermux}. Termux
 * cannot read an app's own {@code Android/data/<pkg>} directory, so shared storage is the only
 * location both can use. The broker publishes a one-line {@code status} file there; the shim reads
 * it before it starts waiting, which turns a missing Termux install or a missing permission into a
 * readable message in Coiman's terminal instead of a silent timeout.</p>
 *
 * ARTCRAFT_COIMANIDE_TERMUX_BROKER_V1
 */
final class TermuxBroker {
    private static final String TAG = "CoimanTermux";

    private static final String TERMUX_PACKAGE = "com.termux";
    private static final String TERMUX_SERVICE = "com.termux.app.RunCommandService";
    private static final String ACTION_RUN_COMMAND = "com.termux.RUN_COMMAND";
    private static final String TERMUX_BASH = "/data/data/com.termux/files/usr/bin/bash";
    private static final String RUN_COMMAND_PERMISSION = "com.termux.permission.RUN_COMMAND";

    /** Where Termux is downloaded from. */
    private static final String FDROID_TERMUX = "https://f-droid.org/packages/com.termux/";

    /** The settings the "always ignore" choice is remembered in. */
    private static final String PREFS = "coiman_termux";
    private static final String KEY_NEVER_ASK = "never_ask_to_install";

    private static final String EXTRA_COMMAND_PATH = "com.termux.RUN_COMMAND_PATH";
    private static final String EXTRA_ARGUMENTS = "com.termux.RUN_COMMAND_ARGUMENTS";
    private static final String EXTRA_WORKDIR = "com.termux.RUN_COMMAND_WORKDIR";
    private static final String EXTRA_BACKGROUND = "com.termux.RUN_COMMAND_BACKGROUND";
    private static final String EXTRA_SESSION_ACTION = "com.termux.RUN_COMMAND_SESSION_ACTION";
    private static final String EXTRA_COMMAND_LABEL = "com.termux.RUN_COMMAND_COMMAND_LABEL";
    private static final String EXTRA_RESULT_DIRECTORY = "com.termux.RUN_COMMAND_RESULT_DIRECTORY";
    private static final String EXTRA_RESULT_SINGLE_FILE = "com.termux.RUN_COMMAND_RESULT_SINGLE_FILE";

    /** The request code used for Termux's runtime permission. */
    private static final int REQ_TERMUX_PERMISSION = 0x5047;

    private final Activity activity;
    private final File requestRoot;
    private final File publicRoot;
    private final Set<String> handled = new HashSet<>();
    private volatile String status = "error: Coiman IDE's Termux bridge is starting.";
    private volatile boolean running = true;
    private Thread thread;

    TermuxBroker(Activity activity) {
        this.activity = activity;
        // The Rust side publishes its requests in getFilesDir()/coiman-termux (android-activity
        // exposes the app's files directory, not its cache directory), so both sides must name
        // the same folder here.
        this.requestRoot = new File(activity.getFilesDir(), "coiman-termux");
        this.publicRoot = new File(Environment.getExternalStorageDirectory(), "Documents/CoimanTermux");
        refreshStatus();
    }

    void start() {
        if (thread != null) return;
        // The shared folders must exist before the first request; the Rust side publishes them too,
        // but the broker is the first thing that runs on some cold starts.
        requestRoot.mkdirs();
        publicRoot.mkdirs();
        sweep();
        writeStatus();
        requestPermission();
        thread = new Thread(this::run, "coiman-termux-broker");
        thread.setDaemon(true);
        thread.start();
    }

    /**
     * Asks for Termux's RUN_COMMAND permission.
     *
     * <p>Termux declares it as a runtime ("dangerous") permission, so it is not granted by the
     * install: the app has to ask for it once. Without it the RUN_COMMAND service refuses every
     * command.</p>
     */
    private void requestPermission() {
        if (activity.checkSelfPermission(RUN_COMMAND_PERMISSION) == PackageManager.PERMISSION_GRANTED) {
            return;
        }
        activity.runOnUiThread(() -> {
            try {
                activity.requestPermissions(new String[]{RUN_COMMAND_PERMISSION}, REQ_TERMUX_PERMISSION);
            } catch (Throwable error) {
                Log.w(TAG, "Could not ask for Termux's RUN_COMMAND permission", error);
            }
        });
    }

    /**
     * Removes what a previous app process left behind. A request is only meaningful to the shim
     * that wrote it: that shim is a child of this app, so after a restart every request directory
     * is stale and must not reach Termux. Scratch trees older than a day are stale too.
     */
    private void sweep() {
        File[] requests = requestRoot.listFiles();
        if (requests != null) {
            for (File request : requests) deleteRecursively(request);
        }
        File scratchRoot = new File(publicRoot, "scratch");
        File[] scratch = scratchRoot.listFiles();
        long cutoff = System.currentTimeMillis() - 24L * 60 * 60 * 1000;
        if (scratch != null) {
            for (File entry : scratch) {
                if (entry.lastModified() < cutoff) deleteRecursively(entry);
            }
        }
    }

    void stop() {
        running = false;
        if (thread != null) {
            thread.interrupt();
            thread = null;
        }
    }

    /** One-line state for the Rust side (read by the shim from the shared status file). */
    String status() {
        return status;
    }

    private void run() {
        long lastStatusCheck = 0;
        while (running) {
            long now = System.currentTimeMillis();
            if (now - lastStatusCheck > 2000) {
                refreshStatus();
                lastStatusCheck = now;
            }
            try {
                File[] entries = requestRoot.listFiles();
                if (entries != null) {
                    Arrays.sort(entries, Comparator.comparingLong(File::lastModified));
                    for (File entry : entries) {
                        if (!running) break;
                        if (!entry.isDirectory() || entry.getName().startsWith(".")) continue;
                        synchronized (handled) {
                            if (handled.contains(entry.getName())) continue;
                            handled.add(entry.getName());
                        }
                        handle(entry);
                    }
                }
                Thread.sleep(80);
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                break;
            } catch (Throwable error) {
                Log.w(TAG, "Termux broker loop failed", error);
                try {
                    Thread.sleep(500);
                } catch (InterruptedException interrupted) {
                    Thread.currentThread().interrupt();
                    break;
                }
            }
        }
    }

    private void handle(File request) {
        String script = read(new File(request, "script"));
        String workdir = read(new File(request, "workdir"));
        String label = read(new File(request, "label"));
        String result = read(new File(request, "result"));
        deleteRecursively(request);
        if (script.isEmpty()) {
            Log.w(TAG, "Ignoring a request without a script: " + request.getName());
            return;
        }

        Intent intent = new Intent(ACTION_RUN_COMMAND);
        intent.setClassName(TERMUX_PACKAGE, TERMUX_SERVICE);
        intent.putExtra(EXTRA_COMMAND_PATH, TERMUX_BASH);
        intent.putExtra(EXTRA_ARGUMENTS, new String[]{script});
        if (!workdir.isEmpty()) {
            intent.putExtra(EXTRA_WORKDIR, workdir);
        }
        intent.putExtra(EXTRA_BACKGROUND, true);
        // 0 = keep the current Termux session (the background task is not shown as a session).
        intent.putExtra(EXTRA_SESSION_ACTION, "0");
        intent.putExtra(EXTRA_COMMAND_LABEL, label.isEmpty() ? "Coiman IDE" : label);
        if (!result.isEmpty()) {
            intent.putExtra(EXTRA_RESULT_DIRECTORY, result);
            intent.putExtra(EXTRA_RESULT_SINGLE_FILE, false);
        }
        try {
            activity.startService(intent);
            Log.i(TAG, "Sent to Termux: " + (label.isEmpty() ? script : label));
        } catch (Throwable error) {
            Log.e(TAG, "Could not start Termux's RUN_COMMAND service", error);
            writeBrokerError(script, error);
        }
    }

    private void writeBrokerError(String script, Throwable error) {
        File scratch = new File(script).getParentFile();
        if (scratch == null) return;
        String message = "Termux could not be started: " + error;
        try (FileOutputStream out = new FileOutputStream(new File(scratch, "error"))) {
            out.write(message.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
        }
    }

    private void refreshStatus() {
        String next = computeStatus();
        if (!next.equals(status)) {
            status = next;
            Log.i(TAG, "Termux status: " + next);
            writeStatus();
            if (next.startsWith("error: Termux is not installed")) {
                maybeAskToInstallTermux();
            }
        }
    }

    /**
     * Asks the user to install Termux, which is where Coiman's Git, its toolchain and its terminal
     * live.
     *
     * <p>The dialog offers the three answers the update window offers: download it, ignore it this
     * once, or never ask again. It is shown on the UI thread, like the update window.</p>
     */
    private void maybeAskToInstallTermux() {
        SharedPreferences prefs = activity.getSharedPreferences(PREFS, Activity.MODE_PRIVATE);
        if (prefs.getBoolean(KEY_NEVER_ASK, false)) {
            return;
        }
        activity.runOnUiThread(() -> {
            try {
                new android.app.AlertDialog.Builder(activity)
                        .setTitle("Termux is needed")
                        .setMessage("Coiman IDE runs Git, your toolchain and the terminal inside "
                                + "Termux, which is not installed on this device.\n\n"
                                + "Install Termux from F-Droid to use them.")
                        .setPositiveButton("Download", new android.content.DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(android.content.DialogInterface dialog, int which) {
                                openTermuxPage();
                            }
                        })
                        .setNeutralButton("Ignore once", null)
                        .setNegativeButton("Always ignore", new android.content.DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(android.content.DialogInterface dialog, int which) {
                                prefs.edit().putBoolean(KEY_NEVER_ASK, true).apply();
                                Log.i(TAG, "not asking about Termux again");
                            }
                        })
                        .show();
            } catch (Throwable error) {
                Log.w(TAG, "Could not ask about Termux: " + error);
            }
        });
    }

    /** Opens Termux's page on F-Droid. */
    private void openTermuxPage() {
        try {
            activity.startActivity(new Intent(Intent.ACTION_VIEW, android.net.Uri.parse(FDROID_TERMUX)));
        } catch (Throwable error) {
            Log.w(TAG, "Could not open the F-Droid page: " + error);
        }
    }

    private String computeStatus() {
        PackageManager manager = activity.getPackageManager();
        boolean installed;
        try {
            manager.getPackageInfo(TERMUX_PACKAGE, 0);
            installed = true;
        } catch (PackageManager.NameNotFoundException missing) {
            installed = false;
        }
        if (!installed) {
            return "error: Termux is not installed. Install Termux from F-Droid (or GitHub), open it once, "
                + "then install the terminal helper with: pkg install util-linux";
        }
        if (activity.checkSelfPermission(RUN_COMMAND_PERMISSION) != PackageManager.PERMISSION_GRANTED) {
            return "error: Coiman IDE does not have Termux's RUN_COMMAND permission yet. Grant it "
                + "when Android asks (the request appears the first time the app starts).";
        }
        if (!publicRoot.isDirectory() && !publicRoot.mkdirs()) {
            return "error: Coiman IDE cannot create " + publicRoot.getAbsolutePath()
                + ". Grant 'All files access' in Android Settings > Apps > Coiman IDE.";
        }
        File probe = new File(publicRoot, ".write-test");
        try (FileOutputStream out = new FileOutputStream(probe)) {
            out.write(1);
        } catch (IOException error) {
            return "error: Coiman IDE cannot write to " + publicRoot.getAbsolutePath()
                + ". Grant 'All files access' in Android Settings > Apps > Coiman IDE.";
        } finally {
            //noinspection ResultOfMethodCallIgnored
            probe.delete();
        }
        return "ready";
    }

    private void writeStatus() {
        File file = new File(publicRoot, "status");
        try (FileOutputStream out = new FileOutputStream(file)) {
            out.write(status.getBytes(StandardCharsets.UTF_8));
        } catch (IOException ignored) {
            // If shared storage is not writable the status file cannot carry the explanation; the
            // shim's timeout message covers that case.
        }
    }

    private static String read(File file) {
        if (!file.isFile()) return "";
        try (FileInputStream input = new FileInputStream(file)) {
            byte[] data = new byte[(int) Math.min(file.length(), 8192)];
            int read = input.read(data);
            return read <= 0 ? "" : new String(data, 0, read, StandardCharsets.UTF_8).trim();
        } catch (IOException error) {
            return "";
        }
    }

    private static void deleteRecursively(File file) {
        File[] children = file.listFiles();
        if (children != null) {
            for (File child : children) deleteRecursively(child);
        }
        //noinspection ResultOfMethodCallIgnored
        file.delete();
    }
}
