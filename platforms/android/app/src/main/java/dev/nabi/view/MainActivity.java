// API review 2026-11-08: https://developer.android.com/training/data-storage/shared/documents-files
package dev.nabi.view;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.content.res.Configuration;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import org.json.JSONArray;
import org.json.JSONObject;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

public final class MainActivity extends NativeActivity {
    private static final int PICK_IMAGES = 100;
    private static final int PICK_FOLDER = 101;
    private static final int EXPORT_SETTINGS = 102;
    private static final int IMPORT_SETTINGS = 103;
    private static final int READ_PHOTOS = 104;
    private static final int PHOTO_SETTINGS = 105;
    private static final String BROWSE_PHOTO_FOLDERS = "nabi-photo-folders";
    private final ExecutorService imports = Executors.newSingleThreadExecutor();
    private volatile boolean pickerBusy;
    private boolean nativeReady;
    // -1 is not attached/known yet, 0 is a standalone window, 1 is windowed.
    // The sentinel ensures that the first snapshot reaches the Rust host.
    private int reportedWindowMode = Integer.MIN_VALUE;
    private final AtomicLong orientationRequest = new AtomicLong();
    private String pendingExport;
    private String pendingPhotoCollection;
    private native String nativeText(String key);
    private native void nativeBack();
    private native void nativeLocaleChanged();
    private native void nativeWindowModeChanged(int state);
    private native boolean nativeBegin();
    private native boolean nativeBeginFolder();
    private native void nativeSettings(int ownedFd);
    private native void nativeExportResult(boolean saved);
    private native boolean nativeDocument(int ownedFd, String displayName);
    private native void nativeFinish();
    private native void nativeError();
    private native void nativePhotoError(int reason);
    private native void nativeInsets(float top, float right, float bottom, float left);
    private native void nativeSource(String id, String displayName);

    // Reopen is called from Rust, while picker requests run on the UI thread.
    // Reserve both through one monitor so they cannot start together.
    private synchronized boolean reservePicker() {
        if (pickerBusy || isFinishing() || isDestroyed()) return false;
        pickerBusy = true;
        return true;
    }

    /** Advisory system memory, excluding Android's low-memory reserve. */
    public long[] memorySnapshot() {
        android.app.ActivityManager manager = getSystemService(android.app.ActivityManager.class);
        if (manager == null) return null;
        android.app.ActivityManager.MemoryInfo memory = new android.app.ActivityManager.MemoryInfo();
        manager.getMemoryInfo(memory);
        return new long[]{memory.totalMem, memory.lowMemory ? 0 : Math.max(0, memory.availMem - memory.threshold)};
    }

    /** HDR capability and current headroom for this Activity's display only.
     * A zero element means unavailable; ratio 1 is a valid lack of headroom.
     * Desired peak is content metadata, not the current peak used in the ratio.
     */
    public float[] hdrDisplay() {
        android.view.Display display = getDisplay();
        if (display == null || !display.isValid()) return null;
        android.view.Display.HdrCapabilities capabilities = display.getHdrCapabilities();
        if (capabilities == null) return null;
        int[] types = android.os.Build.VERSION.SDK_INT >= 34
            ? display.getMode().getSupportedHdrTypes() : capabilities.getSupportedHdrTypes();
        if (types == null || types.length == 0) return null;
        float peak = capabilities.getDesiredMaxLuminance();
        if (!Float.isFinite(peak) || peak <= 0) peak = 0;
        float ratio = 0;
        if (android.os.Build.VERSION.SDK_INT >= 34 && display.isHdrSdrRatioAvailable()) {
            ratio = display.getHdrSdrRatio();
            if (!Float.isFinite(ratio) || ratio < 1) ratio = 0;
        }
        return new float[]{peak, ratio};
    }

    @Override public void onCreate(Bundle state) {
        // Small bundled notices are copied before the Rust host reads them.
        try (InputStream source = getAssets().open("THIRD_PARTY_NOTICES.txt")) {
            Files.copy(source, new File(getCacheDir(), "THIRD_PARTY_NOTICES.txt").toPath(), StandardCopyOption.REPLACE_EXISTING);
        } catch (Exception ignored) { /* Viewer reports missing notices. */ }
        super.onCreate(state);
    }

