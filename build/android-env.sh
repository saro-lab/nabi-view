#!/bin/sh
# Optional project-local Android tools. Explicit environment values take priority.
# Source this file and call nabi_android_environment PROJECT_ROOT, or run it with
# a command: sh build/android-env.sh sdkmanager --list_installed
nabi_android_environment() {
    nabi_env_root=$1
    nabi_env_runtime=$nabi_env_root/ai/runtime
    nabi_env_local=no

    if [ -d "$nabi_env_runtime/android-sdk" ]; then
        nabi_env_local=yes
        if [ -z "${ANDROID_HOME:-}" ] && [ -z "${ANDROID_SDK_ROOT:-}" ]; then
            ANDROID_HOME=$nabi_env_runtime/android-sdk
            export ANDROID_HOME
        fi
    fi
    nabi_env_sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
    if [ -n "$nabi_env_sdk" ]; then
        if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -z "${ANDROID_NDK_ROOT:-}" ] \
            && [ -d "$nabi_env_sdk/ndk/28.2.13676358" ]; then
            ANDROID_NDK_HOME=$nabi_env_sdk/ndk/28.2.13676358
            export ANDROID_NDK_HOME
        fi
        # Use tools from the chosen SDK when those commands are not already set up.
        for nabi_env_tools in "$nabi_env_sdk/cmdline-tools/22.0/bin" "$nabi_env_sdk/cmdline-tools/latest/bin"; do
            if [ -x "$nabi_env_tools/sdkmanager" ] && ! command -v sdkmanager >/dev/null 2>&1; then
                PATH=$nabi_env_tools:$PATH
            fi
        done
        if [ -x "$nabi_env_sdk/platform-tools/adb" ] && ! command -v adb >/dev/null 2>&1; then
            PATH=$nabi_env_sdk/platform-tools:$PATH
        fi
        if [ -x "$nabi_env_sdk/emulator/emulator" ] && ! command -v emulator >/dev/null 2>&1; then
            PATH=$nabi_env_sdk/emulator:$PATH
        fi
        # Platform-tools 37.0.1's mDNS server aborts in libunwind on this
        # Apple Silicon setup. Direct emulator/USB connections still work.
        # Scope the workaround to this local SDK; explicit choices win.
        if [ -z "${ADB_MDNS+x}" ] \
            && [ "$nabi_env_sdk" = "$nabi_env_runtime/android-sdk" ] \
            && [ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] \
            && grep -Eq '^Pkg.Revision[[:space:]]*=[[:space:]]*37[.]0[.]1[[:space:]]*$' \
                "$nabi_env_sdk/platform-tools/source.properties" 2>/dev/null; then
            ADB_MDNS=0
            export ADB_MDNS
        fi
    fi

    if [ -d "$nabi_env_runtime/temurin-17/Contents/Home" ]; then
        nabi_env_local=yes
        if [ -z "${JAVA_HOME:-}" ]; then
            JAVA_HOME=$nabi_env_runtime/temurin-17/Contents/Home
            export JAVA_HOME
        fi
    fi
    if [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/java" ]; then
        PATH=$JAVA_HOME/bin:$PATH
    fi
    for nabi_env_tool in gradle-8.11.1/bin/gradle nasm-2.16.03/bin/nasm cmake/cmake/data/bin/cmake; do
        if [ -x "$nabi_env_runtime/$nabi_env_tool" ]; then
            nabi_env_local=yes
            if ! command -v "${nabi_env_tool##*/}" >/dev/null 2>&1; then
                PATH=$nabi_env_runtime/${nabi_env_tool%/*}:$PATH
            fi
        fi
    done
    export PATH

    # Ordinary SDK installations keep their normal defaults. Only the optional
    # local tool setup puts new caches, AVDs and temporary files under ai/.
    if [ "$nabi_env_local" = yes ]; then
        if [ -z "${GRADLE_USER_HOME:-}" ]; then
            GRADLE_USER_HOME=$nabi_env_root/ai/cache/gradle
            mkdir -p "$GRADLE_USER_HOME" || return 1
            export GRADLE_USER_HOME
        fi
        if [ -z "${ANDROID_USER_HOME:-}" ]; then
            ANDROID_USER_HOME=$nabi_env_runtime/android-user
            mkdir -p "$ANDROID_USER_HOME" || return 1
            export ANDROID_USER_HOME
        fi
        # Native emulator tools do not consistently honor ANDROID_USER_HOME.
        # Their documented override is ANDROID_EMULATOR_HOME.
        if [ -z "${ANDROID_EMULATOR_HOME:-}" ]; then
            ANDROID_EMULATOR_HOME=$ANDROID_USER_HOME
            mkdir -p "$ANDROID_EMULATOR_HOME" || return 1
            export ANDROID_EMULATOR_HOME
        fi
        if [ -z "${ANDROID_AVD_HOME:-}" ]; then
            ANDROID_AVD_HOME=$nabi_env_runtime/android-avd
            mkdir -p "$ANDROID_AVD_HOME" || return 1
            export ANDROID_AVD_HOME
        fi
        if [ -z "${TMPDIR:-}" ]; then
            TMPDIR=$nabi_env_root/ai/tmp/android-environment
            mkdir -p "$TMPDIR" || return 1
            export TMPDIR
        fi
        if [ -z "${ANDROID_ADB_LOG_PATH:-}" ]; then
            mkdir -p "$nabi_env_root/ai/tmp/android-environment" || return 1
            ANDROID_ADB_LOG_PATH=$nabi_env_root/ai/tmp/android-environment/adb.log
            export ANDROID_ADB_LOG_PATH
        fi
        # adb still keeps its default authentication key in $HOME/.android;
        # ADB_VENDOR_KEYS adds keys rather than relocating that default key.
    fi
    unset nabi_env_root nabi_env_runtime nabi_env_local nabi_env_sdk nabi_env_tools nabi_env_tool
}

case "$0" in
    */android-env.sh|android-env.sh)
        set -eu
        case "${1:---help}" in
            --help|-h)
                printf '%s\n' 'Usage: sh build/android-env.sh COMMAND [ARGS...]' \
                    'Use installed project-local Android tools while preserving explicit environment values.' \
                    'Examples: sdkmanager --list_installed, adb devices, emulator -list-avds'
                exit 0 ;;
        esac
        nabi_env_project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
        nabi_android_environment "$nabi_env_project"
        exec "$@"
        ;;
esac
