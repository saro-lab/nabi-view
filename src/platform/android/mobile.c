#include <jni.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <pthread.h>
#include <unistd.h>

extern size_t nabi_mobile_text(const char *, unsigned short *, size_t);
extern void nabi_mobile_back(void);
extern void nabi_mobile_locale_changed(void);
extern void nabi_mobile_window_mode_changed(int);
extern bool nabi_mobile_import_begin(void);
extern bool nabi_mobile_import_begin_folder(void);
extern void nabi_mobile_settings_import(int);
extern void nabi_mobile_export_result(bool);
extern bool nabi_mobile_import_utf16(int, const unsigned short *, size_t);
extern void nabi_mobile_import_error(const char *);
extern void nabi_mobile_photo_error(int);
extern void nabi_mobile_import_finish(void);
extern void nabi_mobile_source_utf16(const unsigned short *, size_t, const unsigned short *, size_t);

// Readers may run on the UI, render and import threads simultaneously. Only
// host replacement/teardown is exclusive; a slow import must not block insets.
static pthread_rwlock_t host_lock = PTHREAD_RWLOCK_INITIALIZER;
static JavaVM *vm;
static jobject activity;
static jmethodID pick_method, awake_method, copy_method, exit_method;
static jmethodID folder_method, orientation_method, immersive_method, share_method, export_method, import_method;
static jmethodID reopen_method, memory_method, hdr_method;
static char *paths[3];
static _Atomic float safe_area[4];

static JNIEnv *attach_environment(bool *attached) {
    JNIEnv *env = NULL;
    *attached = false;
    if (!vm) return NULL;
    jint status = (*vm)->GetEnv(vm, (void **)&env, JNI_VERSION_1_6);
    if (status == JNI_EDETACHED) {
        if ((*vm)->AttachCurrentThread(vm, &env, NULL) != JNI_OK) return NULL;
        *attached = true;
    } else if (status != JNI_OK) {
        return NULL;
    }
    return env;
}
static JNIEnv *environment(bool *attached) {
    pthread_rwlock_rdlock(&host_lock);
    JNIEnv *env = activity ? attach_environment(attached) : NULL;
    if (!env) pthread_rwlock_unlock(&host_lock);
    return env;
}
static void release(bool attached) {
    if (attached) (*vm)->DetachCurrentThread(vm);
    pthread_rwlock_unlock(&host_lock);
}
// Old picker workers can finish after their Activity has been destroyed. Never
// let their callbacks mutate a new viewer's import or display state.
static bool enter_callback(JNIEnv *env, jobject self) {
    pthread_rwlock_rdlock(&host_lock);
    if (activity && (*env)->IsSameObject(env, activity, self)) return true;
    pthread_rwlock_unlock(&host_lock);
    return false;
}
static void leave_callback(void) { pthread_rwlock_unlock(&host_lock); }
static bool clear_exception(JNIEnv *env) {
    if (!(*env)->ExceptionCheck(env)) return false;
    (*env)->ExceptionClear(env);
    return true;
}
static jstring translated_text(JNIEnv *env, jobject self, jstring key) {
    if (!key || !enter_callback(env, self)) return NULL;
    // Translation may query the system locale through JNI. It does not mutate
    // host state, so do not hold the host gate across that nested query.
    leave_callback();
    const char *id = (*env)->GetStringUTFChars(env, key, NULL);
    jstring result = NULL;
    if (id) {
        // Fixed upper bound; translations are short UI text, not user data.
        unsigned short text[4096];
        size_t length = nabi_mobile_text(id, text, 4096);
        if (length <= 4096) result = (*env)->NewString(env, text, (jsize)length);
        (*env)->ReleaseStringUTFChars(env, key, id);
    }
    return result;
}
static jboolean begin(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return false;
    bool result = nabi_mobile_import_begin();
    leave_callback(); return result;
}
static jboolean begin_folder(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return false;
    bool result = nabi_mobile_import_begin_folder();
    leave_callback(); return result;
}
static void settings(JNIEnv *env, jobject self, jint fd) {
    if (!enter_callback(env, self)) { if (fd >= 0) close(fd); return; }
    nabi_mobile_settings_import(fd);
    leave_callback();
}
static void export_result(JNIEnv *env, jobject self, jboolean saved) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_export_result(saved); leave_callback();
}
static jboolean document(JNIEnv *env, jobject self, jint fd, jstring name) {
    if (!enter_callback(env, self)) { if (fd >= 0) close(fd); return false; }
    const jchar *characters = name ? (*env)->GetStringChars(env, name, NULL) : NULL;
    jsize length = characters ? (*env)->GetStringLength(env, name) : 0;
    bool result = nabi_mobile_import_utf16(fd, characters, (size_t)length);
    if (characters) (*env)->ReleaseStringChars(env, name, characters);
    leave_callback(); return result;
}
static void back(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_back(); leave_callback();
}
static void locale_changed(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_locale_changed(); leave_callback();
}
static void window_mode_changed(JNIEnv *env, jobject self, jint state) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_window_mode_changed(state);
    leave_callback();
}
static void source(JNIEnv *env, jobject self, jstring id, jstring name) {
    if (!id || !name || !enter_callback(env, self)) return;
    const jchar *key = (*env)->GetStringChars(env, id, NULL);
    const jchar *label = (*env)->GetStringChars(env, name, NULL);
    if (key && label) nabi_mobile_source_utf16(key, (size_t)(*env)->GetStringLength(env, id),
        label, (size_t)(*env)->GetStringLength(env, name));
    if (key) (*env)->ReleaseStringChars(env, id, key);
    if (label) (*env)->ReleaseStringChars(env, name, label);
    leave_callback();
}
static void finish(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_import_finish(); leave_callback();
}
static void error(JNIEnv *env, jobject self) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_import_error("Could not read the selected document."); leave_callback();
}
static void photo_error(JNIEnv *env, jobject self, jint reason) {
    if (!enter_callback(env, self)) return;
    nabi_mobile_photo_error(reason); leave_callback();
}
static void insets(JNIEnv *env, jobject self, jfloat top, jfloat right, jfloat bottom, jfloat left) {
    if (!enter_callback(env, self)) return;
    atomic_store(&safe_area[0], top); atomic_store(&safe_area[1], right);
    atomic_store(&safe_area[2], bottom); atomic_store(&safe_area[3], left);
    leave_callback();
}

