// API review 2026-11-08: https://developer.android.com/training/data-storage/shared/documents-files
package dev.nabi.view;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
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

public final class MainActivity extends NativeActivity {
    private static final int PICK_IMAGES = 100;
    private static final int PICK_FOLDER = 101;
    private static final int EXPORT_SETTINGS = 102;
    private static final int IMPORT_SETTINGS = 103;
    private final ExecutorService imports = Executors.newSingleThreadExecutor();
    private volatile boolean pickerBusy;
    private boolean nativeReady;
    private String pendingExport;
    private native void nativeBack();
    private native boolean nativeBegin();
    private native boolean nativeBeginFolder();
    private native void nativeSettings(int ownedFd);
    private native void nativeExportResult(boolean saved);
    private native boolean nativeDocument(int ownedFd, String displayName);
    private native void nativeFinish();
    private native void nativeError();
    private native void nativeInsets(float top, float right, float bottom, float left);
    private native void nativeSource(String id, String displayName);

    // Reopen is called from Rust, while picker requests run on the UI thread.
    // Reserve both through one monitor so they cannot start together.
    private synchronized boolean reservePicker() {
        if (pickerBusy || isFinishing()) return false;
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
            runOnUiThread(() -> getSystemService(android.app.LocaleManager.class)
                .setApplicationLocales(android.os.LocaleList.forLanguageTags(tag)));
        }
    }

    public String storagePath(int kind) {
        return (kind == 0 ? getFilesDir() : getCacheDir()).getAbsolutePath();
    }
    public void onRustReady() {
        runOnUiThread(() -> {
            nativeReady = true;
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, () -> nativeBack());
            }
            getWindow().getDecorView().setOnApplyWindowInsetsListener((view, windowInsets) -> {
                if (nativeReady) {
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
            if (android.os.Build.VERSION.SDK_INT >= 30) getWindow().setDecorFitsSystemWindows(false);
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
        runOnUiThread(() -> setRequestedOrientation(
            mode == 1 ? ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
                : mode == 2 ? ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                : ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED));
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
            if (!reservePicker()) return;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            try { startActivityForResult(intent, PICK_FOLDER); }
            catch (Exception error) { pickerBusy = false; nativeBeginFolder(); nativeError(); nativeFinish(); }
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
        imports.execute(() -> importDocuments(uris, rememberSource(uris, false, flags)));
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
        imports.execute(() -> {
            try {
                String saved = getSharedPreferences("document-grants", MODE_PRIVATE).getString(id, null);
                if (saved == null || saved.length() > 1024 * 1024) throw new java.io.IOException("Grant unavailable");
                JSONObject record = new JSONObject(saved);
                JSONArray values = record.getJSONArray("uris");
                if (values.length() == 0 || values.length() > 128) throw new java.io.IOException("Invalid grant");
                List<Uri> uris = new ArrayList<>();
                for (int i = 0; i < values.length(); ++i) {
                    Uri uri = Uri.parse(values.getString(i));
                    if (!"content".equals(uri.getScheme())) throw new java.io.IOException("Invalid grant");
                    uris.add(uri);
                }
                if (record.getBoolean("folder")) importFolder(uris.get(0), id);
                else importDocuments(uris, id);
            } catch (Exception denied) {
                nativeBegin(); nativeError(); nativeFinish();
                runOnUiThread(() -> pickerBusy = false);
            }
        });
        return true;
    }
    /** Imports the folder's files; Rust keeps supported images and skips the rest. */
    private void importFolder(Uri tree, String id) {
        try {
            if (!nativeBeginFolder()) return;
            reportSource(id, displayName(tree, true));
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
            List<String[]> documents = new ArrayList<>();
            try (Cursor cursor = getContentResolver().query(children, new String[]{
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                    DocumentsContract.Document.COLUMN_MIME_TYPE}, null, null, null)) {
                while (cursor != null && cursor.moveToNext()) {
                    if (DocumentsContract.Document.MIME_TYPE_DIR.equals(cursor.getString(2))) continue;
                    String name = cursor.getString(1);
                    if (name != null) documents.add(new String[]{cursor.getString(0), name});
                }
            }
            for (String[] document : documents) {
                Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, document[0]);
                try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r")) {
                    if (descriptor == null) continue;
                    // Rust owns and closes the detached descriptor, including on failure.
                    if (!nativeDocument(descriptor.detachFd(), document[1])) break;
                } catch (Exception skipped) { /* Unreadable files are left out. */ }
            }
        } catch (Exception error) { nativeError(); }
        finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
    }
    @Override protected void onDestroy() {
        nativeReady = false;
        imports.shutdown();
        super.onDestroy();
    }
}
