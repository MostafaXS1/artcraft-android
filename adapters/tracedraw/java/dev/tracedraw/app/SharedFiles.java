// The app's file provider (docs/behavior/android.md, #1398): the files the
// editor hands to another app (a print PDF for the share sheet, a picture
// for an editing app) live in the app's private folders, which no other
// app can open. This provider serves them under content URIs
// (content://dev.tracedraw.app.files/<path inside the folder>); it is not
// exported, so an app reaches a file only through the grant the intent
// that names it carries, and only files inside the app's own files and
// cache folders are served. An editing app given write access writes the
// file back through openFile, and the editor reads it again.
package dev.tracedraw.app;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.List;

public class SharedFiles extends ContentProvider {
    /** The provider's authority, as the manifest names it. */
    static final String AUTHORITY = "dev.tracedraw.app.files";

    /** The folders whose files are served: the app's files and cache folders. */
    private static File[] roots(Context context) {
        return new File[] {context.getFilesDir(), context.getCacheDir()};
    }

    /**
     * The content URI that serves `file`, or null when it is not inside
     * one of the served folders. The first path segment names the folder
     * ("files" or "cache"), the rest the path inside it.
     */
    static Uri uriFor(Context context, File file) {
        try {
            String path = file.getCanonicalPath();
            File[] roots = roots(context);
            String[] names = {"files", "cache"};
            for (int i = 0; i < roots.length; i++) {
                if (roots[i] == null) {
                    continue;
                }
                String root = roots[i].getCanonicalPath();
                if (path.startsWith(root + File.separator)) {
                    Uri.Builder b = new Uri.Builder().scheme("content").authority(AUTHORITY);
                    b.appendPath(names[i]);
                    for (String part : path.substring(root.length() + 1).split(File.separator)) {
                        if (!part.isEmpty()) {
                            b.appendPath(part);
                        }
                    }
                    return b.build();
                }
            }
        } catch (IOException e) {
            // Not a file that can be served.
        }
        return null;
    }

    /** The file `uri` names, or null when it names none inside the served folders. */
    private File fileFor(Uri uri) {
        Context context = getContext();
        List<String> parts = uri.getPathSegments();
        if (context == null || parts == null || parts.size() < 2) {
            return null;
        }
        File root;
        if ("files".equals(parts.get(0))) {
            root = context.getFilesDir();
        } else if ("cache".equals(parts.get(0))) {
            root = context.getCacheDir();
        } else {
            return null;
        }
        if (root == null) {
            return null;
        }
        File file = root;
        for (int i = 1; i < parts.size(); i++) {
            file = new File(file, parts.get(i));
        }
        try {
            String rootPath = root.getCanonicalPath();
            File canonical = file.getCanonicalFile();
            // ".." and links never lead out of the folder.
            if (!canonical.getPath().startsWith(rootPath + File.separator)) {
                return null;
            }
            return canonical;
        } catch (IOException e) {
            return null;
        }
    }

    /** The MIME type of a file name, from its extension. */
    static String mimeOf(String name) {
        int dot = name.lastIndexOf('.');
        if (dot >= 0) {
            String ext = name.substring(dot + 1).toLowerCase();
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
            if (mime != null) {
                return mime;
            }
        }
        return "application/octet-stream";
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        File file = fileFor(uri);
        return file == null ? null : mimeOf(file.getName());
    }

    /** The file's name and size, which share targets show and mail apps attach by. */
    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        File file = fileFor(uri);
        if (file == null) {
            return null;
        }
        if (projection == null) {
            projection = new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        }
        String[] columns = new String[projection.length];
        Object[] values = new Object[projection.length];
        int n = 0;
        for (String column : projection) {
            if (OpenableColumns.DISPLAY_NAME.equals(column)) {
                columns[n] = OpenableColumns.DISPLAY_NAME;
                values[n++] = file.getName();
            } else if (OpenableColumns.SIZE.equals(column)) {
                columns[n] = OpenableColumns.SIZE;
                values[n++] = Long.valueOf(file.length());
            }
        }
        String[] usedColumns = new String[n];
        Object[] usedValues = new Object[n];
        System.arraycopy(columns, 0, usedColumns, 0, n);
        System.arraycopy(values, 0, usedValues, 0, n);
        MatrixCursor cursor = new MatrixCursor(usedColumns, 1);
        cursor.addRow(usedValues);
        return cursor;
    }

    /** The file, read-only or writable as the mode asks ("r", "w", "wt", "rw", "rwt"). */
    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        File file = fileFor(uri);
        if (file == null) {
            throw new FileNotFoundException(uri.toString());
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.parseMode(mode));
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }
}