void nabi_android_init(void *java_vm, void *native_activity) {
    pthread_rwlock_wrlock(&host_lock);
    vm = (JavaVM *)java_vm;
    bool attached;
    JNIEnv *env = attach_environment(&attached);
    if (!env) { pthread_rwlock_unlock(&host_lock); return; }
    activity = (*env)->NewGlobalRef(env, (jobject)native_activity);
    jclass cls = (*env)->GetObjectClass(env, activity);
    JNINativeMethod methods[] = {
        {"nativeText", "(Ljava/lang/String;)Ljava/lang/String;", (void *)translated_text},
        {"nativeBegin", "()Z", (void *)begin},
        {"nativeDocument", "(ILjava/lang/String;)Z", (void *)document},
        {"nativeFinish", "()V", (void *)finish},
        {"nativeError", "()V", (void *)error},
        {"nativePhotoError", "(I)V", (void *)photo_error},
        {"nativeBack", "()V", (void *)back},
        {"nativeLocaleChanged", "()V", (void *)locale_changed},
        {"nativeWindowModeChanged", "(I)V", (void *)window_mode_changed},
        {"nativeInsets", "(FFFF)V", (void *)insets},
        {"nativeBeginFolder", "()Z", (void *)begin_folder},
        {"nativeSettings", "(I)V", (void *)settings},
        {"nativeExportResult", "(Z)V", (void *)export_result},
        {"nativeSource", "(Ljava/lang/String;Ljava/lang/String;)V", (void *)source},
    };
    (*env)->RegisterNatives(env, cls, methods, sizeof(methods) / sizeof(methods[0]));
    pick_method = (*env)->GetMethodID(env, cls, "pickImages", "()V");
    copy_method = (*env)->GetMethodID(env, cls, "copyText", "(Ljava/lang/String;)V");
    exit_method = (*env)->GetMethodID(env, cls, "closeViewer", "()V");
    awake_method = (*env)->GetMethodID(env, cls, "keepAwake", "(Z)V");
    folder_method = (*env)->GetMethodID(env, cls, "pickFolder", "()V");
    orientation_method = (*env)->GetMethodID(env, cls, "setOrientation", "(I)V");
    immersive_method = (*env)->GetMethodID(env, cls, "setImmersive", "(Z)V");
    share_method = (*env)->GetMethodID(env, cls, "share", "(Ljava/lang/String;)V");
    export_method = (*env)->GetMethodID(env, cls, "exportSettings", "(Ljava/lang/String;)V");
    import_method = (*env)->GetMethodID(env, cls, "importSettings", "()V");
    reopen_method = (*env)->GetMethodID(env, cls, "reopenSource", "(Ljava/lang/String;)Z");
    memory_method = (*env)->GetMethodID(env, cls, "memorySnapshot", "()[J");
    hdr_method = (*env)->GetMethodID(env, cls, "hdrDisplay", "()[F");
    jmethodID path_method = (*env)->GetMethodID(env, cls, "storagePath", "(I)Ljava/lang/String;");
    if (!clear_exception(env) && path_method) {
        for (int i = 0; i < 3; ++i) {
            jstring text = (*env)->CallObjectMethod(env, activity, path_method, i);
            if (clear_exception(env) || !text) continue;
            const char *utf8 = (*env)->GetStringUTFChars(env, text, NULL);
            if (utf8) { paths[i] = strdup(utf8); (*env)->ReleaseStringUTFChars(env, text, utf8); }
            (*env)->DeleteLocalRef(env, text);
        }
        jmethodID ready = (*env)->GetMethodID(env, cls, "onRustReady", "()V");
        if (ready) (*env)->CallVoidMethod(env, activity, ready);
        clear_exception(env);
    }
    (*env)->DeleteLocalRef(env, cls);
    release(attached);
}

