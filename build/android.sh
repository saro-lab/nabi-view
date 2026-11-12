#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. "$project_dir/build/android-env.sh"
nabi_android_environment "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
# Git for Windows runs this script for mobile.bat; its native tools need Windows paths.
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) windows=true ;; *) windows=false ;; esac
native_path() { if $windows; then cygpath -m "$1"; else printf '%s\n' "$1"; fi; }
mode=run
abi=arm64-v8a
variant=debug
format=apk
require_signing=false
serial=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --build-only) mode=build ;;
        --native-only) mode=native ;;
        --release) variant=release; mode=build ;;
        --bundle) variant=release; format=bundle; mode=build ;;
        --signed) variant=release; require_signing=true; mode=build ;;
        --help|-h)
            printf '%s\n' 'Usage: build/android.sh [--build-only | --native-only] [--abi arm64-v8a|x86_64|all]' \
                '       [--release] [--bundle] [--signed] [--serial DEVICE]' \
                'Default: build and run a debug APK. --release/--bundle only build; --signed requires release credentials.'
            exit 0 ;;
        --abi) [ "$#" -ge 2 ] || fail '--abi requires a value'; abi=$2; shift ;;
        --serial) [ "$#" -ge 2 ] || fail '--serial requires an adb serial'; serial=$2; shift ;;
        *) fail "Unknown option: $1" ;;
    esac
    shift
done
if $require_signing; then
    [ -n "${NABI_ANDROID_KEYSTORE:-}" ] && [ -n "${NABI_ANDROID_KEYSTORE_PASSWORD:-}" ] && \
        [ -n "${NABI_ANDROID_KEY_ALIAS:-}" ] && [ -n "${NABI_ANDROID_KEY_PASSWORD:-}" ] || \
        fail '--signed requires NABI_ANDROID_KEYSTORE, NABI_ANDROID_KEYSTORE_PASSWORD, NABI_ANDROID_KEY_ALIAS and NABI_ANDROID_KEY_PASSWORD.'
fi
case "$abi" in
    arm64-v8a) target=aarch64-linux-android; compiler=aarch64-linux-android30 ;;
    x86_64) target=x86_64-linux-android; compiler=x86_64-linux-android30 ;;
    all) target=; compiler= ;;
    *) fail 'Supported Android ABIs: arm64-v8a, x86_64, all' ;;
esac
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
if [ -z "$sdk" ]; then
    if $windows && [ -n "${LOCALAPPDATA:-}" ]; then sdk=$LOCALAPPDATA/Android/Sdk
    elif [ "$(uname -s)" = Darwin ]; then sdk=$HOME/Library/Android/sdk
    else sdk=$HOME/Android/Sdk
    fi
fi
ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
if $windows; then
    sdk=$(cygpath -u "$sdk")
    [ -z "$ndk" ] || ndk=$(cygpath -u "$ndk")
