#include <jni.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

extern void nabi_mobile_back(void);
extern bool nabi_mobile_import_begin(void);
extern bool nabi_mobile_import_utf16(int, const unsigned short *, size_t);
extern void nabi_mobile_import_error(const char *);
extern void nabi_mobile_import_finish(void);

static JavaVM *vm;
static jobject activity;
static jmethodID pick_method, awake_method, copy_method, exit_method;
static char *paths[3];
static _Atomic float safe_area[4];

static JNIEnv *environment(bool *attached) {
    JNIEnv *env = NULL;
    *attached = false;
    if (!vm) return NULL;
    if ((*vm)->GetEnv(vm, (void **)&env, JNI_VERSION_1_6) == JNI_EDETACHED) {
        if ((*vm)->AttachCurrentThread(vm, &env, NULL) != JNI_OK) return NULL;
        *attached = true;
    }
    return env;
}
static void release(bool attached) { if (attached) (*vm)->DetachCurrentThread(vm); }
static bool clear_exception(JNIEnv *env) {
    if (!(*env)->ExceptionCheck(env)) return false;
    (*env)->ExceptionClear(env);
    return true;
}
static jboolean begin(JNIEnv *env, jobject self) { (void)env; (void)self; return nabi_mobile_import_begin(); }
static jboolean document(JNIEnv *env, jobject self, jint fd, jstring name) {
    (void)self;
    const jchar *characters = name ? (*env)->GetStringChars(env, name, NULL) : NULL;
    jsize length = characters ? (*env)->GetStringLength(env, name) : 0;
    bool result = nabi_mobile_import_utf16(fd, characters, (size_t)length);
    if (characters) (*env)->ReleaseStringChars(env, name, characters);
    return result;
}
static void back(JNIEnv *env, jobject self) { (void)env; (void)self; nabi_mobile_back(); }
static void finish(JNIEnv *env, jobject self) { (void)env; (void)self; nabi_mobile_import_finish(); }
static void error(JNIEnv *env, jobject self) {
    (void)env; (void)self;
    nabi_mobile_import_error("Could not read the selected document.");
}
static void insets(JNIEnv *env, jobject self, jfloat top, jfloat right, jfloat bottom, jfloat left) {
    (void)env; (void)self;
    atomic_store(&safe_area[0], top); atomic_store(&safe_area[1], right);
    atomic_store(&safe_area[2], bottom); atomic_store(&safe_area[3], left);
}

void nabi_android_init(void *java_vm, void *native_activity) {
    vm = (JavaVM *)java_vm;
    bool attached;
    JNIEnv *env = environment(&attached);
    if (!env) return;
    activity = (*env)->NewGlobalRef(env, (jobject)native_activity);
    jclass cls = (*env)->GetObjectClass(env, activity);
    JNINativeMethod methods[] = {
        {"nativeBegin", "()Z", (void *)begin},
        {"nativeDocument", "(ILjava/lang/String;)Z", (void *)document},
        {"nativeFinish", "()V", (void *)finish},
        {"nativeError", "()V", (void *)error},
        {"nativeBack", "()V", (void *)back},
        {"nativeInsets", "(FFFF)V", (void *)insets},
    };
    (*env)->RegisterNatives(env, cls, methods, sizeof(methods) / sizeof(methods[0]));
    pick_method = (*env)->GetMethodID(env, cls, "pickImages", "()V");
    copy_method = (*env)->GetMethodID(env, cls, "copyText", "(Ljava/lang/String;)V");
    exit_method = (*env)->GetMethodID(env, cls, "closeViewer", "()V");
    awake_method = (*env)->GetMethodID(env, cls, "keepAwake", "(Z)V");
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
    bool attached; JNIEnv *env = environment(&attached);
    if (!env || length < 0) return;
    jstring value = (*env)->NewString(env, text, length);
    if (value && copy_method) (*env)->CallVoidMethod(env, activity, copy_method, value);
    if (value) (*env)->DeleteLocalRef(env, value);
    clear_exception(env); release(attached);
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
