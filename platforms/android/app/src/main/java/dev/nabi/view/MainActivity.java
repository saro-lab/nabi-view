// API review 2026-11-08: https://developer.android.com/training/data-storage/shared/documents-files
package dev.nabi.view;

import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.view.WindowInsets;
import android.view.WindowManager;
import java.io.File;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class MainActivity extends NativeActivity {
    private static final int PICK_IMAGES = 100;
    private final ExecutorService imports = Executors.newSingleThreadExecutor();
    private boolean pickerBusy;
    private boolean nativeReady;
    private native void nativeBack();
    private native boolean nativeBegin();
    private native boolean nativeDocument(int ownedFd, String displayName);
    private native void nativeFinish();
    private native void nativeError();
    private native void nativeInsets(float top, float right, float bottom, float left);

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
    public void pickImages() {
        runOnUiThread(() -> {
            if (pickerBusy || isFinishing()) return;
            pickerBusy = true;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            // Some providers do not classify JXL/AVIF/RAW as image/*.
            intent.setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            try { startActivityForResult(intent, PICK_IMAGES); }
            catch (Exception error) { pickerBusy = false; nativeBegin(); nativeError(); nativeFinish(); }
        });
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_IMAGES) return;
        if (result != RESULT_OK || data == null) { pickerBusy = false; return; }
        List<Uri> uris = new ArrayList<>();
        if (data.getClipData() != null) {
            for (int i = 0; i < data.getClipData().getItemCount(); ++i) uris.add(data.getClipData().getItemAt(i).getUri());
        } else if (data.getData() != null) uris.add(data.getData());
        imports.execute(() -> {
            try {
                if (!nativeBegin()) return;
                for (Uri uri : uris) {
                    String name = null;
                    try (Cursor cursor = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
                        if (cursor != null && cursor.moveToFirst()) name = cursor.getString(0);
                    }
                    try (ParcelFileDescriptor descriptor = getContentResolver().openFileDescriptor(uri, "r")) {
                        if (descriptor == null || name == null) throw new java.io.IOException("Unavailable document");
                        // Rust owns and closes the detached descriptor, including on failure.
                        if (!nativeDocument(descriptor.detachFd(), name)) break;
                    }
                }
            } catch (Exception error) { nativeError(); }
            finally { nativeFinish(); runOnUiThread(() -> pickerBusy = false); }
        });
    }
    @Override protected void onDestroy() {
        nativeReady = false;
        imports.shutdown();
        super.onDestroy();
    }
}
