// The Android app's activity (docs/behavior/android.md): the system's
// NativeActivity, which loads libtracedraw.so and hands it the window and
// the input, plus the few things the editor can only do from Java: the
// system document picker (Storage Access Framework), reading and writing
// the documents it returns, the system clipboard, the print service, the
// share sheet and "open with" (files served by SharedFiles), and an
// invisible view the on-screen keyboard types into (TextInputView). The Rust side calls
// the public methods through JNI from its own thread; a picked document
// comes back through the native method nativeDocumentResult, and so does a
// document another app (a file manager, a mail, Share) asks TraceDraw to
// open, under the request code OPENED_ELSEWHERE. Built by
// scripts/android/build.sh with javac and d8 into classes.dex.
package dev.tracedraw.app;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ContentResolver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.print.PageRange;
import android.print.PrintAttributes;
import android.print.PrintDocumentAdapter;
import android.print.PrintDocumentInfo;
import android.os.Process;
import android.print.PrintManager;
import android.os.Parcelable;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewTreeObserver;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;

public class TraceDrawActivity extends NativeActivity {
    private static final String TAG = "tracedraw";

    static {
        // The same library NativeActivity loads (android.app.lib_name), so
        // that the native methods below resolve to its symbols.
        System.loadLibrary("tracedraw");
    }

    /**
     * The documents picked for the request `request` (a code the Rust side
     * chose), as newline-separated lists of the same length: the content
     * URIs, their display names and their MIME types (an empty entry when
     * unknown). All three are null when the picker was cancelled or failed.
     * Implemented in Rust (android.rs).
     */
    private native void nativeDocumentResult(int request, String uris, String names, String mimes);

    /**
     * What the on-screen keyboard's input method did in TextInputView: the
     * text it composes (empty: the composition ends), the text it commits,
     * and the characters it deletes before and after the caret.
     * Implemented in Rust (android.rs).
     */
    private native void nativeImeCompose(String text);
    private native void nativeImeCommit(String text);
    private native void nativeImeDelete(int before, int after);

    /** The view the on-screen keyboard types into. */
    private TextInputView textInput;

    /**
     * Wait, at most `timeoutMs`, until the editor's thread has let go of
     * the activity's window, which it does after backing the drawings up
     * (eframe's save on suspend). Implemented in Rust (android.rs).
     */
    private native void nativeAwaitWindowGone(long timeoutMs);

    /** How long the end of the activity waits for the backup. */
    private static final long BACKUP_WAIT_MS = 10000;

    // ----- documents other apps open with TraceDraw ---------------------------

    /**
     * The request code of documents another app asked TraceDraw to open
     * (VIEW, EDIT, SEND): the editor opens each in a tab of its own
     * (documents::OPENED_ELSEWHERE in android.rs).
     */
    private static final int OPENED_ELSEWHERE = 0;

    /** An intent that came before the editor could take documents. */
    private Intent pendingOpen;

    /** Set once the editor's native methods are registered. */
    private boolean editorReady;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        textInput = new TextInputView(this);
        addContentView(textInput, new ViewGroup.LayoutParams(1, 1));
        // The screen's safe area, read again at every layout (a rotation,
        // a resize) for the editor (#1917).
        final View decor = getWindow().getDecorView();
        decor.getViewTreeObserver().addOnGlobalLayoutListener(
            new ViewTreeObserver.OnGlobalLayoutListener() {
                @Override
                public void onGlobalLayout() {
                    readScreenInsets(decor);
                }
            });
        // The intent that started the app; not again when the system
        // recreates the activity (its documents are open already) or when
        // the app is brought back from the recent apps, which hands the
        // first intent over once more.
        Intent start = getIntent();
        boolean fromRecents = start != null
                && (start.getFlags() & Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0;
        if (savedInstanceState == null && !fromRecents) {
            pendingOpen = start;
        }
    }

