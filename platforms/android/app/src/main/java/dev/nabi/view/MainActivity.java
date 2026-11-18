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
    private volatile boolean nativeReady;
    // -1 is not attached/known yet, 0 is a standalone window, 1 is windowed.
    // The sentinel ensures that the first snapshot reaches the Rust host.
    private int reportedWindowMode = Integer.MIN_VALUE;
    private final AtomicLong orientationRequest = new AtomicLong();
    private String pendingExport;
    private String pendingPhotoCollection;
    // UI-thread-owned dialog and query. Every result belongs to both identities.
    private PhotoFolderDialog photoFolderDialog;
    private android.os.CancellationSignal photoFolderQuery;
    // Only the currently owned, fully authorized MediaStore batch is reusable.
    // Epochs invalidate imports spanning a lifecycle, permission or media change.
    private final AtomicLong sourceEpoch = new AtomicLong(1);
    private static final AtomicLong sourceTokens = new AtomicLong();
    private volatile boolean sourceResumed;
    private volatile boolean sourceObserverReady;
    private volatile MediaSourceSnapshot mediaSnapshot;
    private final android.database.ContentObserver sourceObserver = new android.database.ContentObserver(null) {
        @Override public void onChange(boolean selfChange) { invalidateSourceCache(); }
        @Override public void onChange(boolean selfChange, Uri uri) { invalidateSourceCache(); }
    };
    private native String nativeText(String key);
    private native boolean nativeIsRtl();
    private native void nativeRequestStarted();
    private native void nativeImportPending();
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
    private native void nativeSourceSnapshot(long token);
    private native boolean nativeSourceReused(long token, long epoch);
    private native boolean nativeDocumentDeferred(String displayName, String token, long bytes,
        long modifiedMillis, int kind);
    private native void nativeMediaAccessChanged();
    // Source kinds for nativeDocumentDeferred/openSource; must match android_media.rs.
    private static final int KIND_MEDIA = 1, KIND_DOCUMENT = 2, KIND_MEDIA_PARTIAL = 3;
    private boolean fullPhotoAccess;
    private long pickerRequest;

    // Reopen is called from Rust, while picker requests run on the UI thread.
    // Reserve both through one monitor so they cannot start together.
    private synchronized boolean reservePicker() {
        if (pickerBusy || !nativeReady || isFinishing() || isDestroyed()) return false;
        pickerBusy = true;
        ++pickerRequest;
        // Only a successful reservation supersedes a return to home. A busy
        // worker keeps its reservation until its final callback has drained.
        nativeRequestStarted();
        return true;
    }

    /** Fallback imports can nest. Release only after their outer worker has
     * drained every native callback, and never release a newer reservation. */
    private synchronized void finishPickerWorker() {
        long request = pickerRequest;
        try {
            imports.execute(() -> runOnUiThread(() -> {
                synchronized (MainActivity.this) {
                    if (pickerRequest == request) pickerBusy = false;
                }
            }));
        } catch (java.util.concurrent.RejectedExecutionException destroyed) {
            // onDestroy already released the reservation and native host.
        }
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
        if (photoFolderDialog != null && photoFolderDialog.isShowing()) {
            photoFolderDialog.refreshConfiguration();
        }
    }

    @Override protected void onResume() {
        super.onResume();
        invalidateSourceCache();
        sourceResumed = true;
        observeMediaSources();
        reportPhotoAccess();
        // Covers an OS app-language change while this Activity was stopped.
        if (nativeReady) nativeLocaleChanged();
        refreshWindowMode();
        if (photoFolderDialog != null && photoFolderDialog.isShowing()) loadPhotoFolders(photoFolderDialog);
    }

    @Override protected void onPause() {
        sourceResumed = false;
        invalidateSourceCache();
        if (photoFolderDialog != null) {
            cancelPhotoFolderQuery();
            photoFolderDialog.setLoading();
        }
        super.onPause();
    }

    private void invalidateSourceCache() {
        long epoch = sourceEpoch.incrementAndGet();
        mediaSnapshot = null;
        runOnUiThread(() -> {
            if (sourceEpoch.get() == epoch && sourceResumed && nativeReady
                    && photoFolderDialog != null && photoFolderDialog.isShowing()) {
                loadPhotoFolders(photoFolderDialog);
            }
        });
    }

    private synchronized void observeMediaSources() {
        if (sourceObserverReady || !nativeReady || isDestroyed()) return;
        try {
            // Observe the authority so concrete and synthetic external-volume
            // notifications both invalidate the same conservative epoch.
            getContentResolver().registerContentObserver(Uri.parse("content://media"), true, sourceObserver);
            sourceObserverReady = true;
        } catch (Exception unavailable) { sourceObserverReady = false; }
    }

    /** Fast final guard after a worker has validated MediaStore generations. */
    public long sourceReuseEpoch() {
        return sourceResumed && sourceObserverReady && nativeReady && !isDestroyed() && canReadAllPhotos()
            ? sourceEpoch.get() : 0;
    }

    private java.util.Map<String, MediaSourceSnapshot.Volume> mediaVersions(String volume) {
        java.util.Set<String> mounted = android.provider.MediaStore.getExternalVolumeNames(this);
        java.util.Set<String> selected = volume == null ? mounted : java.util.Collections.singleton(volume);
        if (selected.isEmpty() || !mounted.containsAll(selected)) throw new IllegalStateException("Media volume unavailable");
        java.util.Map<String, MediaSourceSnapshot.Volume> result = new java.util.TreeMap<>();
        for (String name : selected) result.put(name, new MediaSourceSnapshot.Volume(
            android.provider.MediaStore.getVersion(this, name), android.provider.MediaStore.getGeneration(this, name)));
        return result;
    }

    private MediaSourceSnapshot captureMediaSnapshot(String source, String volume) {
        mediaSnapshot = null;
        long epoch = sourceReuseEpoch();
        if (epoch == 0) return null;
        try {
            java.util.Map<String, MediaSourceSnapshot.Volume> versions = mediaVersions(volume);
            if (sourceReuseEpoch() != epoch) return null;
            return new MediaSourceSnapshot(source, volume, sourceTokens.incrementAndGet(), epoch, versions);
        } catch (Exception unavailable) { return null; }
    }

    private void finishMediaSnapshot(MediaSourceSnapshot snapshot, boolean complete) {
        if (snapshot == null || !complete) return;
        try {
            java.util.Map<String, MediaSourceSnapshot.Volume> versions = mediaVersions(snapshot.volume);
            long epoch = sourceReuseEpoch();
            if (!snapshot.matchesContent(epoch, epoch != 0, versions)) return;
            mediaSnapshot = snapshot;
            // Keep the starting epoch: changes during import never bless old pixels.
            nativeSourceSnapshot(snapshot.token);
        } catch (Exception unavailable) { /* Successful imports still open without reuse. */ }
    }

    private boolean reuseMediaSource(String source, long token, String volume) {
        MediaSourceSnapshot snapshot = mediaSnapshot;
        if (token == 0 || snapshot == null || !java.util.Objects.equals(snapshot.volume, volume)) return false;
        try {
            java.util.Map<String, MediaSourceSnapshot.Volume> versions = mediaVersions(volume);
            long epoch = sourceReuseEpoch();
            if (!snapshot.matches(source, token, epoch, epoch != 0, versions)) return false;
            synchronized (this) {
                if (sourceReuseEpoch() != epoch) return false;
                // The Rust UI rechecks epoch and batch identity before reusing.
                // Its fallback request waits for this monitor to release busy.
                if (!nativeSourceReused(token, epoch)) return false;
                pickerBusy = false;
            }
            return true;
        } catch (Exception unavailable) { return false; }
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
            observeMediaSources();
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
        catch (Exception error) { nativeBeginFolder(); nativeError(); nativeFinish(); pickerBusy = false; }
    }
    /** Includes indexed images in Download, whose SAF root is not selectable.
     * The worker queries only MediaStore rows currently allowed by the OS. */
    private void showPhotoFolders() {
        if (!nativeReady || isDestroyed() || isFinishing()) { pickerBusy = false; return; }
        cancelPhotoFolderQuery();
        if (photoFolderDialog != null) photoFolderDialog.dismissSilently();
        class FolderActions implements PhotoFolderDialog.Listener {
            PhotoFolderDialog dialog;
            @Override public void onFolder(PhotoFolders.Folder folder) {
                if (!takePhotoFolderDialog(dialog)) return;
                nativeImportPending();
                imports.execute(() -> {
                    if (nativeReady) importMediaFolder(folder, null);
                });
            }
            @Override public void onFiles() {
                if (takePhotoFolderDialog(dialog)) pickDocumentFolder();
            }
            @Override public void onAccess() {
                if (takePhotoFolderDialog(dialog)) requestPhotoAccess(BROWSE_PHOTO_FOLDERS);
            }
            @Override public void onCancel() {
                if (takePhotoFolderDialog(dialog)) pickerBusy = false;
            }
            @Override public void onRetry() {
                if (photoFolderDialog == dialog && dialog.isShowing()) loadPhotoFolders(dialog);
            }
        }
        FolderActions actions = new FolderActions();
        PhotoFolderDialog dialog = new PhotoFolderDialog(this, this::nativeText, this::nativeIsRtl, actions);
        actions.dialog = dialog;
        photoFolderDialog = dialog;
        dialog.show();
        loadPhotoFolders(dialog);
    }

    /** Dismissed handoffs retain the picker reservation for the next operation. */
    private boolean takePhotoFolderDialog(PhotoFolderDialog dialog) {
        if (photoFolderDialog != dialog) return false;
        photoFolderDialog = null;
        cancelPhotoFolderQuery();
        return nativeReady && !isDestroyed() && !isFinishing();
    }

    private void cancelPhotoFolderQuery() {
        android.os.CancellationSignal query = photoFolderQuery;
        photoFolderQuery = null;
        if (query != null) query.cancel();
    }

    private void loadPhotoFolders(PhotoFolderDialog dialog) {
        if (photoFolderDialog != dialog || !dialog.isShowing() || !nativeReady
                || isDestroyed() || isFinishing()) return;
        cancelPhotoFolderQuery();
        dialog.setLoading();
        android.os.CancellationSignal query = new android.os.CancellationSignal();
        photoFolderQuery = query;
        long epoch = sourceEpoch.get();
        boolean readable = canReadPhotos(), fullAccess = canReadAllPhotos();
        imports.execute(() -> {
            List<PhotoFolders.Folder> folders = null;
            try {
                query.throwIfCanceled();
                folders = readable ? PhotoFolders.list(getContentResolver(), query) : new ArrayList<>();
            } catch (android.os.OperationCanceledException cancelled) {
                if (query.isCanceled()) return;
            } catch (Exception unavailable) { /* A failed query is distinct from an empty folder list. */ }
            final List<PhotoFolders.Folder> choices = folders;
            runOnUiThread(() -> {
                if (photoFolderDialog != dialog || photoFolderQuery != query || query.isCanceled()
                        || !dialog.isShowing() || !nativeReady || isDestroyed() || isFinishing()) return;
                photoFolderQuery = null;
                if (sourceEpoch.get() != epoch || readable != canReadPhotos() || fullAccess != canReadAllPhotos()) {
                    if (sourceResumed) loadPhotoFolders(dialog);
                    return;
                }
                if (choices == null) dialog.setFailure(readable, fullAccess);
                else dialog.setFolders(choices, readable, fullAccess);
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
            catch (Exception error) { nativeBegin(); nativeError(); nativeFinish(); pickerBusy = false; }
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
                finally { nativeExportResult(saved); finishPickerWorker(); }
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
                finally { finishPickerWorker(); }
            });
            return;
        }
        if (request == PICK_FOLDER) {
            if (picked == null) { pickerBusy = false; return; }
            nativeImportPending();
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
        if (!uris.isEmpty()) nativeImportPending();
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
        boolean reusable = true;
        MediaSourceSnapshot snapshot = captureMediaSnapshot(folder.id(), folder.volume);
        try (Cursor cursor = PhotoFolders.images(getContentResolver(), folder)) {
            if (cursor == null) reusable = false;
            if (!nativeBeginFolder()) return;
            begun = true;
            String source = rememberMediaFolder(folder);
            if (source != null) nativeSource(source, folder.label());
            // Stage the user's chosen image first, even when it sorts later.
            if (selected != null) {
                try (ParcelFileDescriptor fd = getContentResolver().openFileDescriptor(selected.uri, "r")) {
                    if (fd != null && selected.name != null) {
                        if (!nativeDocument(fd.detachFd(), selected.name)) reusable = false;
                        imported++;
                    } else reusable = false;
                }
            }
            int kind = mediaKind();
            while (cursor != null && cursor.moveToNext()) {
                long item = cursor.getLong(0);
                if (selected != null && selected.id == item) continue;
                String name = cursor.getString(1);
                if (name == null) { reusable = false; continue; }
                // Metadata only: Rust opens the item through openSource when it
                // first needs pixels. DATE_MODIFIED is in seconds.
                imported++;
                if (!nativeDocumentDeferred(name, folder.volume + "\n" + item,
                        cursor.isNull(2) ? 0 : cursor.getLong(2),
                        cursor.isNull(3) ? 0 : cursor.getLong(3) * 1000, kind)) { reusable = false; break; }
            }
            if (imported == 0) nativePhotoError(2);
        } catch (Exception unavailable) {
            reusable = false;
            if (!begun && selected != null) {
                // Losing permission to list must not lose access to the picked file.
                importDocuments(java.util.Collections.singletonList(selected.uri), null);
                return;
            }
            if (!begun) nativeBeginFolder();
            if (imported == 0) nativePhotoError(unavailable instanceof SecurityException ? 1 : 3);
        } finally {
            finishMediaSnapshot(snapshot, reusable && begun && imported > 0);
            nativeFinish(); finishPickerWorker();
        }
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
        mediaSnapshot = null;
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
        finally { nativeFinish(); finishPickerWorker(); }
    }

    /** Called by Rust with an opaque local ID, never a user-supplied URI. */
    public boolean reopenSource(String id) { return reopenSourceCached(id, 0); }

    /** A token only proposes reuse of Rust's still-owned batch; it grants no access. */
    public synchronized boolean reopenSourceCached(String id, long token) {
        if (!id.matches("[A-Za-z0-9-]{8,64}") || !reservePicker()) return false;
        return reopenReservedSource(id, token);
    }

    /** Permission handoffs keep the original request, including cancellation. */
    private boolean reopenReservedSource(String id, long token) {
        if (isPhotoCollection(id)) {
            if (canReadAllPhotos()) {
                nativeImportPending();
                imports.execute(() -> {
                    if (!reuseMediaSource(id, token, null)) importPhotoCollection(id);
                });
            } else runOnUiThread(() -> openPhotoCollection(id));
            return true;
        }
        nativeImportPending();
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
                    } else if (!reuseMediaSource(id, token, folder.volume)) importMediaFolder(folder, null);
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
                finishPickerWorker();
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
    /** Losing full photo access immediately invalidates listed MediaStore
     * identities; gaining it needs nothing, since the next listing reports it. */
    private void reportPhotoAccess() {
        boolean full = canReadAllPhotos();
        if (nativeReady && fullPhotoAccess && !full) nativeMediaAccessChanged();
        fullPhotoAccess = full;
    }
    private int mediaKind() { return canReadAllPhotos() ? KIND_MEDIA : KIND_MEDIA_PARTIAL; }
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
        reportPhotoAccess();
        invalidateSourceCache();
        observeMediaSources();
        String id = pendingPhotoCollection;
        pendingPhotoCollection = null;
        if (id == null) { pickerBusy = false; return; }
        if (BROWSE_PHOTO_FOLDERS.equals(id)) showPhotoFolders();
        else if (!canReadPhotos()) failPhotoCollection(1);
        else if (isPhotoCollection(id)) imports.execute(() -> importPhotoCollection(id));
        else reopenReservedSource(id, 0);
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == READ_PHOTOS) completePhotoAccess();
    }

    /** Query only OS-authorized images, including Android 14 limited selections.
     * Paths are MediaStore metadata filters, never filesystem access grants. */
    private void importPhotoCollection(String id) {
        int count = 0;
        boolean reusable = true;
        MediaSourceSnapshot snapshot = captureMediaSnapshot(id, null);
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
                android.provider.MediaStore.Images.Media.DISPLAY_NAME,
                android.provider.MediaStore.MediaColumns.SIZE,
                android.provider.MediaStore.MediaColumns.DATE_MODIFIED,
                android.provider.MediaStore.MediaColumns.VOLUME_NAME};
            int kind = mediaKind();
            // No unbounded in-memory list; Rust enforces per-file/batch limits.
            try (Cursor cursor = getContentResolver().query(collection, columns, filter, values,
                    android.provider.MediaStore.Images.Media.DATE_ADDED + " DESC, _id DESC")) {
                while (cursor != null && cursor.moveToNext()) {
                    String name = cursor.getString(1);
                    if (name == null) { reusable = false; continue; }
                    long item = cursor.getLong(0);
                    // Stable unique names prevent duplicate camera filenames colliding.
                    name = item + "-" + name;
                    String volume = cursor.isNull(4)
                        ? android.provider.MediaStore.VOLUME_EXTERNAL : cursor.getString(4);
                    count++;
                    if (!nativeDocumentDeferred(name, volume + "\n" + item,
                            cursor.isNull(2) ? 0 : cursor.getLong(2),
                            cursor.isNull(3) ? 0 : cursor.getLong(3) * 1000, kind)) { reusable = false; break; }
                }
                if (count == 0) nativePhotoError(2);
            }
        } catch (SecurityException denied) { reusable = false; nativePhotoError(1); }
        catch (Exception error) { reusable = false; nativePhotoError(3); }
        finally {
            finishMediaSnapshot(snapshot, reusable && count > 0);
            nativeFinish(); finishPickerWorker();
        }
    }
    /** Imports only this directory, using the actual granted tree for every URI. */
    private void importFolder(Uri tree, String id) {
        importFolder(tree, id, DocumentsContract.getTreeDocumentId(tree), null);
    }
    private void importFolder(Uri tree, String id, String directory, Uri selected) {
        mediaSnapshot = null; // SAF providers have no universal reliable generation stamp.
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
                    DocumentsContract.Document.COLUMN_MIME_TYPE,
                    DocumentsContract.Document.COLUMN_SIZE,
                    DocumentsContract.Document.COLUMN_LAST_MODIFIED}, null, null, null)) {
                if (cursor == null) throw new java.io.IOException("Folder listing unavailable");
                while (cursor.moveToNext()) {
                    String document = cursor.getString(0), name = cursor.getString(1);
                    if (DocumentsContract.Document.MIME_TYPE_DIR.equals(cursor.getString(2))
                        || name == null || java.util.Objects.equals(document, selectedId)) continue;
                    Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, document);
                    // Metadata only; the tree grant is checked again when opened.
                    if (!nativeDocumentDeferred(name, uri.toString(),
                            cursor.isNull(3) ? 0 : cursor.getLong(3),
                            cursor.isNull(4) ? 0 : cursor.getLong(4), KIND_DOCUMENT)) break;
                }
            }
        } catch (Exception error) {
            if (selected != null && !selectedImported) {
                importDocuments(java.util.Collections.singletonList(selected), null);
            } else if (!selectedImported) nativeError();
            // Preserve the explicitly selected image if the parent grant was
            // revoked while listing its neighbors.
        } finally { nativeFinish(); finishPickerWorker(); }
    }
    private static Uri sourceUri(int kind, String token) {
        if (token == null) return null;
        if (kind == KIND_MEDIA || kind == KIND_MEDIA_PARTIAL) {
            int split = token.indexOf('\n');
            if (split <= 0) return null;
            long id = Long.parseLong(token.substring(split + 1));
            return android.content.ContentUris.withAppendedId(
                android.provider.MediaStore.Images.Media.getContentUri(token.substring(0, split)), id);
        }
        if (kind == KIND_DOCUMENT) {
            Uri uri = Uri.parse(token);
            return "content".equals(uri.getScheme()) ? uri : null;
        }
        return null;
    }

    /** Called by Rust decode workers for a source this host listed earlier.
     * Returns an owned read-only descriptor, or -1 when access is gone. */
    public int openSource(int kind, String token) {
        try {
            Uri uri = sourceUri(kind, token);
            if (uri == null) return -1;
            ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r");
            return descriptor == null ? -1 : descriptor.detachFd();
        } catch (Exception unavailable) { return -1; }
    }

    /** MediaStore's cached thumbnail, as a software ARGB_8888 bitmap no larger
     * than edge x edge. Called on Rust decode workers, never the UI thread. */
    public android.graphics.Bitmap loadMediaThumbnail(String token, int edge) {
        try {
            Uri uri = sourceUri(KIND_MEDIA, token);
            if (uri == null || edge <= 0) return null;
            return softwareRgba(getContentResolver().loadThumbnail(uri, new android.util.Size(edge, edge), null),
                edge, (long) edge * edge);
        } catch (Exception | OutOfMemoryError unavailable) { return null; }
    }

    /** Decodes an app-owned local file with ImageDecoder, scaled while decoding
     * to at most maxSide and maxPixels, upright and in sRGB. original receives
     * the header size. Returns null to let Rust use its own decoders. */
    public android.graphics.Bitmap decodeScaled(String path, int maxSide, long maxPixels, int[] original) {
        try {
            android.graphics.ImageDecoder.Source source = android.graphics.ImageDecoder.createSource(new File(path));
            android.graphics.Bitmap bitmap = android.graphics.ImageDecoder.decodeBitmap(source, (decoder, info, ignored) -> {
                int w = info.getSize().getWidth(), h = info.getSize().getHeight();
                original[0] = w; original[1] = h;
                if (w <= 0 || h <= 0 || (long) w * h > 256_000_000L) throw new IllegalArgumentException("Image too large");
                double scale = Math.min(1.0, Math.min((double) maxSide / Math.max(w, h),
                    Math.sqrt((double) maxPixels / ((double) w * h))));
                decoder.setTargetSize(Math.max(1, (int) Math.floor(w * scale)), Math.max(1, (int) Math.floor(h * scale)));
                decoder.setAllocator(android.graphics.ImageDecoder.ALLOCATOR_SOFTWARE);
                decoder.setTargetColorSpace(android.graphics.ColorSpace.get(android.graphics.ColorSpace.Named.SRGB));
            });
            return softwareRgba(bitmap, maxSide, maxPixels);
        } catch (Exception | OutOfMemoryError unavailable) { return null; }
    }

    /** Native code copies raw ARGB_8888 bytes; convert other configs and keep
     * the output within the size the Rust caller allocated for. */
    private static android.graphics.Bitmap softwareRgba(android.graphics.Bitmap bitmap, int maxSide, long maxPixels) {
        if (bitmap == null) return null;
        android.graphics.Bitmap result = bitmap;
        if (result.getConfig() != android.graphics.Bitmap.Config.ARGB_8888) {
            android.graphics.Bitmap copy = result.copy(android.graphics.Bitmap.Config.ARGB_8888, false);
            result.recycle();
            if (copy == null) return null;
            result = copy;
        }
        int w = result.getWidth(), h = result.getHeight();
        double scale = Math.min(1.0, Math.min((double) maxSide / Math.max(w, h),
            Math.sqrt((double) maxPixels / ((double) w * h))));
        if (scale < 1.0) {
            android.graphics.Bitmap scaled = android.graphics.Bitmap.createScaledBitmap(result,
                Math.max(1, (int) Math.floor(w * scale)), Math.max(1, (int) Math.floor(h * scale)), true);
            if (scaled != result) result.recycle();
            result = scaled;
        }
        return result;
    }

    @Override protected void onDestroy() {
        cancelPhotoFolderQuery();
        if (photoFolderDialog != null) {
            photoFolderDialog.dismissSilently();
            photoFolderDialog = null;
        }
        pickerBusy = false;
        sourceResumed = false;
        invalidateSourceCache();
        if (sourceObserverReady) {
            try { getContentResolver().unregisterContentObserver(sourceObserver); }
            catch (Exception ignored) { }
            sourceObserverReady = false;
        }
        nativeReady = false;
        orientationRequest.incrementAndGet();
        imports.shutdown();
        super.onDestroy();
    }
}
