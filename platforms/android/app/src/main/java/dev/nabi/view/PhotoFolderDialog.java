// Android View API review 2026-11-16: https://developer.android.com/reference/android/widget/ListView
package dev.nabi.view;

import android.app.Activity;
import android.app.Dialog;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.Rect;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.os.Bundle;
import android.provider.MediaStore;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.widget.BaseAdapter;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.ProgressBar;
import android.widget.TextView;
import java.text.NumberFormat;
import java.util.ArrayList;
import java.util.List;
import java.util.function.BooleanSupplier;
import java.util.function.Function;

/** Presentation only: the Activity owns permissions, queries and imports.
 * Uses recycled native rows, without decoding photos or creating new grants. */
final class PhotoFolderDialog extends Dialog {
    interface Listener {
        void onFolder(PhotoFolders.Folder folder);
        void onFiles();
        void onAccess();
        void onCancel();
        void onRetry();
    }

    // Same surface, ink and sage accent as the Rust viewer overlays.
    private static final int BACKGROUND = 0xff151719, SURFACE = 0xff1c1e1f;
    private static final int BORDER = 0xff353a3c, INK = 0xffecede7, MUTED = 0xff9fa5a1;
    private static final int ACCENT = 0xffcce8a5, ACCENT_SURFACE = 0xff293326, WARNING = 0xfff4c691;
    private static final int FOLDER = 0, FILES = 1, CLOSE = 2, CHEVRON = 3, ACCESS = 4;
    private enum State { LOADING, READY, FAILED }

    private final Activity activity;
    private final Function<String, String> text;
    private final BooleanSupplier rtl;
    private final Listener listener;
    private final List<PhotoFolders.Folder> folders = new ArrayList<>();
    private State state = State.LOADING;
    private boolean readable, fullAccess, finished;
    private LinearLayout root;
    private ListView list;
    private final View.OnLayoutChangeListener hostLayout = (view, left, top, right, bottom,
            oldLeft, oldTop, oldRight, oldBottom) -> {
        if (left != oldLeft || top != oldTop || right != oldRight || bottom != oldBottom) resize();
    };