fi
if [ -z "$ndk" ]; then
    for directory in "$sdk"/ndk/*; do
        [ ! -d "$directory/toolchains/llvm" ] || ndk=$directory
    done
fi
if [ -z "$ndk" ]; then
    fail 'Set ANDROID_NDK_HOME to an installed NDK (r28 or newer recommended). Install Android SDK Platform 36, Build Tools 35 and NDK through Android Studio.'
fi
ANDROID_HOME=$(native_path "$sdk")
ANDROID_NDK_HOME=$(native_path "$ndk")
export ANDROID_HOME ANDROID_NDK_HOME
if [ "$abi" != all ]; then
    rustup target list --installed | grep -Fx "$target" >/dev/null || fail "Install the Rust target first: rustup target add $target"
fi
if [ "$mode" != native ]; then
    command -v gradle >/dev/null || fail 'Install Gradle 8.11.1 and JDK 17, or open platforms/android in Android Studio.'
fi
# Windows' python3/python may be Microsoft Store stubs; the py launcher is reliable there.
if python3 -c 'import sys' >/dev/null 2>&1; then python=python3
elif python -c 'import sys; sys.exit(sys.version_info < (3,))' >/dev/null 2>&1; then python=python
elif py -3 -c 'import sys' >/dev/null 2>&1; then python='py -3'
else fail 'Python 3 is required.'
fi
if ! command -v cmake >/dev/null 2>&1; then PATH="$project_dir/ai/runtime/cmake/cmake/data/bin:$PATH"; export PATH; fi
command -v cmake >/dev/null || fail 'CMake is required.'
# Keep per-ABI builds sequential: they share generated notices and resources.
if [ "$abi" = all ]; then
    sh "$project_dir/build/android.sh" --native-only --abi arm64-v8a
    sh "$project_dir/build/android.sh" --native-only --abi x86_64
else
    prebuilt=
    for directory in "$ndk"/toolchains/llvm/prebuilt/*; do
        if $windows; then
            [ ! -e "$directory/bin/$compiler-clang.cmd" ] || [ ! -e "$directory/bin/clang.exe" ] || prebuilt=$directory
        else
            [ ! -x "$directory/bin/$compiler-clang" ] || prebuilt=$directory
        fi
    done
    [ -n "$prebuilt" ] || fail 'No compatible NDK clang toolchain found.'
    target_key=$(printf '%s' "$target" | tr '-' '_')
    target_upper=$(printf '%s' "$target_key" | tr '[:lower:]' '[:upper:]')
    target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
    ! $windows || target_dir=$(cygpath -u "$target_dir")
    case "$target_dir" in /*) ;; *) target_dir="$project_dir/$target_dir" ;; esac
    link_flags='-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384'
    if $windows; then
        # The NDK's .cmd wrappers are limited by cmd.exe's command-line length,
        # so call clang directly with the same target the wrapper would add.
        cc=$(native_path "$prebuilt/bin/clang.exe")
        set -- "CC_$target_key=$cc" \
            "CXX_$target_key=$(native_path "$prebuilt/bin/clang++.exe")" \
            "AR_$target_key=$(native_path "$prebuilt/bin/llvm-ar.exe")" \
            "CFLAGS_$target_key=--target=$compiler" "CXXFLAGS_$target_key=--target=$compiler"
        link_flags="-C link-arg=--target=$compiler $link_flags"
        # Visual Studio generators cannot target Android; the SDK's CMake package ships Ninja.
        if [ -z "${CMAKE_GENERATOR:-}" ]; then
            if ! command -v ninja >/dev/null 2>&1; then
                for directory in "$sdk"/cmake/*/bin; do
                    [ ! -e "$directory/ninja.exe" ] || PATH="$directory:$PATH"
                done
                export PATH
            fi
            command -v ninja >/dev/null 2>&1 || fail 'Install Ninja, for example with Android Studio SDK Manager > SDK Tools > CMake.'
            CMAKE_GENERATOR=Ninja
            export CMAKE_GENERATOR
        fi
    else
        cc=$prebuilt/bin/$compiler-clang
        set -- "CC_$target_key=$cc" \
            "CXX_$target_key=$prebuilt/bin/$compiler-clang++" \
            "AR_$target_key=$prebuilt/bin/llvm-ar"
    fi
    env "$@" \
        "CARGO_TARGET_${target_upper}_LINKER=$cc" \
        "CARGO_TARGET_${target_upper}_RUSTFLAGS=$link_flags" \
        "CMAKE_TOOLCHAIN_FILE=$(native_path "$project_dir/build/android-toolchain.cmake")" "NABI_ANDROID_ABI=$abi" \
        cargo build --locked --release -p nabi-view-android --target "$target"
    mkdir -p "target/android/jniLibs/$abi" target/android/assets target/android/res/drawable-nodpi
    $python build/check-android-elf.py "$target_dir/$target/release/libnabi_view_android.so"
    cp "$target_dir/$target/release/libnabi_view_android.so" "target/android/jniLibs/$abi/"
    $python build/notices.py
    cp target/notices/THIRD_PARTY_NOTICES.txt target/android/assets/
    cp LICENSE THIRD_PARTY.md target/android/assets/
    cp assets/icons/png/nabi-icon-1024.png target/android/res/drawable-nodpi/nabi_launcher_artwork.png
    # Remove the previous generated legacy icon when reusing an existing build tree.
    rm -f target/android/res/mipmap-xxxhdpi/ic_launcher.png
fi
[ "$mode" != native ] || exit 0
case "$variant:$format" in
    debug:apk) task=assembleDebug; output=apk/debug/app-debug.apk ;;
    release:bundle) task=bundleRelease; output=bundle/release/app-release.aab ;;
    release:apk)
        task=assembleRelease
        if [ -n "${NABI_ANDROID_KEYSTORE:-}" ]; then output=apk/release/app-release.apk
        else output=apk/release/app-release-unsigned.apk
        fi ;;
esac
# Keep Gradle's local output and cache inside the ignored target directory.
gradle -p platforms/android --project-cache-dir "$(native_path "$project_dir/target/android/gradle-cache")" \
    "-PnabiAbi=$abi" -PnabiNativePrepared=true ":app:$task"
apk="$project_dir/target/android/gradle/app/outputs/$output"
[ -f "$apk" ] || fail 'Gradle did not produce the expected package.'
if [ "$abi" = all ]; then package_abis=arm64-v8a,x86_64; else package_abis=$abi; fi
$python build/check-android-elf.py "$apk" --abis "$package_abis"
printf 'Package: %s\n' "$(native_path "$apk")"
if [ "$variant" = release ]; then
    if [ -n "${NABI_ANDROID_KEYSTORE:-}" ]; then printf '%s\n' 'Release signing configured; verify the certificate before distribution.'
    else printf '%s\n' 'UNSIGNED release: cannot install or submit until signed.'
    fi
fi
[ "$mode" = run ] || exit 0
if $windows; then adb="$sdk/platform-tools/adb.exe"; else adb="$sdk/platform-tools/adb"; fi
[ -x "$adb" ] || fail 'Install Android SDK Platform Tools.'
if [ -n "$serial" ]; then
    "$adb" -s "$serial" install -r "$(native_path "$apk")"
    "$adb" -s "$serial" shell am start -n "${NABI_ANDROID_APPLICATION_ID:-dev.nabi.view}/dev.nabi.view.MainActivity"
else
    "$adb" install -r "$(native_path "$apk")"
    "$adb" shell am start -n "${NABI_ANDROID_APPLICATION_ID:-dev.nabi.view}/dev.nabi.view.MainActivity"
fi