void nabi_android_shutdown(void) {
    pthread_rwlock_wrlock(&host_lock);
    bool attached;
    JNIEnv *env = attach_environment(&attached);
    if (env && activity) (*env)->DeleteGlobalRef(env, activity);
    activity = NULL;
    for (int i = 0; i < 3; ++i) { free(paths[i]); paths[i] = NULL; }
    for (int i = 0; i < 4; ++i) atomic_store(&safe_area[i], 0);
    release(attached);
}

const char *nabi_mobile_path(int kind) { return kind >= 0 && kind < 3 ? paths[kind] : NULL; }
void nabi_mobile_safe_area(float *out) { for (int i = 0; i < 4; ++i) out[i] = atomic_load(&safe_area[i]); }
void nabi_mobile_pick(void) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (pick_method) (*env)->CallVoidMethod(env, activity, pick_method);
    if (clear_exception(env)) {
        nabi_mobile_import_begin(); nabi_mobile_import_error("The document picker is unavailable."); nabi_mobile_import_finish();
    }
    release(attached);
}
void nabi_mobile_keep_awake(bool enabled) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (awake_method) (*env)->CallVoidMethod(env, activity, awake_method, enabled);
    clear_exception(env); release(attached);
}

void nabi_mobile_copy_text(const unsigned short *text, int length) {
    if (length < 0) return;
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    jstring value = (*env)->NewString(env, text, length);
    if (value && copy_method) (*env)->CallVoidMethod(env, activity, copy_method, value);
    if (value) (*env)->DeleteLocalRef(env, value);
    clear_exception(env); release(attached);
}
void nabi_mobile_pick_folder(void) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (folder_method) (*env)->CallVoidMethod(env, activity, folder_method);
    if (clear_exception(env)) {
        nabi_mobile_import_begin_folder(); nabi_mobile_import_error("The folder picker is unavailable."); nabi_mobile_import_finish();
    }
    release(attached);
}
void nabi_mobile_set_orientation(int mode) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (orientation_method) (*env)->CallVoidMethod(env, activity, orientation_method, (jint)mode);
    clear_exception(env); release(attached);
}
void nabi_mobile_set_immersive(bool enabled) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (immersive_method) (*env)->CallVoidMethod(env, activity, immersive_method, (jboolean)enabled);
    clear_exception(env); release(attached);
}
static void call_with_text(jmethodID *method, const unsigned short *text, int length) {
    if (length < 0) return;
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    jstring value = (*env)->NewString(env, text, length);
    if (value && *method) (*env)->CallVoidMethod(env, activity, *method, value);
    if (value) (*env)->DeleteLocalRef(env, value);
    clear_exception(env); release(attached);
}
void nabi_mobile_share(const unsigned short *path, int length) { call_with_text(&share_method, path, length); }
void nabi_mobile_export_settings(const unsigned short *path, int length) {
    call_with_text(&export_method, path, length);
}
void nabi_mobile_import_settings(void) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (import_method) (*env)->CallVoidMethod(env, activity, import_method);
    if (clear_exception(env)) nabi_mobile_settings_import(-1);
    release(attached);
}

bool nabi_mobile_reopen(const unsigned short *text, int length) {
    if (length < 0) return false;
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return false;
    jstring value = (*env)->NewString(env, text, length);
    bool accepted = value && reopen_method && (*env)->CallBooleanMethod(env, activity, reopen_method, value);
    if (clear_exception(env)) accepted = false;
    if (value) (*env)->DeleteLocalRef(env, value);
    release(attached);
    return accepted;
}