    /**
     * The activity ends: relaunched by the system (a change of its
     * resource overlays, which no configChanges entry can take over, or of
     * the screen density) or finished. NativeActivity.onDestroy then waits
     * for the editor's thread to return from android_main, which it never
     * does (winit runs one event loop per process and cannot start
     * another), so the app froze for good with the system's starting
     * window on the screen. Instead, once the editor has given up the
     * window (and backed the drawings up while doing so, which happens
     * inside super.onDestroy), the process ends; the manifest's
     * stateNotNeeded makes the system start the activity again in a new
     * process, which offers the backed-up drawings.
     */
    @Override
    protected void onDestroy() {
        final boolean relaunch = isChangingConfigurations();
        Thread end = new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    nativeAwaitWindowGone(BACKUP_WAIT_MS);
                } catch (UnsatisfiedLinkError e) {
                    // The editor's thread has not started: nothing to back up.
                    Log.w(TAG, "ending: " + e);
                }
                Log.i(TAG, relaunch
                        ? "the activity is relaunched: the process ends and the system starts it again"
                        : "the activity ends: the process ends");
                Process.killProcess(Process.myPid());
            }
        }, "tracedraw-end");
        end.setDaemon(true);
        end.start();
        super.onDestroy();
    }

    // ----- the on-screen keyboard ---------------------------------------------

    /**
     * Show the on-screen keyboard, typing into the editor (`on`: a text or
     * a field takes typed text), or hide it. `again`: it was shown and the
     * user tapped into the text or field once more, so it comes back if
     * the system's key hid it, still connected to what it was composing.
     */
    public void showTextInput(final boolean on, final boolean again) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm == null || textInput == null) {
                    return;
                }
                if (on) {
                    if (!again || !textInput.isFocused()) {
                        boolean focused = textInput.requestFocus();
                        imm.restartInput(textInput);
                        if (!focused) {
                            Log.w(TAG, "keyboard: the text view did not take the focus");
                        }
                    }
                    showKeyboard(imm, 0);
                } else {
                    keyboardWanted = false;
                    // A composition left open ends with the editing.
                    imm.restartInput(textInput);
                    imm.hideSoftInputFromWindow(textInput.getWindowToken(), 0);
                }
            }
        });
    }

    /** Whether the editor wants the keyboard (shown or being shown). */
    private boolean keyboardWanted;

    /** How many times the keyboard is asked again when the system ignores the request. */
    private static final int SHOW_TRIES = 5;

    /**
     * Ask for the keyboard on the text view. The system ignores the
     * request while the view is not yet the one the input method serves
     * (its focus reaches the input method a moment later, more so right
     * after a tap): then the request is made again shortly after.
     */
    private void showKeyboard(final InputMethodManager imm, final int tried) {
        keyboardWanted = true;
        boolean shown = imm.showSoftInput(textInput, 0);
        if (shown) {
            Log.i(TAG, "keyboard: shown" + (tried > 0 ? " after " + tried + " retries" : ""));
            return;
        }
        if (tried + 1 >= SHOW_TRIES) {
            Log.w(TAG, "keyboard: the system did not show it (focus " + textInput.isFocused()
                    + ", window focus " + textInput.hasWindowFocus() + ")");
            return;
        }
        textInput.postDelayed(new Runnable() {
            @Override
            public void run() {
                if (!keyboardWanted) {
                    return;
                }
                if (!textInput.isFocused()) {
                    textInput.requestFocus();
                    imm.restartInput(textInput);
                }
                showKeyboard(imm, tried + 1);
            }
        }, 100);
    }

    void imeCompose(String text) {
        try {
            nativeImeCompose(text);
        } catch (UnsatisfiedLinkError e) {
            Log.w(TAG, "keyboard: " + e);
        }
    }

    void imeCommit(String text) {
        try {
            nativeImeCommit(text);
        } catch (UnsatisfiedLinkError e) {
            Log.w(TAG, "keyboard: " + e);
        }
    }

    void imeDelete(int before, int after) {
        try {
            nativeImeDelete(before, after);
        } catch (UnsatisfiedLinkError e) {
            Log.w(TAG, "keyboard: " + e);
        }
    }

    // ----- the intents of documents other apps open ---------------------------

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        // The app runs (launchMode singleTask): the documents open in tabs
        // of the running editor.
        if (editorReady) {
            deliverOpened(intent);
        } else {
            pendingOpen = intent;
        }
    }

    /**
     * Called by the editor once it can take documents: the documents of
     * the intent that started the app, if any, are handed over now.
     */
    public void readyForDocuments() {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                editorReady = true;
                Intent intent = pendingOpen;
                pendingOpen = null;
                if (intent != null) {
                    deliverOpened(intent);
                }
            }
        });
    }

    /** The documents of an intent that asks to open, edit or share files. */
    private void deliverOpened(Intent intent) {
        String action = intent.getAction();
        if (action == null) {
            return;
        }
        ArrayList<Uri> uris = new ArrayList<Uri>();
        try {
            if (Intent.ACTION_VIEW.equals(action) || Intent.ACTION_EDIT.equals(action)) {
                if (intent.getData() != null) {
                    uris.add(intent.getData());
                }
            } else if (Intent.ACTION_SEND.equals(action)) {
                Parcelable stream = intent.getParcelableExtra(Intent.EXTRA_STREAM);
                if (stream instanceof Uri) {
                    uris.add((Uri) stream);
                }
            } else if (Intent.ACTION_SEND_MULTIPLE.equals(action)) {
                ArrayList<Parcelable> streams = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
                if (streams != null) {
                    for (Parcelable p : streams) {
                        if (p instanceof Uri && !uris.contains(p)) {
                            uris.add((Uri) p);
                        }
                    }
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "cannot read the documents of " + action, e);
        }
        if (uris.isEmpty()) {
            return;
        }
        // Rights granted with a VIEW or SEND last as long as the activity:
        // nothing to keep past it.
        sendDocuments(OPENED_ELSEWHERE, uris, 0);
    }

    // ----- the document picker -----------------------------------------------

    /**
     * Show the system's document picker for one document (or several when
     * `multiple`) of any type; the editor tells the type from the name.
     * The result comes through nativeDocumentResult.
     */
    public void openDocument(final int request, final boolean multiple) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                    intent.addCategory(Intent.CATEGORY_OPENABLE);
                    intent.setType("*/*");
                    intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                    startActivityForResult(intent, request);
                } catch (Exception e) {
                    Log.w(TAG, "the document picker could not open", e);
                    nativeDocumentResult(request, null, null, null);
                }
            }
        });
    }

    /** The requests waiting for a folder from the folder picker (#1926). */
    private final java.util.HashSet<Integer> treeRequests = new java.util.HashSet<Integer>();

    /**
     * Show the system's folder picker (ACTION_OPEN_DOCUMENT_TREE). The
     * documents in the folder chosen (not its subfolders) come through
     * nativeDocumentResult as picked documents (#1926).
     */
    public void openDocumentTree(final int request) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    treeRequests.add(request);
                    Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                    startActivityForResult(intent, request);
                } catch (Exception e) {
                    Log.w(TAG, "the folder picker could not open", e);
                    treeRequests.remove(request);
                    nativeDocumentResult(request, null, null, null);
                }
            }
        });
    }

    /**
     * Hand the documents in the folder `tree` (a tree URI the folder
     * picker gave) to the editor under `requestCode`; subfolders are left
     * out. The right to read the folder is kept across restarts.
     */
    private void sendTree(int requestCode, Uri tree) {
        try {
            getContentResolver().takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (Exception e) {
            Log.i(TAG, "no persistable permission for " + tree);
        }
        StringBuilder u = new StringBuilder();
        StringBuilder n = new StringBuilder();
        StringBuilder m = new StringBuilder();
        Cursor cursor = null;
        try {
            Uri children = android.provider.DocumentsContract.buildChildDocumentsUriUsingTree(
                tree, android.provider.DocumentsContract.getTreeDocumentId(tree));
            cursor = getContentResolver().query(children, new String[] {
                android.provider.DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                android.provider.DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                android.provider.DocumentsContract.Document.COLUMN_MIME_TYPE,
            }, null, null, null);
            while (cursor != null && cursor.moveToNext()) {
                String id = cursor.getString(0);
                String name = cursor.getString(1);
                String mime = cursor.getString(2);
                if (id == null
                    || android.provider.DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                    continue;
                }
                Uri doc = android.provider.DocumentsContract.buildDocumentUriUsingTree(tree, id);
                if (u.length() > 0) {
                    u.append('\n');
                    n.append('\n');
                    m.append('\n');
                }
                u.append(doc.toString());
                n.append(name == null ? "document" : name.replace('\n', ' '));
                m.append(mime == null ? "" : mime.replace('\n', ' '));
            }
        } catch (Exception e) {
            Log.w(TAG, "the folder could not be listed", e);
        } finally {
            if (cursor != null) {
                cursor.close();
            }
        }
        if (u.length() == 0) {
            nativeDocumentResult(requestCode, null, null, null);
        } else {
            nativeDocumentResult(requestCode, u.toString(), n.toString(), m.toString());
        }
    }

    /**
     * Show the system's picker for a place to save a new document named
     * `name` of type `mime`. The result comes through nativeDocumentResult.
     */
    public void createDocument(final int request, final String mime, final String name) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                    intent.addCategory(Intent.CATEGORY_OPENABLE);
                    intent.setType(mime == null || mime.isEmpty() ? "application/octet-stream" : mime);
                    intent.putExtra(Intent.EXTRA_TITLE, name);
                    startActivityForResult(intent, request);
                } catch (Exception e) {
                    Log.w(TAG, "the save picker could not open", e);
                    nativeDocumentResult(request, null, null, null);
                }
            }
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        boolean tree = treeRequests.remove(requestCode);
        if (resultCode != RESULT_OK || data == null) {
            nativeDocumentResult(requestCode, null, null, null);
            return;
        }
        // A folder from the folder picker: its documents (#1926).
        if (tree) {
            if (data.getData() == null) {
                nativeDocumentResult(requestCode, null, null, null);
            } else {
                sendTree(requestCode, data.getData());
            }
            return;
        }
        ArrayList<Uri> uris = new ArrayList<Uri>();
        if (data.getData() != null) {
            uris.add(data.getData());
        }
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) {
                Uri u = clip.getItemAt(i).getUri();
                if (u != null && !uris.contains(u)) {
                    uris.add(u);
                }
            }
        }
        if (uris.isEmpty()) {
            nativeDocumentResult(requestCode, null, null, null);
            return;
        }
        // Keep the right to read (and write, when granted) the document
        // after the process restarts, so the recent list reopens it.
        int flags = data.getFlags()
            & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        sendDocuments(requestCode, uris, flags);
    }

    /**
     * Hand `uris` to the editor under `requestCode`, with their names and
     * types, keeping the persistable permissions `flags` names.
     */
    private void sendDocuments(int requestCode, ArrayList<Uri> uris, int flags) {
        StringBuilder u = new StringBuilder();
        StringBuilder n = new StringBuilder();
        StringBuilder m = new StringBuilder();
        for (int i = 0; i < uris.size(); i++) {
            Uri uri = uris.get(i);
            try {
                if (flags != 0) {
                    getContentResolver().takePersistableUriPermission(uri, flags);
                }
            } catch (Exception e) {
                Log.i(TAG, "no persistable permission for " + uri);
            }
            if (i > 0) {
                u.append('\n');
                n.append('\n');
                m.append('\n');
            }
            u.append(uri.toString());
            n.append(displayName(uri).replace('\n', ' '));
            String mime = null;
            try {
                mime = getContentResolver().getType(uri);
            } catch (Exception e) {
                // Unknown type: the editor goes by the name.
            }
            m.append(mime == null ? "" : mime.replace('\n', ' '));
        }
        nativeDocumentResult(requestCode, u.toString(), n.toString(), m.toString());
    }

    /** The name the document's provider shows for it; its last path segment when it has none. */
    private String displayName(Uri uri) {
        Cursor cursor = null;
        try {
            cursor = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null);
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    String name = cursor.getString(column);
                    if (name != null && !name.isEmpty()) {
                        return name;
                    }
                }
            }
        } catch (Exception e) {
            Log.i(TAG, "no display name for " + uri);
        } finally {
            if (cursor != null) {
                cursor.close();
            }
        }
        String last = uri.getLastPathSegment();
        if (last == null || last.isEmpty()) {
            return "document";
        }
        // Document ids look like "primary:Pictures/a.png": the name is the
        // part after the last separator.
        int cut = Math.max(last.lastIndexOf('/'), last.lastIndexOf(':'));
        return cut >= 0 && cut + 1 < last.length() ? last.substring(cut + 1) : last;
    }

    // ----- reading and writing documents --------------------------------------

    /** The whole content of the document at `uri`; null when it cannot be read. */
    public byte[] readDocument(String uri) {
        InputStream in = null;
        try {
            in = getContentResolver().openInputStream(Uri.parse(uri));
            if (in == null) {
                return null;
            }
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            int got;
            while ((got = in.read(buffer)) != -1) {
                out.write(buffer, 0, got);
            }
            return out.toByteArray();
        } catch (Exception e) {
            Log.w(TAG, "cannot read " + uri + ": " + e);
            return null;
        } finally {
            close(in);
        }
    }

    /**
     * Replace the content of the document at `uri` with `data`; false when
     * it cannot be written. The document is truncated first ("wt"); a
     * provider without that mode gets a plain write.
     */
    public boolean writeDocument(String uri, byte[] data) {
        ContentResolver resolver = getContentResolver();
        OutputStream out = null;
        try {
            try {
                out = resolver.openOutputStream(Uri.parse(uri), "wt");
            } catch (Exception e) {
                out = resolver.openOutputStream(Uri.parse(uri), "w");
            }
            if (out == null) {
                return false;
            }
            out.write(data);
            out.flush();
            return true;
        } catch (Exception e) {
            Log.w(TAG, "cannot write " + uri + ": " + e);
            return false;
        } finally {
            close(out);
        }
    }

    private static void close(java.io.Closeable c) {
        if (c != null) {
            try {
                c.close();
            } catch (Exception e) {
                // Nothing left to do with it.
            }
        }
    }

    // ----- printing, sharing and "open with" (#1398) ----------------------------

    /**
     * Hand the PDF at `path` (one of the app's files) to the system's print
     * service as the job `job`; its dialog picks the printer, the copies
     * and the paper. Whether the dialog opened.
     */
    public boolean printFile(final String path, final String job) {
        Boolean done = onUiThread(new Callable<Boolean>() {
            @Override
            public Boolean call() {
                PrintManager printing = (PrintManager) getSystemService(Context.PRINT_SERVICE);
                final File file = new File(path);
                if (printing == null || !file.isFile()) {
                    return Boolean.FALSE;
                }
                printing.print(job, new PdfAdapter(file, job), null);
                return Boolean.TRUE;
            }
        }, Boolean.FALSE);
        return done != null && done.booleanValue();
    }

    /** Hands a ready PDF to the print service as it is. */
    private static final class PdfAdapter extends PrintDocumentAdapter {
        private final File file;
        private final String name;

        PdfAdapter(File file, String name) {
            this.file = file;
            this.name = name;
        }

        @Override
        public void onLayout(PrintAttributes oldAttributes, PrintAttributes newAttributes,
                CancellationSignal cancel, LayoutResultCallback callback, Bundle extras) {
            if (cancel != null && cancel.isCanceled()) {
                callback.onLayoutCancelled();
                return;
            }
            PrintDocumentInfo info = new PrintDocumentInfo.Builder(name.endsWith(".pdf") ? name : name + ".pdf")
                .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
                .setPageCount(PrintDocumentInfo.PAGE_COUNT_UNKNOWN)
                .build();
            callback.onLayoutFinished(info, oldAttributes == null || !oldAttributes.equals(newAttributes));
        }

        @Override
        public void onWrite(PageRange[] pages, ParcelFileDescriptor destination,
                CancellationSignal cancel, WriteResultCallback callback) {
            InputStream in = null;
            FileOutputStream out = null;
            try {
                in = new FileInputStream(file);
                out = new FileOutputStream(destination.getFileDescriptor());
                byte[] buffer = new byte[64 * 1024];
                int got;
                while ((got = in.read(buffer)) != -1) {
                    if (cancel != null && cancel.isCanceled()) {
                        callback.onWriteCancelled();
                        return;
                    }
                    out.write(buffer, 0, got);
                }
                out.flush();
                callback.onWriteFinished(new PageRange[] {PageRange.ALL_PAGES});
            } catch (Exception e) {
                Log.w(TAG, "print: " + e);
                callback.onWriteFailed(e.toString());
            } finally {
                close(in);
                close(out);
            }
        }
    }

    /**
     * Offer the file at `path` (one of the app's files, of type `mime`) to
     * another app through the share sheet, with `subject` for a mail;
     * `mail` limits the choice to mail apps. Whether the sheet opened.
     */
    public boolean shareFile(final String path, final String mime, final String subject, final boolean mail) {
        Boolean done = onUiThread(new Callable<Boolean>() {
            @Override
            public Boolean call() {
                Uri uri = SharedFiles.uriFor(TraceDrawActivity.this, new File(path));
                if (uri == null) {
                    Log.w(TAG, "share: " + path + " is not one of the app's files");
                    return Boolean.FALSE;
                }
                Intent send = new Intent(Intent.ACTION_SEND);
                send.setType(mime);
                send.putExtra(Intent.EXTRA_STREAM, uri);
                send.putExtra(Intent.EXTRA_SUBJECT, subject);
                send.setClipData(ClipData.newRawUri(subject, uri));
                send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                if (mail) {
                    // Only the apps that send mail.
                    send.setSelector(new Intent(Intent.ACTION_SENDTO, Uri.parse("mailto:")));
                }
                try {
                    startActivity(Intent.createChooser(send, subject));
                    return Boolean.TRUE;
                } catch (ActivityNotFoundException e) {
                    Log.w(TAG, "share: no app takes " + mime);
                    return Boolean.FALSE;
                }
            }
        }, Boolean.FALSE);
        return done != null && done.booleanValue();
    }

    /**
     * Open the file at `path` (one of the app's files, of type `mime`) in
     * an app that edits it, with the right to write it back; the system
     * asks which app when there are several. False when none edits it.
     */
    public boolean editFile(final String path, final String mime) {
        Boolean done = onUiThread(new Callable<Boolean>() {
            @Override
            public Boolean call() {
                Uri uri = SharedFiles.uriFor(TraceDrawActivity.this, new File(path));
                if (uri == null) {
                    Log.w(TAG, "edit: " + path + " is not one of the app's files");
                    return Boolean.FALSE;
                }
                Intent edit = new Intent(Intent.ACTION_EDIT);
                edit.setDataAndType(uri, mime);
                edit.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
                try {
                    startActivity(edit);
                    return Boolean.TRUE;
                } catch (ActivityNotFoundException e) {
                    Log.i(TAG, "edit: no app edits " + mime);
                    return Boolean.FALSE;
                }
            }
        }, Boolean.FALSE);
        return done != null && done.booleanValue();
    }

    /**
     * Show `target` in the app the system has for it: a web address (or
     * any other URI with a scheme, such as a picked document's content
     * URI), or one of the app's files (an absolute path), of type `mime`
     * when it is not empty. False when no app opens it.
     */
    public boolean viewTarget(final String target, final String mime) {
        Boolean done = onUiThread(new Callable<Boolean>() {
            @Override
            public Boolean call() {
                Intent view = new Intent(Intent.ACTION_VIEW);
                if (target.startsWith("/")) {
                    Uri uri = SharedFiles.uriFor(TraceDrawActivity.this, new File(target));
                    if (uri == null) {
                        Log.w(TAG, "view: " + target + " is not one of the app's files");
                        return Boolean.FALSE;
                    }
                    String type = mime == null || mime.isEmpty() ? SharedFiles.mimeOf(target) : mime;
                    view.setDataAndType(uri, type);
                    view.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                } else {
                    // A web address, or a document the picker returned,
                    // whose grant this app holds and passes on to read it.
                    Uri uri = Uri.parse(target);
                    if (mime == null || mime.isEmpty()) {
                        view.setData(uri);
                    } else {
                        view.setDataAndType(uri, mime);
                    }
                    if ("content".equals(uri.getScheme())) {
                        view.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                    }
                }
                try {
                    startActivity(view);
                    return Boolean.TRUE;
                } catch (ActivityNotFoundException e) {
                    Log.i(TAG, "view: no app opens " + target);
                    return Boolean.FALSE;
                }
            }
        }, Boolean.FALSE);
        return done != null && done.booleanValue();
    }

    // ----- the system clipboard -----------------------------------------------

    /** The clipboard's text, or null when it holds none (or the app is not in front). */
    public String getClipboardText() {
        return onUiThread(new Callable<String>() {
            @Override
            public String call() {
                ClipboardManager clipboard = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                if (clipboard == null || !clipboard.hasPrimaryClip()) {
                    return null;
                }
                ClipData clip = clipboard.getPrimaryClip();
                if (clip == null || clip.getItemCount() == 0) {
                    return null;
                }
                // A document (a picture copied in a gallery or a browser)
                // is no text: its URI would paste as one (#1403).
                for (int i = 0; i < clip.getItemCount(); i++) {
                    ClipData.Item item = clip.getItemAt(i);
                    if (item.getUri() != null && item.getText() == null) {
                        continue;
                    }
                    CharSequence text = item.coerceToText(TraceDrawActivity.this);
                    if (text != null && text.length() > 0) {
                        return text.toString();
                    }
                }
                return null;
            }
        }, null);
    }

    /**
     * The documents on the clipboard (pictures and files other apps
     * copied), one per line as "uri TAB mime TAB name"; null when there is
     * none. Reading the clip grants this app the right to read them.
     */
    public String getClipboardDocuments() {
        return onUiThread(new Callable<String>() {
            @Override
            public String call() {
                ClipboardManager clipboard = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                if (clipboard == null || !clipboard.hasPrimaryClip()) {
                    return null;
                }
                ClipData clip = clipboard.getPrimaryClip();
                if (clip == null) {
                    return null;
                }
                StringBuilder out = new StringBuilder();
                for (int i = 0; i < clip.getItemCount(); i++) {
                    Uri uri = clip.getItemAt(i).getUri();
                    if (uri == null) {
                        continue;
                    }
                    String mime = null;
                    try {
                        mime = getContentResolver().getType(uri);
                    } catch (Exception e) {
                        // Unknown type: the editor goes by the name.
                    }
                    if (mime == null && clip.getDescription() != null
                        && clip.getDescription().getMimeTypeCount() > 0) {
                        mime = clip.getDescription().getMimeType(0);
                    }
                    if (out.length() > 0) {
                        out.append('\n');
                    }
                    out.append(uri.toString().replace('\t', ' ').replace('\n', ' '));
                    out.append('\t');
                    out.append(mime == null ? "" : mime.replace('\t', ' ').replace('\n', ' '));
                    out.append('\t');
                    out.append(displayName(uri).replace('\t', ' ').replace('\n', ' '));
                }
                return out.length() == 0 ? null : out.toString();
            }
        }, null);
    }

    /** Put `text` on the clipboard; whether it went there. */
    public boolean setClipboardText(final String text) {
        Boolean done = onUiThread(new Callable<Boolean>() {
            @Override
            public Boolean call() {
                ClipboardManager clipboard = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                if (clipboard == null) {
                    return Boolean.FALSE;
                }
                clipboard.setPrimaryClip(ClipData.newPlainText("TraceDraw", text));
                return Boolean.TRUE;
            }
        }, Boolean.FALSE);
        return done != null && done.booleanValue();
    }

    // ----- the screen's safe area (#1917) -------------------------------------

    /**
     * The room the system bars and the camera cutout take on each side of
     * the window and the radii of the screen's rounded corners, in pixels,
     * as "top bottom left right top-left top-right bottom-right
     * bottom-left"; empty before the first layout. Written on the UI
     * thread at each layout, read by the editor's thread.
     */
    private volatile String screenInsets = "";

    private void readScreenInsets(View decor) {
        WindowInsets insets = decor.getRootWindowInsets();
        if (insets == null) {
            return;
        }
        int top, bottom, left, right;
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            // Bars that are hidden (the status bar of a full screen
            // window) take no room.
            android.graphics.Insets safe = insets.getInsets(
                WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            top = safe.top;
            bottom = safe.bottom;
            left = safe.left;
            right = safe.right;
        } else {
            top = insets.getSystemWindowInsetTop();
            bottom = insets.getSystemWindowInsetBottom();
            left = insets.getSystemWindowInsetLeft();
            right = insets.getSystemWindowInsetRight();
            if (android.os.Build.VERSION.SDK_INT >= 28) {
                android.view.DisplayCutout cutout = insets.getDisplayCutout();
                if (cutout != null) {
                    top = Math.max(top, cutout.getSafeInsetTop());
                    bottom = Math.max(bottom, cutout.getSafeInsetBottom());
                    left = Math.max(left, cutout.getSafeInsetLeft());
                    right = Math.max(right, cutout.getSafeInsetRight());
                }
            }
        }
        int[] radii = new int[4];
        if (android.os.Build.VERSION.SDK_INT >= 31) {
            int[] positions = {
                android.view.RoundedCorner.POSITION_TOP_LEFT,
                android.view.RoundedCorner.POSITION_TOP_RIGHT,
                android.view.RoundedCorner.POSITION_BOTTOM_RIGHT,
                android.view.RoundedCorner.POSITION_BOTTOM_LEFT,
            };
            for (int i = 0; i < 4; i++) {
                android.view.RoundedCorner corner = insets.getRoundedCorner(positions[i]);
                radii[i] = corner == null ? 0 : corner.getRadius();
            }
        }
        screenInsets = top + " " + bottom + " " + left + " " + right + " "
            + radii[0] + " " + radii[1] + " " + radii[2] + " " + radii[3];
    }

    /** The screen's safe area as last read (see screenInsets). */
    public String getScreenInsets() {
        return screenInsets;
    }

    // ----- accessibility settings (#1412) -------------------------------------

    /**
     * The system's settings the interface follows, as "font-scale contrast
     * dark": the font scale (Settings > Display > Font size), 1 when the
     * contrast is raised (Android 14's contrast setting at medium or high,
     * or high contrast text) else 0, and 1 when the system shows dark
     * windows else 0. Read by the editor every few seconds; a font scale
     * change reaches the activity without restarting it (configChanges).
     */
    public String getAccessibility() {
        android.content.res.Configuration config = getResources().getConfiguration();
        int night = config.uiMode & android.content.res.Configuration.UI_MODE_NIGHT_MASK;
        boolean dark = night == android.content.res.Configuration.UI_MODE_NIGHT_YES;
        boolean contrast = false;
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            try {
                android.app.UiModeManager modes =
                    (android.app.UiModeManager) getSystemService(Context.UI_MODE_SERVICE);
                contrast = modes != null && modes.getContrast() >= 0.5f;
            } catch (Exception e) {
                // No contrast setting to read.
            }
        }
        try {
            contrast = contrast
                || android.provider.Settings.Secure.getInt(getContentResolver(), "high_text_contrast_enabled", 0) == 1;
        } catch (Exception e) {
            // The setting is closed to apps on this system.
        }
        return config.fontScale + " " + (contrast ? "1" : "0") + " " + (dark ? "1" : "0");
    }

    /**
     * Run `task` on the UI thread and wait for its result (two seconds at
     * most); `fallback` when it fails or takes longer. The caller is the
     * editor's own thread, so the UI thread is free to run it.
     */
    private <T> T onUiThread(Callable<T> task, T fallback) {
        FutureTask<T> future = new FutureTask<T>(task);
        runOnUiThread(future);
        try {
            return future.get(2, TimeUnit.SECONDS);
        } catch (Exception e) {
            Log.w(TAG, "UI thread call failed: " + e);
            return fallback;
        }
    }
}