    public String preferredLanguageTags() {
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            android.app.LocaleManager manager = getSystemService(android.app.LocaleManager.class);
            android.os.LocaleList appLocales = manager.getApplicationLocales();
            return (appLocales.isEmpty() ? manager.getSystemLocales() : appLocales).toLanguageTags();
        }
        return getResources().getConfiguration().getLocales().toLanguageTags();
    }
    public String savedLanguagePreference(String persisted) {
        if (android.os.Build.VERSION.SDK_INT < 33) return persisted;
        android.app.LocaleManager manager = getSystemService(android.app.LocaleManager.class);
        android.os.LocaleList locales = manager.getApplicationLocales();
        android.content.SharedPreferences state = getSharedPreferences("locale-sync", MODE_PRIVATE);
        if (!state.getBoolean("migrated", false)) {
            state.edit().putBoolean("migrated", true).apply();
            if (locales.isEmpty() && !persisted.equals("system")) {
                setApplicationLanguage(persisted);
                return persisted;
            }
        }
        return locales.isEmpty() ? "system" : locales.get(0).toLanguageTag();
    }
    public void setApplicationLanguage(String tag) {
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            runOnUiThread(() -> {
                if (isDestroyed()) return;
                android.app.LocaleManager manager = getSystemService(android.app.LocaleManager.class);
                android.os.LocaleList locales = android.os.LocaleList.forLanguageTags(tag);
                if (!manager.getApplicationLocales().equals(locales)) manager.setApplicationLocales(locales);
            });
        }
    }

    @Override public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        // The Rust UI owns translation/RTL layout. Notify its thread instead
        // of recreating NativeActivity when app or system locales change.
        if (nativeReady) nativeLocaleChanged();
        refreshWindowMode();
    }

    @Override protected void onResume() {
        super.onResume();
        // Covers an OS app-language change while this Activity was stopped.
        if (nativeReady) nativeLocaleChanged();
        refreshWindowMode();
    }

    @Override public void onMultiWindowModeChanged(boolean windowed, Configuration configuration) {
        super.onMultiWindowModeChanged(windowed, configuration);
        refreshWindowMode();
    }

    private void refreshWindowMode() {
        refreshWindowMode(getWindow().getDecorView().getRootWindowInsets());
    }

    // Activity window state owns this policy, independent of display size,
    // external display attachment, and whether the viewer hides its controls.
    private void refreshWindowMode(WindowInsets insets) {
        if (!nativeReady || isFinishing() || isDestroyed()) return;
        View decor = getWindow().getDecorView();
        int state = isInMultiWindowMode() ? 1
            : !decor.isAttachedToWindow() || insets == null ? -1
            : insets.isVisible(WindowInsets.Type.captionBar()) ? 1 : 0;
        if (state == reportedWindowMode) return;
        reportedWindowMode = state;
        // Invalidate requests queued for the previous window environment.
        orientationRequest.incrementAndGet();
        if (state != 0) applyOrientation(0);
        nativeWindowModeChanged(state);
    }

    public String storagePath(int kind) {
        return (kind == 0 ? getFilesDir() : getCacheDir()).getAbsolutePath();
    }
    public void onRustReady() {
        runOnUiThread(() -> {
            if (isFinishing() || isDestroyed()) return;
            nativeReady = true;
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, () -> nativeBack());
            }
            getWindow().getDecorView().setOnApplyWindowInsetsListener((view, windowInsets) -> {
                if (nativeReady) {
                    refreshWindowMode(windowInsets);
                    if (android.os.Build.VERSION.SDK_INT >= 30) {
                        android.graphics.Insets safe = windowInsets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                        nativeInsets(safe.top, safe.right, safe.bottom, safe.left);
                    } else {
                        nativeInsets(windowInsets.getSystemWindowInsetTop(), windowInsets.getSystemWindowInsetRight(),
                            windowInsets.getSystemWindowInsetBottom(), windowInsets.getSystemWindowInsetLeft());
                    }
                }
                return windowInsets;
            });
            getWindow().getDecorView().addOnLayoutChangeListener(
                (view, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom) -> refreshWindowMode());
            if (android.os.Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
            refreshWindowMode();
            getWindow().getDecorView().requestApplyInsets();
        });
    }
    @SuppressWarnings("deprecation")
    @Override public void onBackPressed() {
        if (nativeReady) nativeBack(); else super.onBackPressed();
    }
    public void closeViewer() { runOnUiThread(this::finish); }
    public void copyText(String text) {
        runOnUiThread(() -> {
            android.content.ClipboardManager clipboard = getSystemService(android.content.ClipboardManager.class);
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText("NABI VIEW", text));
        });
    }
    public void keepAwake(boolean enabled) {
        runOnUiThread(() -> {
            if (enabled) getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            else getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        });
    }
    /** 0 follows the system rotation setting, 1 locks portrait, 2 locks landscape. */
    public void setOrientation(int mode) {
        long request = orientationRequest.incrementAndGet();
        runOnUiThread(() -> {
            if (!nativeReady || isFinishing() || isDestroyed() || request != orientationRequest.get()) return;
            // The window may have changed while this request was in the queue.
            refreshWindowMode();
            if (request != orientationRequest.get()) return;
            applyOrientation(reportedWindowMode == 0 ? mode : 0);
        });
    }
    private void applyOrientation(int mode) {
        // A standalone "auto" explicitly restores the user's OS rotation preference.
        // UNSPECIFIED delegates to device-specific policy; retain it for OS-managed windows.
        int orientation = reportedWindowMode != 0 ? ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
                : mode == 1 ? ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
                : mode == 2 ? ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                : ActivityInfo.SCREEN_ORIENTATION_USER;
        if (getRequestedOrientation() != orientation) setRequestedOrientation(orientation);
    }
    /** Viewer full screen hides the status and navigation bars until swiped in. */
    public void setImmersive(boolean enabled) {
        runOnUiThread(() -> {
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller == null) return;
            if (enabled) {
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                controller.hide(WindowInsets.Type.systemBars());
            } else {
                controller.show(WindowInsets.Type.systemBars());
            }
        });
    }
    public void share(String path) {
        runOnUiThread(() -> {
            try {
                File file = new File(path);
                Uri uri = NabiFileProvider.uriFor(this, file);
                Intent send = new Intent(Intent.ACTION_SEND);
                send.setType(NabiFileProvider.mimeType(file.getName()));
                send.putExtra(Intent.EXTRA_STREAM, uri);
                send.setClipData(android.content.ClipData.newRawUri(file.getName(), uri));
                send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                startActivity(Intent.createChooser(send, null));
            } catch (Exception ignored) { /* No share target or the copy is gone. */ }
        });
    }
    public void pickFolder() {
        runOnUiThread(() -> {
            if (reservePicker()) showPhotoFolders();
        });
    }
    private void pickDocumentFolder() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        try { startActivityForResult(intent, PICK_FOLDER); }
        catch (Exception error) { pickerBusy = false; nativeBeginFolder(); nativeError(); nativeFinish(); }
    }
    /** Includes indexed images in Download, whose SAF root is not selectable.
     * The worker queries only MediaStore rows currently allowed by the OS. */
    private void showPhotoFolders() {
        imports.execute(() -> {
            List<PhotoFolders.Folder> folders;
            try { folders = canReadPhotos() ? PhotoFolders.list(getContentResolver()) : new ArrayList<>(); }
            catch (Exception unavailable) { folders = new ArrayList<>(); }
            final List<PhotoFolders.Folder> choices = folders;
            runOnUiThread(() -> {
                if (isDestroyed() || isFinishing()) { pickerBusy = false; return; }
                String title = nativeText("photo-folders");
                if (canReadPhotos() && !canReadAllPhotos()) title += " — " + nativeText("photo-access-limited");
                android.app.AlertDialog.Builder dialog = new android.app.AlertDialog.Builder(this)
                    .setTitle(title)
                    .setNeutralButton(nativeText("file-folder"), (d, which) -> pickDocumentFolder())
                    .setPositiveButton(nativeText("photo-access"), (d, which) -> requestPhotoAccess(BROWSE_PHOTO_FOLDERS))
                    .setNegativeButton(android.R.string.cancel, (d, which) -> pickerBusy = false)
                    .setOnCancelListener(d -> pickerBusy = false);
                if (choices.isEmpty()) {
                    dialog.setMessage(nativeText(canReadPhotos() ? "photo-collection-empty" : "photo-access-help"));
                } else {
                    String[] names = new String[choices.size()];
                    for (int i = 0; i < names.length; ++i) {
                        PhotoFolders.Folder folder = choices.get(i);
                        names[i] = folder.label() + "  (" + folder.count + ")"
                            + (folder.volume.equals(android.provider.MediaStore.VOLUME_EXTERNAL_PRIMARY) ? "" : " — " + folder.volume);
                    }
                    dialog.setItems(names, (d, which) -> imports.execute(() -> importMediaFolder(choices.get(which), null)));
                }
                dialog.show();
            });
        });
    }
    public void exportSettings(String path) {
        runOnUiThread(() -> {
            if (!reservePicker()) return;
            pendingExport = path;
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("application/json");
            intent.putExtra(Intent.EXTRA_TITLE, "nabi-view-settings.json");
            try { startActivityForResult(intent, EXPORT_SETTINGS); }
            catch (Exception error) { pickerBusy = false; pendingExport = null; nativeExportResult(false); }
        });
    }
    public void importSettings() {
        runOnUiThread(() -> {
            if (!reservePicker()) return;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            // Providers often label JSON as text or generic data.
            intent.setType("*/*");
            try { startActivityForResult(intent, IMPORT_SETTINGS); }
            catch (Exception error) { pickerBusy = false; nativeSettings(-1); }
        });
    }
    public void pickImages() {
        runOnUiThread(() -> {
            if (!reservePicker()) return;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            // Some providers do not classify JXL/AVIF/RAW as image/*.
            intent.setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            try { startActivityForResult(intent, PICK_IMAGES); }
            catch (Exception error) { pickerBusy = false; nativeBegin(); nativeError(); nativeFinish(); }
        });
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        Uri picked = result == RESULT_OK && data != null ? data.getData() : null;
        if (request == PHOTO_SETTINGS) { completePhotoAccess(); return; }
        if (request == EXPORT_SETTINGS) {
            String source = pendingExport;
            pendingExport = null;
            if (picked == null || source == null) { pickerBusy = false; return; }
            imports.execute(() -> {
                boolean saved = false;
                try (InputStream input = new FileInputStream(source);
                     OutputStream output = getContentResolver().openOutputStream(picked, "wt")) {
                    if (output == null) throw new java.io.IOException("Unavailable document");
                    byte[] buffer = new byte[8192];
                    for (int count; (count = input.read(buffer)) > 0; ) output.write(buffer, 0, count);
                    saved = true;
                } catch (Exception ignored) { /* Reported below. */ }
                finally { nativeExportResult(saved); runOnUiThread(() -> pickerBusy = false); }
            });
            return;
        }
        if (request == IMPORT_SETTINGS) {
            if (picked == null) { pickerBusy = false; return; }
            imports.execute(() -> {
                try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(picked, "r")) {
                    // Rust owns and closes the detached descriptor.
                    nativeSettings(descriptor == null ? -1 : descriptor.detachFd());
                } catch (Exception error) { nativeSettings(-1); }
                finally { runOnUiThread(() -> pickerBusy = false); }
            });
            return;
        }
        if (request == PICK_FOLDER) {
            if (picked == null) { pickerBusy = false; return; }
            int flags = data.getFlags();
            imports.execute(() -> {
                String id = rememberSource(java.util.Collections.singletonList(picked), true, flags);
                importFolder(picked, id);
            });
            return;
        }
        if (request != PICK_IMAGES) return;
        if (result != RESULT_OK || data == null) { pickerBusy = false; return; }
        List<Uri> uris = new ArrayList<>();
        if (data.getClipData() != null) {
            for (int i = 0; i < data.getClipData().getItemCount(); ++i) uris.add(data.getClipData().getItemAt(i).getUri());
        } else if (data.getData() != null) uris.add(data.getData());
        int flags = data.getFlags();
        imports.execute(() -> importPickedDocuments(uris, flags));
    }

    private String displayName(Uri uri, boolean folder) {
        Uri document = folder ? DocumentsContract.buildDocumentUriUsingTree(uri, DocumentsContract.getTreeDocumentId(uri)) : uri;
        try (Cursor cursor = getContentResolver().query(document, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && cursor.getString(0) != null) return cursor.getString(0);
        } catch (Exception ignored) { /* Stable ID remains usable without a label. */ }
        return "Selected documents";
    }

    /** Saves only OS-granted read access. URIs stay in the app's private state. */
    private String rememberSource(List<Uri> uris, boolean folder, int flags) {
        if (uris.isEmpty() || uris.size() > 128 || (flags & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) == 0) return null;
        try {
            JSONArray values = new JSONArray();
            for (Uri uri : uris) {
                if (!"content".equals(uri.getScheme())) return null;
                getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
                values.put(uri.toString());
            }
            android.content.SharedPreferences grants = getSharedPreferences("document-grants", MODE_PRIVATE);
            String identity = values.toString();
            for (java.util.Map.Entry<String, ?> entry : grants.getAll().entrySet()) {
                try {
                    JSONObject record = new JSONObject((String)entry.getValue());
                    if (record.getBoolean("folder") == folder && record.getJSONArray("uris").toString().equals(identity)) return entry.getKey();
                } catch (Exception ignored) { /* Ignore a corrupt record; do not execute it. */ }
            }
            String id = UUID.randomUUID().toString();
            JSONObject record = new JSONObject().put("folder", folder).put("uris", values).put("name", displayName(uris.get(0), folder));
            return grants.edit().putString(id, record.toString()).commit() ? id : null;
        } catch (Exception denied) { return null; /* The current, temporary grant can still be used. */ }
    }

    private void reportSource(String id, String fallback) {
        if (id == null) return;
        String name = fallback;
        try { name = new JSONObject(getSharedPreferences("document-grants", MODE_PRIVATE).getString(id, "{}")).optString("name", fallback); }
        catch (Exception ignored) { }
        nativeSource(id, name);
    }

    private void importPickedDocuments(List<Uri> uris, int flags) {
        // An explicit multi-selection remains that selection. A single image
        // expands only through existing media permission or a saved tree grant.
        if (uris.size() == 1) {
            Uri selected = uris.get(0);
            if (canReadPhotos()) {
                PhotoFolders.Selection photo = PhotoFolders.selected(this, selected);
                if (photo != null) { importMediaFolder(photo.folder, photo); return; }
            }
            try {
                String mime = getContentResolver().getType(selected);
                if (mime != null && mime.startsWith("image/") && importGrantedParent(selected)) return;
            } catch (Exception unavailable) { /* The original document remains usable. */ }
        }
        importDocuments(uris, rememberSource(uris, false, flags));
        if (uris.size() == 1) {
            try {
                String mime = getContentResolver().getType(uris.get(0));
                if (mime != null && mime.startsWith("image/")) runOnUiThread(() -> {
                    if (!isDestroyed()) android.widget.Toast.makeText(this,
                        nativeText("single-image-access"), android.widget.Toast.LENGTH_LONG).show();
                });
            } catch (Exception ignored) { /* The selected document was still opened. */ }
        }
    }

    private String rememberMediaFolder(PhotoFolders.Folder folder) {
        try {
            JSONObject record = new JSONObject().put("kind", "media-folder")
                .put("volume", folder.volume).put("path", folder.path).put("name", folder.label());
            return getSharedPreferences("document-grants", MODE_PRIVATE).edit()
                .putString(folder.id(), record.toString()).commit() ? folder.id() : null;
        } catch (Exception unavailable) { return null; }
    }
    private void importMediaFolder(PhotoFolders.Folder folder, PhotoFolders.Selection selected) {
        int imported = 0;
        boolean begun = false;
        try (Cursor cursor = PhotoFolders.images(getContentResolver(), folder)) {
            if (!nativeBeginFolder()) return;
            begun = true;
            String source = rememberMediaFolder(folder);
            if (source != null) nativeSource(source, folder.label());
            // Stage the user's chosen image first, even when it sorts later.
            if (selected != null) {
                try (ParcelFileDescriptor fd = getContentResolver().openFileDescriptor(selected.uri, "r")) {
                    if (fd != null) { nativeDocument(fd.detachFd(), selected.name); imported++; }
                }
            }
            while (cursor != null && cursor.moveToNext()) {
                long item = cursor.getLong(0);
                if (selected != null && selected.id == item) continue;
                String name = cursor.getString(1);
                if (name == null) continue;
                Uri uri = android.content.ContentUris.withAppendedId(folder.collection(), item);
                try (ParcelFileDescriptor fd = getContentResolver().openFileDescriptor(uri, "r")) {
                    if (fd == null) continue;
                    imported++;
                    if (!nativeDocument(fd.detachFd(), name)) break;
                } catch (java.io.IOException | SecurityException skipped) { /* Deleted/revoked item; keep readable neighbors. */ }
            }
            if (imported == 0) nativePhotoError(2);
        } catch (Exception unavailable) {
            if (!begun && selected != null) {
                // Losing permission to list must not lose access to the picked file.
                importDocuments(java.util.Collections.singletonList(selected.uri), null);
                return;
            }
            if (!begun) nativeBeginFolder();
            if (imported == 0) nativePhotoError(unavailable instanceof SecurityException ? 1 : 3);
        } finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
    }

    /** A single document grant never grants its parent. Ask the provider for a
     * parent only through an already persisted, readable tree for this authority. */
    private boolean importGrantedParent(Uri selected) {
        if (!DocumentsContract.isDocumentUri(this, selected)) return false;
        for (android.content.UriPermission permission : getContentResolver().getPersistedUriPermissions()) {
            Uri tree = permission.getUri();
            if (!permission.isReadPermission() || !DocumentsContract.isTreeUri(tree)
                || !java.util.Objects.equals(tree.getAuthority(), selected.getAuthority())) continue;
            try {
                String documentId = DocumentsContract.getDocumentId(selected);
                Uri withinTree = DocumentsContract.buildDocumentUriUsingTree(tree, documentId);
                DocumentsContract.Path path = DocumentsContract.findDocumentPath(getContentResolver(), withinTree);
                if (path == null || path.getPath().size() < 2) continue;
                List<String> parts = path.getPath();
                if (!documentId.equals(parts.get(parts.size() - 1))) continue;
                String parent = parts.get(parts.size() - 2);
                Uri directory = DocumentsContract.buildDocumentUriUsingTree(tree, parent);
                String name = displayName(directory, false);
                String id = UUID.nameUUIDFromBytes((tree.toString() + "\n" + parent)
                    .getBytes(java.nio.charset.StandardCharsets.UTF_8)).toString();
                JSONObject record = new JSONObject().put("folder", true).put("directory", parent)
                    .put("uris", new JSONArray().put(tree.toString())).put("name", name);
                if (!getSharedPreferences("document-grants", MODE_PRIVATE).edit().putString(id, record.toString()).commit()) id = null;
                importFolder(tree, id, parent, selected);
                return true;
            } catch (Exception unavailable) { /* This grant does not cover the selected document. */ }
        }
        return false;
    }

    private void importDocuments(List<Uri> uris, String id) {
        try {
            if (!nativeBegin()) return;
            reportSource(id, "Selected documents");
            if (uris.size() > 128) throw new java.io.IOException("Too many documents");
            for (Uri uri : uris) {
                String name = displayName(uri, false);
                try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r")) {
                    if (descriptor == null) throw new java.io.IOException("Unavailable document");
                    if (!nativeDocument(descriptor.detachFd(), name)) break;
                }
            }
        } catch (Exception error) { nativeError(); }
        finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
    }

    /** Called by Rust with an opaque local ID, never a user-supplied URI. */
    public synchronized boolean reopenSource(String id) {
        if (!id.matches("[A-Za-z0-9-]{8,64}") || !reservePicker()) return false;
        if (isPhotoCollection(id)) {
            runOnUiThread(() -> openPhotoCollection(id));
            return true;
        }
        imports.execute(() -> {
            try {
                String saved = getSharedPreferences("document-grants", MODE_PRIVATE).getString(id, null);
                if (saved == null || saved.length() > 1024 * 1024) throw new java.io.IOException("Grant unavailable");
                JSONObject record = new JSONObject(saved);
                if ("media-folder".equals(record.optString("kind"))) {
                    PhotoFolders.Folder folder = new PhotoFolders.Folder(record.getString("volume"), record.getString("path"));
                    if (!folder.id().equals(id)) throw new java.io.IOException("Invalid photo folder");
                    if (!canReadPhotos()) {
                        runOnUiThread(() -> requestPhotoAccess(id));
                    } else importMediaFolder(folder, null);
                    return;
                }
                JSONArray values = record.getJSONArray("uris");
                if (values.length() == 0 || values.length() > 128) throw new java.io.IOException("Invalid grant");
                List<Uri> uris = new ArrayList<>();
                for (int i = 0; i < values.length(); ++i) {
                    Uri uri = Uri.parse(values.getString(i));
                    if (!"content".equals(uri.getScheme())) throw new java.io.IOException("Invalid grant");
                    uris.add(uri);
                }
                if (record.getBoolean("folder")) importFolder(uris.get(0), id,
                    record.optString("directory", DocumentsContract.getTreeDocumentId(uris.get(0))), null);
                else importDocuments(uris, id);
            } catch (Exception denied) {
                nativeBegin(); nativeError(); nativeFinish();
                runOnUiThread(() -> pickerBusy = false);
            }
        });
        return true;
    }

    private static boolean isPhotoCollection(String id) {
        return id.equals("nabi-android-camera") || id.equals("nabi-android-pictures")
            || id.equals("nabi-android-screenshots");
    }

    private boolean canReadAllPhotos() {
        String permission = android.os.Build.VERSION.SDK_INT >= 33 ? android.Manifest.permission.READ_MEDIA_IMAGES
            : android.Manifest.permission.READ_EXTERNAL_STORAGE;
        return checkSelfPermission(permission) == android.content.pm.PackageManager.PERMISSION_GRANTED;
    }
    private boolean canReadPhotos() {
        return canReadAllPhotos() || (android.os.Build.VERSION.SDK_INT >= 34
            && checkSelfPermission(android.Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
                == android.content.pm.PackageManager.PERMISSION_GRANTED);
    }
    private void failPhotoCollection(int reason) {
        nativeBeginFolder(); nativePhotoError(reason); nativeFinish();
        pickerBusy = false;
    }
    private void openPhotoCollection(String id) {
        if (canReadPhotos()) imports.execute(() -> importPhotoCollection(id));
        else requestPhotoAccess(id);
    }
    /** Called only from a user action, never repeatedly from onResume. */
    private void requestPhotoAccess(String destination) {
        pendingPhotoCollection = destination;
        int api = android.os.Build.VERSION.SDK_INT;
        String full = api >= 33 ? android.Manifest.permission.READ_MEDIA_IMAGES : android.Manifest.permission.READ_EXTERNAL_STORAGE;
        android.content.SharedPreferences state = getSharedPreferences("photo-access", MODE_PRIVATE);
        boolean deniedPermanently = !canReadPhotos() && state.getBoolean("requested", false)
            && !shouldShowRequestPermissionRationale(full);
        if (canReadAllPhotos() || deniedPermanently) {
            Intent settings = new Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", getPackageName(), null));
            try { startActivityForResult(settings, PHOTO_SETTINGS); }
            catch (Exception unavailable) { pendingPhotoCollection = null; failPhotoCollection(1); }
            return;
        }
        String[] permissions = api >= 34 ? new String[]{full, android.Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED}
            : new String[]{full};
        state.edit().putBoolean("requested", true).apply();
        try { requestPermissions(permissions, READ_PHOTOS); }
        catch (Exception error) { pendingPhotoCollection = null; failPhotoCollection(1); }
    }
    private void completePhotoAccess() {
        String id = pendingPhotoCollection;
        pendingPhotoCollection = null;
        if (id == null) { pickerBusy = false; return; }
        if (BROWSE_PHOTO_FOLDERS.equals(id)) showPhotoFolders();
        else if (!canReadPhotos()) failPhotoCollection(1);
        else if (isPhotoCollection(id)) imports.execute(() -> importPhotoCollection(id));
        else { pickerBusy = false; reopenSource(id); }
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == READ_PHOTOS) completePhotoAccess();
    }

    /** Query only OS-authorized images, including Android 14 limited selections.
     * Paths are MediaStore metadata filters, never filesystem access grants. */
    private void importPhotoCollection(String id) {
        try {
            if (!nativeBeginFolder()) return;
            nativeSource(id, "Photos");
            String relative = android.provider.MediaStore.Images.Media.RELATIVE_PATH;
            String filter;
            String[] values;
            if (id.equals("nabi-android-camera")) {
                filter = relative + " LIKE ?";
                values = new String[]{"DCIM/Camera/%"};
            } else if (id.equals("nabi-android-screenshots")) {
                filter = "(" + relative + " LIKE ? OR " + relative + " LIKE ?)";
                values = new String[]{"Pictures/Screenshots/%", "DCIM/Screenshots/%"};
            } else {
                filter = relative + " LIKE ?";
                values = new String[]{"Pictures/%"};
            }
            Uri collection = android.provider.MediaStore.Images.Media.getContentUri(android.provider.MediaStore.VOLUME_EXTERNAL);
            String[] columns = {android.provider.MediaStore.Images.Media._ID,
                android.provider.MediaStore.Images.Media.DISPLAY_NAME};
            // No unbounded in-memory list; Rust enforces per-file/batch limits.
            try (Cursor cursor = getContentResolver().query(collection, columns, filter, values,
                    android.provider.MediaStore.Images.Media.DATE_ADDED + " DESC, _id DESC")) {
                int count = 0;
                while (cursor != null && cursor.moveToNext()) {
                    Uri uri = android.content.ContentUris.withAppendedId(collection, cursor.getLong(0));
                    String name = cursor.getString(1);
                    if (name == null) continue;
                    // Stable unique names prevent duplicate camera filenames colliding.
                    name = cursor.getLong(0) + "-" + name;
                    try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r")) {
                        if (descriptor == null) continue;
                        count++;
                        if (!nativeDocument(descriptor.detachFd(), name)) break;
                    } catch (java.io.IOException skipped) { /* A removed item is skipped. */ }
                }
                if (count == 0) nativePhotoError(2);
            }
        } catch (SecurityException denied) { nativePhotoError(1); }
        catch (Exception error) { nativePhotoError(3); }
        finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
    }
    /** Imports only this directory, using the actual granted tree for every URI. */
    private void importFolder(Uri tree, String id) {
        importFolder(tree, id, DocumentsContract.getTreeDocumentId(tree), null);
    }
    private void importFolder(Uri tree, String id, String directory, Uri selected) {
        boolean selectedImported = false;
        try {
            if (!nativeBeginFolder()) return;
            reportSource(id, displayName(DocumentsContract.buildDocumentUriUsingTree(tree, directory), false));
            String selectedId = selected == null ? null : DocumentsContract.getDocumentId(selected);
            if (selected != null) {
                try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(selected, "r")) {
                    if (descriptor != null) {
                        nativeDocument(descriptor.detachFd(), displayName(selected, false));
                        selectedImported = true;
                    }
                }
            }
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, directory);
            try (Cursor cursor = getContentResolver().query(children, new String[]{
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                    DocumentsContract.Document.COLUMN_MIME_TYPE}, null, null, null)) {
                if (cursor == null) throw new java.io.IOException("Folder listing unavailable");
                while (cursor.moveToNext()) {
                    String document = cursor.getString(0), name = cursor.getString(1);
                    if (DocumentsContract.Document.MIME_TYPE_DIR.equals(cursor.getString(2))
                        || name == null || java.util.Objects.equals(document, selectedId)) continue;
                    Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, document);
                    try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r")) {
                        if (descriptor == null) continue;
                        if (!nativeDocument(descriptor.detachFd(), name)) break;
                    } catch (java.io.IOException | SecurityException skipped) { /* Keep readable images. */ }
                }
            }
        } catch (Exception error) {
            if (selected != null && !selectedImported) {
                importDocuments(java.util.Collections.singletonList(selected), null);
            } else if (!selectedImported) nativeError();
            // Preserve the explicitly selected image if the parent grant was
            // revoked while listing its neighbors.
        } finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
    }
    @Override protected void onDestroy() {
        nativeReady = false;
        orientationRequest.incrementAndGet();
        imports.shutdown();
        super.onDestroy();
    }
}