bool nabi_mobile_memory(uint64_t *total, uint64_t *available) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return false;
    jlongArray values = memory_method ? (jlongArray)(*env)->CallObjectMethod(env, activity, memory_method) : NULL;
    bool ok = !clear_exception(env) && values && (*env)->GetArrayLength(env, values) == 2;
    if (ok) {
        jlong memory[2] = {0, 0};
        (*env)->GetLongArrayRegion(env, values, 0, 2, memory);
        ok = !clear_exception(env) && memory[0] > 0 && memory[1] >= 0;
        if (ok) { *total = (uint64_t)memory[0]; *available = (uint64_t)memory[1]; }
    }
    if (values) (*env)->DeleteLocalRef(env, values);
    release(attached);
    return ok;
}

bool nabi_mobile_hdr(float *desired_max_nits, float *hdr_sdr_ratio) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return false;
    jfloatArray values = hdr_method ? (jfloatArray)(*env)->CallObjectMethod(env, activity, hdr_method) : NULL;
    bool ok = !clear_exception(env) && values && (*env)->GetArrayLength(env, values) == 2;
    if (ok) {
        jfloat hdr[2] = {0, 0};
        (*env)->GetFloatArrayRegion(env, values, 0, 2, hdr);
        ok = !clear_exception(env);
        if (ok) { *desired_max_nits = hdr[0]; *hdr_sdr_ratio = hdr[1]; }
    }
    if (values) (*env)->DeleteLocalRef(env, values);
    release(attached);
    return ok;
}
void nabi_mobile_exit(void) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    if (exit_method) (*env)->CallVoidMethod(env, activity, exit_method);
    clear_exception(env); release(attached);
}

// The Java host handles API-level differences and app-specific preferences.
size_t nabi_locale_languages(unsigned char *buffer, size_t capacity) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return 0;
    jclass cls = (*env)->GetObjectClass(env, activity);
    jmethodID method = (*env)->GetMethodID(env, cls, "preferredLanguageTags", "()Ljava/lang/String;");
    jstring value = method ? (jstring)(*env)->CallObjectMethod(env, activity, method) : NULL;
    size_t size = 0;
    if (!clear_exception(env) && value) {
        const char *text = (*env)->GetStringUTFChars(env, value, NULL);
        if (text) {
            size = strlen(text) + 1;
            if (buffer && capacity >= size) memcpy(buffer, text, size);
            (*env)->ReleaseStringUTFChars(env, value, text);
        }
    }
    if (value) (*env)->DeleteLocalRef(env, value);
    (*env)->DeleteLocalRef(env, cls);
    clear_exception(env); release(attached);
    return size;
}
void nabi_mobile_set_language(const char *code) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return;
    jclass cls = (*env)->GetObjectClass(env, activity);
    jmethodID method = (*env)->GetMethodID(env, cls, "setApplicationLanguage", "(Ljava/lang/String;)V");
    jstring value = (*env)->NewStringUTF(env, code);
    if (method && value) (*env)->CallVoidMethod(env, activity, method, value);
    clear_exception(env);
    if (value) (*env)->DeleteLocalRef(env, value);
    (*env)->DeleteLocalRef(env, cls);
    release(attached);
}

size_t nabi_mobile_saved_language(const char *persisted, unsigned char *buffer, size_t capacity) {
    bool attached; JNIEnv *env = environment(&attached);
    if (!env) return 0;
    jclass cls = (*env)->GetObjectClass(env, activity);
    if (clear_exception(env) || !cls) { release(attached); return 0; }
    jmethodID method = (*env)->GetMethodID(env, cls, "savedLanguagePreference", "(Ljava/lang/String;)Ljava/lang/String;");
    jstring input = NULL, value = NULL;
    size_t size = 0;
    if (!clear_exception(env) && method) {
        input = (*env)->NewStringUTF(env, persisted);
        if (input && !clear_exception(env)) value = (jstring)(*env)->CallObjectMethod(env, activity, method, input);
        if (!clear_exception(env) && value) {
            const char *text = (*env)->GetStringUTFChars(env, value, NULL);
            if (text) {
                size = strlen(text) + 1;
                if (buffer && capacity >= size) memcpy(buffer, text, size);
                (*env)->ReleaseStringUTFChars(env, value, text);
            }
        }
    }
    clear_exception(env);
    if (input) (*env)->DeleteLocalRef(env, input);
    if (value) (*env)->DeleteLocalRef(env, value);
    (*env)->DeleteLocalRef(env, cls);
    release(attached);
    return size;
}
