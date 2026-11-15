// API review 2026-11-15: https://developer.android.com/training/data-storage/shared/media
package dev.nabi.view;

import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructStat;
import android.provider.MediaStore;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.UUID;

/** MediaStore folders are photo collections, not filesystem or SAF grants. */
final class PhotoFolders {
    static final String VOLUME = MediaStore.Images.Media.VOLUME_NAME;
    static final String PATH = MediaStore.Images.Media.RELATIVE_PATH;
    static final String NAME = MediaStore.Images.Media.DISPLAY_NAME;
    static final String ID = MediaStore.Images.Media._ID;
    static final Uri IMAGES = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL);

    static final class Folder {
        final String volume, path;
        int count;
        Folder(String volume, String path) { this.volume = volume; this.path = path; }
        String id() {
            return UUID.nameUUIDFromBytes(("media-folder\n" + volume + "\n" + path)
                .getBytes(StandardCharsets.UTF_8)).toString();
        }
        String label() { return path.isEmpty() ? "/" : path; }
        Uri collection() { return MediaStore.Images.Media.getContentUri(volume); }
    }
    static final class Selection {
        final Folder folder;
        final Uri uri;
        final long id;
        final String name;
        Selection(Folder folder, Uri uri, long id, String name) {
            this.folder = folder; this.uri = uri; this.id = id; this.name = name;
        }
    }
    static List<Folder> list(ContentResolver resolver) {
        LinkedHashMap<String, Folder> folders = new LinkedHashMap<>();
        try (Cursor cursor = resolver.query(IMAGES, new String[]{VOLUME, PATH}, null, null, null)) {
            while (cursor != null && cursor.moveToNext()) {
                String volume = cursor.getString(0), path = cursor.getString(1);
                if (volume == null || path == null) continue;
                String key = volume + "\n" + path;
                Folder folder = folders.get(key);
                if (folder == null) {
                    if (folders.size() >= 4096) continue;
                    folder = new Folder(volume, path);
                    folders.put(key, folder);
                }
                folder.count++;
            }
        }
        List<Folder> result = new ArrayList<>(folders.values());
        result.sort((a, b) -> {
            int path = a.path.compareToIgnoreCase(b.path);
            return path == 0 ? a.volume.compareTo(b.volume) : path;
        });
        return result;
    }
    static Cursor images(ContentResolver resolver, Folder folder) {
        return resolver.query(folder.collection(), new String[]{ID, NAME}, PATH + " = ?",
            new String[]{folder.path}, NAME + " ASC, " + ID + " ASC");
    }
    /** Only metadata from the granted URI or Android's equivalent MediaStore URI.
     * Never guess a folder from a display filename or parse opaque document IDs. */
    static Selection selected(Context context, Uri picked) {
        try {
            Uri media = null;
            try { media = "media".equals(picked.getAuthority()) ? picked : MediaStore.getMediaUri(context, picked); }
            catch (Exception unsupportedProvider) { /* Downloads and third-party document providers. */ }
            if (media == null) return sameLocalImage(context, picked);
            try (Cursor cursor = context.getContentResolver().query(media,
                    new String[]{ID, NAME, VOLUME, PATH, MediaStore.MediaColumns.MIME_TYPE}, null, null, null)) {
                if (cursor == null || !cursor.moveToFirst()) return null;
                String name = cursor.getString(1), volume = cursor.getString(2), path = cursor.getString(3);
                String mime = cursor.getString(4);
                if (name == null || volume == null || path == null || mime == null || !mime.startsWith("image/")) return null;
                Folder folder = new Folder(volume, path);
                long id = cursor.getLong(0);
                return new Selection(folder, ContentUris.withAppendedId(folder.collection(), id), id, name);
            }
        } catch (Exception unavailable) { return null; }
    }
    /** Some providers cannot map their document URI to MediaStore. A filename
     * narrows the query but never establishes identity: require the same open
     * regular-file device/inode through two independently authorized URIs.
     * Pipes, cloud streams and different mount identities simply fall back. */
    private static Selection sameLocalImage(Context context, Uri picked) {
        ContentResolver resolver = context.getContentResolver();
        try {
            String mime = resolver.getType(picked);
            if (mime == null || !mime.startsWith("image/")) return null;
            String name;
            try (Cursor cursor = resolver.query(picked, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
                if (cursor == null || !cursor.moveToFirst() || cursor.isNull(0)) return null;
                name = cursor.getString(0);
            }
            try (ParcelFileDescriptor original = resolver.openFileDescriptor(picked, "r")) {
                if (original == null) return null;
                StructStat stat = Os.fstat(original.getFileDescriptor());
                if (!OsConstants.S_ISREG(stat.st_mode) || stat.st_ino == 0) return null;
                try (Cursor rows = resolver.query(IMAGES, new String[]{ID, NAME, VOLUME, PATH},
                        NAME + " = ? AND " + MediaStore.MediaColumns.SIZE + " = ?",
                        new String[]{name, Long.toString(stat.st_size)}, ID + " DESC")) {
                    int checked = 0;
                    while (rows != null && rows.moveToNext() && checked++ < 64) {
                        if (rows.isNull(2) || rows.isNull(3)) continue;
                        Folder folder = new Folder(rows.getString(2), rows.getString(3));
                        long id = rows.getLong(0);
                        Uri candidate = ContentUris.withAppendedId(folder.collection(), id);
                        try (ParcelFileDescriptor file = resolver.openFileDescriptor(candidate, "r")) {
                            if (file == null) continue;
                            StructStat other = Os.fstat(file.getFileDescriptor());
                            if (stat.st_dev == other.st_dev && stat.st_ino == other.st_ino)
                                return new Selection(folder, picked, id, name);
                        } catch (Exception unavailable) { /* No access or file removed. */ }
                    }
                }
            }
        } catch (Exception unavailable) { /* Keep the original single-document grant. */ }
        return null;
    }
    private PhotoFolders() {}
}