    PhotoFolderDialog(Activity activity, Function<String, String> text, BooleanSupplier rtl, Listener listener) {
        super(activity, android.R.style.Theme_Material_NoActionBar);
        this.activity = activity;
        this.text = text;
        this.rtl = rtl;
        this.listener = listener;
        setOnCancelListener(ignored -> finish(listener::onCancel));
        setOnDismissListener(ignored -> {
            // Dismiss from a selected action must not release the Activity's
            // picker reservation before its permission/import handoff finishes.
            if (!finished) finish(listener::onCancel);
        });
    }

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        Window window = getWindow();
        window.setBackgroundDrawableResource(android.R.color.transparent);
        window.addFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND);
        window.setDimAmount(0.64f);
        window.setStatusBarColor(BACKGROUND);
        window.setNavigationBarColor(BACKGROUND);
        window.setDecorFitsSystemWindows(true);
        // Install the decor before asking for its (initially pending) insets controller.
        window.getDecorView();
        styleSystemBars();
        WindowManager.LayoutParams attributes = window.getAttributes();
        attributes.setFitInsetsTypes(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
        attributes.setFitInsetsIgnoringVisibility(true);
        window.setAttributes(attributes);
        setCanceledOnTouchOutside(true);
        rebuild();
    }

    @Override protected void onStart() {
        super.onStart();
        activity.getWindow().getDecorView().addOnLayoutChangeListener(hostLayout);
        styleSystemBars();
        resize();
    }

    @Override protected void onStop() {
        activity.getWindow().getDecorView().removeOnLayoutChangeListener(hostLayout);
        super.onStop();
    }

    private void styleSystemBars() {
        android.view.WindowInsetsController controller = getWindow().getInsetsController();
        if (controller != null) controller.setSystemBarsAppearance(0,
            android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                | android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS);
    }

    void setLoading() {
        if (finished) return;
        state = State.LOADING;
        folders.clear();
        if (root != null) rebuild();
    }

    void setFolders(List<PhotoFolders.Folder> values, boolean readable, boolean fullAccess) {
        if (finished) return;
        folders.clear();
        folders.addAll(values);
        this.readable = readable;
        this.fullAccess = fullAccess;
        state = State.READY;
        rebuild();
    }

    void setFailure(boolean readable, boolean fullAccess) {
        if (finished) return;
        folders.clear();
        this.readable = readable;
        this.fullAccess = fullAccess;
        state = State.FAILED;
        rebuild();
    }

    void refreshConfiguration() {
        if (finished || root == null) return;
        int position = list.getFirstVisiblePosition();
        View first = list.getChildAt(0);
        int top = first == null ? 0 : first.getTop();
        rebuild();
        list.setSelectionFromTop(position, top);
        resize();
    }

    void dismissSilently() {
        finished = true;
        dismiss();
    }

    private void finish(Runnable action) {
        if (finished) return;
        finished = true;
        dismiss();
        action.run();
    }

    private void rebuild() {
        root = vertical();
        // Android 12 supports an app language through Rust, independently of
        // the system Configuration. Match the same catalog that provides text.
        root.setLayoutDirection(rtl.getAsBoolean() ? View.LAYOUT_DIRECTION_RTL : View.LAYOUT_DIRECTION_LTR);
        root.setAccessibilityPaneTitle(text.apply("open-folder"));
        root.setPadding(dp(20), dp(12), dp(20), dp(16));
        root.setFocusableInTouchMode(true);

        LinearLayout heading = horizontal();
        TextView title = label(text.apply("open-folder"), 24, INK, true);
        title.setAccessibilityHeading(true);
        heading.addView(title, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        Glyph close = new Glyph(CLOSE, MUTED);
        close.setContentDescription(text.apply("access-close"));
        close.setTooltipText(text.apply("access-close"));
        close.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        buttonSemantics(close);
        close.setBackground(ripple(SURFACE, 12));
        close.setFocusable(true);
        close.setOnClickListener(ignored -> finish(listener::onCancel));
        heading.addView(close, new LinearLayout.LayoutParams(dp(48), dp(48)));
        root.addView(heading, matchWrap());

        list = new ListView(getContext());
        list.setBackgroundColor(Color.TRANSPARENT);
        list.setDivider(null);
        list.setSelector(ripple(0x00000000, 12));
        list.setDrawSelectorOnTop(true);
        list.setCacheColorHint(Color.TRANSPARENT);
        list.setClipToPadding(false);
        list.setPadding(0, 0, 0, dp(8));
        list.setScrollBarStyle(View.SCROLLBARS_INSIDE_OVERLAY);
        list.setOverScrollMode(View.OVER_SCROLL_IF_CONTENT_SCROLLS);
        list.setEdgeEffectColor(ACCENT);
        list.setVerticalScrollbarThumbDrawable(shape(BORDER, 4, 0));
        list.addHeaderView(listHeader(), null, false);
        if (state != State.READY || folders.isEmpty()) list.addFooterView(statusPanel(), null, false);
        list.setAdapter(new FolderAdapter());
        // Keep folder rows free of tooltips so touch events reach ListView.
        list.setOnItemClickListener((parent, view, position, id) -> {
            int index = position - list.getHeaderViewsCount();
            if (state == State.READY && index >= 0 && index < folders.size()) {
                PhotoFolders.Folder folder = folders.get(index);
                finish(() -> listener.onFolder(folder));
            }
        });
        LinearLayout.LayoutParams scrolling = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1);
        scrolling.topMargin = dp(8);
        root.addView(list, scrolling);

        View rule = new View(getContext());
        rule.setBackgroundColor(BORDER);
        root.addView(rule, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)));
        LinearLayout files = action(FILES, text.apply("file-folder"), null, true);
        files.setOnClickListener(ignored -> finish(listener::onFiles));
        LinearLayout.LayoutParams bottom = matchWrap();
        bottom.topMargin = dp(12);
        root.addView(files, bottom);
        setContentView(root);
        resize();
    }

    private View listHeader() {
        LinearLayout header = vertical();
        header.setPadding(0, dp(6), 0, dp(12));
        String status = state == State.LOADING ? null : text.apply(fullAccess
            ? "photo-access-authorized" : readable ? "photo-access-limited" : "photo-access-request");
        LinearLayout access = action(ACCESS, text.apply("photo-access"), status, false);
        access.setOnClickListener(ignored -> finish(listener::onAccess));
        header.addView(access, matchWrap());
        LinearLayout section = horizontal();
        section.setPadding(0, dp(22), 0, 0);
        TextView name = label(text.apply("photo-folders"), 14, MUTED, true);
        name.setAccessibilityHeading(true);
        section.addView(name, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        if (state == State.READY && !folders.isEmpty()) {
            TextView count = label(number(folders.size()), 12, MUTED, false);
            count.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
            section.addView(count);
        }
        header.addView(section, matchWrap());
        return header;
    }

    private View statusPanel() {
        LinearLayout panel = vertical();
        panel.setGravity(Gravity.CENTER_HORIZONTAL);
        panel.setPadding(dp(20), dp(32), dp(20), dp(32));
        panel.setBackground(shape(SURFACE, 12, BORDER));
        if (state == State.LOADING) {
            ProgressBar progress = new ProgressBar(getContext());
            progress.setIndeterminateTintList(ColorStateList.valueOf(ACCENT));
            panel.addView(progress, new LinearLayout.LayoutParams(dp(28), dp(28)));
        } else {
            Glyph icon = new Glyph(FOLDER, MUTED);
            panel.addView(icon, new LinearLayout.LayoutParams(dp(40), dp(40)));
        }
        String message = text.apply(state == State.LOADING ? "opening-folder-dialog"
            : state == State.FAILED ? "photo-read-failed"
            : readable ? "photo-collection-empty" : "photo-access-help");
        TextView description = label(message, 14, MUTED, false);
        description.setGravity(Gravity.CENTER);
        description.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        LinearLayout.LayoutParams copy = matchWrap();
        copy.topMargin = dp(16);
        panel.addView(description, copy);
        if (state == State.FAILED) {
            TextView retry = label(text.apply("access-retry"), 14, ACCENT, true);
            retry.setGravity(Gravity.CENTER);
            retry.setMinHeight(dp(48));
            retry.setPadding(dp(16), 0, dp(16), 0);
            retry.setBackground(ripple(ACCENT_SURFACE, 10));
            retry.setFocusable(true);
            buttonSemantics(retry);
            retry.setOnClickListener(ignored -> { setLoading(); listener.onRetry(); });
            LinearLayout.LayoutParams button = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            button.topMargin = dp(16);
            panel.addView(retry, button);
        }
        return panel;
    }

    private LinearLayout action(int glyph, String title, String detail, boolean primary) {
        LinearLayout row = horizontal();
        row.setMinimumHeight(dp(56));
        row.setPaddingRelative(dp(14), dp(10), dp(14), dp(10));
        row.setBackground(ripple(primary ? ACCENT : SURFACE, 12));
        row.setFocusable(true);
        buttonSemantics(row);
        int ink = primary ? BACKGROUND : INK;
        Glyph icon = new Glyph(glyph, primary ? BACKGROUND : state == State.LOADING ? MUTED : fullAccess ? ACCENT : WARNING);
        row.addView(icon, new LinearLayout.LayoutParams(dp(24), dp(24)));
        LinearLayout copy = vertical();
        copy.setPaddingRelative(dp(12), 0, dp(12), 0);
        copy.addView(label(title, 15, ink, true));
        if (detail != null) {
            TextView secondary = label(detail, 12, MUTED, false);
            secondary.setPadding(0, dp(3), 0, 0);
            copy.addView(secondary);
        }
        row.addView(copy, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
        row.addView(new Glyph(CHEVRON, primary ? BACKGROUND : MUTED),
            new LinearLayout.LayoutParams(dp(16), dp(24)));
        row.setContentDescription(title + (detail == null ? "" : ", " + detail));
        copy.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
        return row;
    }

    private final class FolderAdapter extends BaseAdapter {
        @Override public int getCount() { return folders.size(); }
        @Override public PhotoFolders.Folder getItem(int position) { return folders.get(position); }
        @Override public long getItemId(int position) { return position; }
        @Override public View getView(int position, View recycled, ViewGroup parent) {
            FolderRow row = recycled instanceof FolderRow ? (FolderRow) recycled : new FolderRow();
            row.bind(getItem(position));
            return row;
        }
    }

    private final class FolderRow extends FrameLayout {
        private final TextView name, path, count;
        FolderRow() {
            super(PhotoFolderDialog.this.getContext());
            setPadding(0, 0, 0, dp(8));
            LinearLayout card = horizontal();
            card.setMinimumHeight(dp(88));
            card.setPaddingRelative(dp(14), dp(14), dp(12), dp(14));
            card.setBackground(shape(SURFACE, 12, BORDER));
            Glyph icon = new Glyph(FOLDER, ACCENT);
            icon.setBackground(shape(ACCENT_SURFACE, 10, 0));
            card.addView(icon, new LinearLayout.LayoutParams(dp(40), dp(40)));
            LinearLayout copy = vertical();
            copy.setPaddingRelative(dp(12), 0, dp(12), 0);
            name = label("", 16, INK, true);
            // Real folder names may be long: wrap the complete name rather
            // than collapsing different folders to identical ellipses.
            name.setTextDirection(View.TEXT_DIRECTION_FIRST_STRONG);
            copy.addView(name, matchWrap());
            path = label("", 12, MUTED, false);
            path.setSingleLine(true);
            path.setEllipsize(TextUtils.TruncateAt.MIDDLE);
            path.setTextDirection(View.TEXT_DIRECTION_FIRST_STRONG_LTR);
            path.setPadding(0, dp(5), 0, 0);
            copy.addView(path, matchWrap());
            card.addView(copy, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1));
            count = label("", 12, ACCENT, true);
            count.setGravity(Gravity.CENTER);
            count.setMinWidth(dp(32));
            count.setPadding(dp(8), dp(5), dp(8), dp(5));
            count.setBackground(shape(ACCENT_SURFACE, 7, 0));
            card.addView(count);
            LinearLayout.LayoutParams arrow = new LinearLayout.LayoutParams(dp(12), dp(20));
            arrow.setMarginStart(dp(8));
            card.addView(new Glyph(CHEVRON, MUTED), arrow);
            card.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
            addView(card, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        }
        void bind(PhotoFolders.Folder folder) {
            String clean = folder.path.replaceAll("/+$", "");
            int separator = clean.lastIndexOf('/');
            String title = clean.isEmpty() ? "/" : clean.substring(separator + 1);
            String parent = separator < 0 ? "/" : clean.substring(0, separator) + "/";
            if (!MediaStore.VOLUME_EXTERNAL_PRIMARY.equals(folder.volume)) parent += " · " + folder.volume;
            name.setText(title);
            path.setText(parent);
            count.setText(number(folder.count));
            setContentDescription(title + ", " + parent + ", " + text.apply("images") + ": " + number(folder.count));
        }
    }

    private void resize() {
        if (root == null) return;
        android.view.WindowMetrics metrics = activity.getWindowManager().getCurrentWindowMetrics();
        Rect bounds = metrics.getBounds();
        android.graphics.Insets insets = metrics.getWindowInsets().getInsetsIgnoringVisibility(
            WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
        int width = Math.max(1, bounds.width() - insets.left - insets.right);
        int height = Math.max(1, bounds.height() - insets.top - insets.bottom);
        boolean compact = width <= dp(600);
        int gutter = compact ? 0 : dp(24);
        root.setBackground(shape(BACKGROUND, compact ? 0 : 20, compact ? 0 : BORDER));
        getWindow().setGravity(Gravity.CENTER);
        int targetWidth = compact ? width : Math.min(dp(640), Math.max(1, width - gutter * 2));
        int targetHeight = compact ? height : Math.min(dp(720), Math.max(1, height - gutter * 2));
        WindowManager.LayoutParams current = getWindow().getAttributes();
        if (current.width != targetWidth || current.height != targetHeight) getWindow().setLayout(targetWidth, targetHeight);
    }

    private LinearLayout vertical() {
        LinearLayout view = new LinearLayout(getContext());
        view.setOrientation(LinearLayout.VERTICAL);
        return view;
    }
    private LinearLayout horizontal() {
        LinearLayout view = new LinearLayout(getContext());
        view.setOrientation(LinearLayout.HORIZONTAL);
        view.setGravity(Gravity.CENTER_VERTICAL);
        return view;
    }
    private LinearLayout.LayoutParams matchWrap() {
        return new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }
    private void buttonSemantics(View view) {
        view.setAccessibilityDelegate(new View.AccessibilityDelegate() {
            @Override public void onInitializeAccessibilityNodeInfo(View host,
                    android.view.accessibility.AccessibilityNodeInfo info) {
                super.onInitializeAccessibilityNodeInfo(host, info);
                info.setClassName(android.widget.Button.class.getName());
            }
        });
    }
    private TextView label(String value, float size, int color, boolean strong) {
        TextView view = new TextView(getContext());
        view.setText(value);
        view.setTextSize(size);
        view.setTextColor(color);
        view.setFontFeatureSettings("tnum");
        view.setTypeface(Typeface.create("sans-serif", strong ? Typeface.BOLD : Typeface.NORMAL));
        view.setIncludeFontPadding(false);
        view.setGravity(Gravity.START);
        return view;
    }
    private String number(int value) {
        return NumberFormat.getIntegerInstance(activity.getResources().getConfiguration().getLocales().get(0)).format(value);
    }
    private int dp(float value) { return Math.round(value * getContext().getResources().getDisplayMetrics().density); }
    private GradientDrawable shape(int color, float radius, int border) {
        GradientDrawable result = new GradientDrawable();
        result.setColor(color);
        result.setCornerRadius(dp(radius));
        if (border != 0) result.setStroke(dp(1), border);
        return result;
    }
    private RippleDrawable ripple(int color, float radius) {
        return new RippleDrawable(ColorStateList.valueOf(0x35cce8a5), shape(color, radius, 0), shape(Color.WHITE, radius, 0));
    }

    /** Small stroked icons match the viewer toolbar; no bitmap or font dependency. */
    private final class Glyph extends View {
        private final int kind;
        private final Paint pen = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path path = new Path();
        Glyph(int kind, int color) {
            super(PhotoFolderDialog.this.getContext());
            this.kind = kind;
            pen.setColor(color);
            pen.setStrokeWidth(1.7f);
            pen.setStyle(Paint.Style.STROKE);
            pen.setStrokeJoin(Paint.Join.ROUND);
            pen.setStrokeCap(Paint.Cap.ROUND);
            setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        }
        @Override protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            canvas.save();
            float edge = Math.min(Math.min(getWidth(), getHeight()), dp(24));
            canvas.translate((getWidth() - edge) / 2, (getHeight() - edge) / 2);
            canvas.scale(edge / 24, edge / 24);
            path.reset();
            switch (kind) {
                case CLOSE -> { canvas.drawLine(6, 6, 18, 18, pen); canvas.drawLine(18, 6, 6, 18, pen); }
                case CHEVRON -> {
                    if (getLayoutDirection() == View.LAYOUT_DIRECTION_RTL) { canvas.translate(24, 0); canvas.scale(-1, 1); }
                    path.moveTo(9, 6); path.lineTo(15, 12); path.lineTo(9, 18); canvas.drawPath(path, pen);
                }
                case ACCESS -> {
                    path.moveTo(12, 3); path.lineTo(20, 6); path.lineTo(19, 14);
                    path.quadTo(17, 19, 12, 22); path.quadTo(7, 19, 5, 14);
                    path.lineTo(4, 6); path.close(); canvas.drawPath(path, pen);
                    if (fullAccess && state != State.LOADING) {
                        path.reset(); path.moveTo(8, 12); path.lineTo(11, 15); path.lineTo(16, 10); canvas.drawPath(path, pen);
                    } else {
                        canvas.drawLine(12, 9, 12, 13, pen); canvas.drawPoint(12, 16, pen);
                    }
                }
                default -> {
                    path.moveTo(3, 7); path.lineTo(3, 5); path.lineTo(10, 5); path.lineTo(12, 8);
                    path.lineTo(21, 8); path.lineTo(21, 19); path.lineTo(3, 19); path.close();
                    canvas.drawPath(path, pen);
                    if (kind == FILES) { canvas.drawLine(8, 13, 16, 13, pen); canvas.drawLine(13, 10, 16, 13, pen); canvas.drawLine(13, 16, 16, 13, pen); }
                }
            }
            canvas.restore();
        }
    }
}
