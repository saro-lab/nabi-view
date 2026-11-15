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
usb=false
native_debug=false
logs=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --build-only) mode=build ;;
        --native-only) mode=native ;;
        --debug) native_debug=true ;;
        --usb) usb=true ;;
        --list) mode=list ;;
        --logs) logs=true ;;
        --release) variant=release; mode=build ;;
        --bundle) variant=release; format=bundle; mode=build ;;
        --signed) variant=release; require_signing=true; mode=build ;;
        --help|-h)
            printf '%s\n' 'Usage: build/android.sh [--build-only | --native-only] [--abi arm64-v8a|x86_64|all]' \
                '       [--debug] [--usb | --serial DEVICE] [--logs] [--list]' \
                '       [--release] [--bundle] [--signed]' \
                'Default: debug APK with optimized Rust. --debug enables Rust source/variable debugging.' \
                '--usb selects the single USB device, ignoring emulators. --list lists adb devices.' \
                '--logs streams only the launched app process; Ctrl-C stops log viewing.' \
                '--release/--bundle only build; --signed requires release credentials.'
            exit 0 ;;
        --abi) [ "$#" -ge 2 ] || fail '--abi requires a value'; abi=$2; shift ;;
        --serial) [ "$#" -ge 2 ] || fail '--serial requires an adb serial'; serial=$2; shift ;;
        *) fail "Unknown option: $1" ;;
    esac
    shift
done
! $usb || [ -z "$serial" ] || fail 'Use either --usb or --serial, not both.'
! $native_debug || [ "$variant" = debug ] || fail '--debug cannot be combined with --release, --bundle or --signed.'
! $logs || [ "$mode" = run ] || fail '--logs requires an install/run operation.'
profile=release
native_dir=target/android/jniLibs
if $native_debug; then
    profile=mobile-debug
    native_dir=target/android/debug/jniLibs
fi
if $require_signing; then
    [ -n "${NABI_ANDROID_KEYSTORE:-}" ] && [ -n "${NABI_ANDROID_KEYSTORE_PASSWORD:-}" ] && \
        [ -n "${NABI_ANDROID_KEY_ALIAS:-}" ] && [ -n "${NABI_ANDROID_KEY_PASSWORD:-}" ] || \
        fail '--signed requires NABI_ANDROID_KEYSTORE, NABI_ANDROID_KEYSTORE_PASSWORD, NABI_ANDROID_KEY_ALIAS and NABI_ANDROID_KEY_PASSWORD.'
fi
case "$abi" in
    arm64-v8a) target=aarch64-linux-android; compiler=aarch64-linux-android31 ;;
    x86_64) target=x86_64-linux-android; compiler=x86_64-linux-android31 ;;
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
# Device queries and selection do not need an NDK, Gradle or a Rust rebuild.
if $windows; then sdk=$(cygpath -u "$sdk"); adb="$sdk/platform-tools/adb.exe"
else adb="$sdk/platform-tools/adb"
fi
if [ "$mode" = list ]; then
    [ -x "$adb" ] || fail 'Install Android SDK Platform Tools.'
    exec "$adb" devices -l
fi
if [ "$mode" = run ]; then
    [ -x "$adb" ] || fail 'Install Android SDK Platform Tools.'
    if $usb; then
        serial=$("$adb" -d get-serialno) || fail 'Connect one USB device and approve USB debugging on its unlocked screen.'
    elif [ -z "$serial" ]; then
        serial=$("$adb" get-serialno) || fail 'Select one connected device with --usb or --serial ID.'
    fi
    [ -n "$serial" ] && [ "$serial" != unknown ] || fail 'No authorized Android device. Enable USB debugging and accept its authorization prompt.'
    [ "$("$adb" -s "$serial" get-state)" = device ] || fail 'Device is offline or unauthorized; unlock it and allow USB debugging.'
    device_abis=$("$adb" -s "$serial" shell getprop ro.product.cpu.abilist | tr -d '\r')
    if [ "$abi" = all ]; then
        case ",$device_abis," in *",arm64-v8a,"*|*",x86_64,"*) ;; *) fail "No supported 64-bit ABI on device: $device_abis" ;; esac
    else
        case ",$device_abis," in *",$abi,"*) ;; *) fail "Device ABIs: $device_abis. Choose a supported --abi before building." ;; esac
    fi
    device_api=$("$adb" -s "$serial" shell getprop ro.build.version.sdk | tr -d '\r')
    case "$device_api" in ''|*[!0-9]*) fail 'Could not read the Android API level.' ;; esac
    [ "$device_api" -ge 31 ] || fail 'NABI VIEW requires Android 12 (API 31) or newer.'
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
    if $native_debug; then set -- --debug; else set --; fi
    sh "$project_dir/build/android.sh" --native-only --abi arm64-v8a "$@"
    sh "$project_dir/build/android.sh" --native-only --abi x86_64 "$@"
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
        cargo build --locked --profile "$profile" -p nabi-view-android --target "$target"
    mkdir -p "$native_dir/$abi" target/android/assets target/android/res/drawable-nodpi
    $python build/check-android-elf.py "$target_dir/$target/$profile/libnabi_view_android.so"
    cp "$target_dir/$target/$profile/libnabi_view_android.so" "$native_dir/$abi/"
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
    "-PnabiAbi=$abi" "-PnabiRustDebug=$native_debug" -PnabiNativePrepared=true ":app:$task"
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
application_id=${NABI_ANDROID_APPLICATION_ID:-saro.nabi.view}
"$adb" -s "$serial" install -r "$(native_path "$apk")"
# Restart only this app so it runs the freshly installed binary.
"$adb" -s "$serial" shell am force-stop "$application_id"
"$adb" -s "$serial" shell am start -W -n "$application_id/dev.nabi.view.MainActivity"
if $logs; then
    app_pid=$("$adb" -s "$serial" shell pidof -s "$application_id" | tr -d '\r') || fail 'App exited before log attachment. Inspect: adb logcat -b crash -d'
    [ -n "$app_pid" ] || fail 'App process was not found; inspect the crash buffer.'
    printf 'App log (PID %s). Ctrl-C stops viewing.\n' "$app_pid"
    exec "$adb" -s "$serial" logcat --pid="$app_pid" -v threadtime
fi
