// API review 2026-11-09: https://developer.android.com/training/secure-file-sharing/setup-sharing
package dev.nabi.view;

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
import java.util.Locale;

/** Read-only access to the viewer's own cached copies, granted per share intent. */
public final class NabiFileProvider extends ContentProvider {
    static Uri uriFor(Context context, File file) throws IOException {
        String root = context.getCacheDir().getCanonicalPath() + File.separator;
        String path = file.getCanonicalPath();
        if (!path.startsWith(root)) throw new IOException("Only cached copies can be shared");
        return new Uri.Builder().scheme("content").authority(context.getPackageName() + ".files")
            .encodedPath("/" + Uri.encode(path.substring(root.length()), "/")).build();
    }

    static String mimeType(String name) {
        int dot = name.lastIndexOf('.');
        String type = dot < 0 ? null
            : MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substring(dot + 1).toLowerCase(Locale.ROOT));
        return type != null ? type : "application/octet-stream";
    }

    private File file(Uri uri) throws FileNotFoundException {
        try {
            File root = getContext().getCacheDir().getCanonicalFile();
            String path = uri.getPath();
            File file = new File(root, path == null ? "" : path).getCanonicalFile();
            if (!file.getPath().startsWith(root.getPath() + File.separator) || !file.isFile()) {
                throw new FileNotFoundException(uri.toString());
            }
            return file;
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
    }

    @Override public boolean onCreate() { return true; }

    @Override public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        return mimeType(name == null ? "" : name);
    }

    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) throw new SecurityException("Shared images are read-only");
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] args, String order) {
        File file;
        try { file = file(uri); } catch (FileNotFoundException error) { return null; }
        String[] columns = projection != null ? projection : new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor cursor = new MatrixCursor(columns, 1);
        Object[] row = new Object[columns.length];
        for (int i = 0; i < columns.length; ++i) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[i])) row[i] = file.getName();
            else if (OpenableColumns.SIZE.equals(columns[i])) row[i] = file.length();
        }
        cursor.addRow(row);
        return cursor;
    }

    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String selection, String[] args) { return 0; }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] args) { return 0; }
}
