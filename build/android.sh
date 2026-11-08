#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
mode=run
abi=arm64-v8a
serial=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --build-only) mode=build ;;
        --native-only) mode=native ;;
        --abi) [ "$#" -ge 2 ] || fail '--abi requires a value'; abi=$2; shift ;;
        --serial) [ "$#" -ge 2 ] || fail '--serial requires an adb serial'; serial=$2; shift ;;
        *) fail "Unknown option: $1" ;;
    esac
    shift
done
case "$abi" in
    arm64-v8a) target=aarch64-linux-android; compiler=aarch64-linux-android30 ;;
    x86_64) target=x86_64-linux-android; compiler=x86_64-linux-android30 ;;
    *) fail 'Supported Android ABIs: arm64-v8a, x86_64' ;;
esac
: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
: "${ANDROID_NDK_HOME:=${ANDROID_NDK_ROOT:-}}"
if [ -z "$ANDROID_NDK_HOME" ]; then
    for directory in "$ANDROID_HOME"/ndk/*; do
        [ ! -d "$directory/toolchains/llvm" ] || ANDROID_NDK_HOME=$directory
    done
fi
if [ -z "$ANDROID_NDK_HOME" ]; then
    fail 'Set ANDROID_NDK_HOME to an installed NDK (r28 or newer recommended). Install Android SDK Platform 35, Build Tools 35 and NDK through Android Studio.'
fi
export ANDROID_HOME ANDROID_NDK_HOME
rustup target list --installed | grep -Fx "$target" >/dev/null || fail "Install the Rust target first: rustup target add $target"
if [ "$mode" != native ]; then
    command -v gradle >/dev/null || fail 'Install Gradle 8.11.1 and JDK 17, or open platforms/android in Android Studio.'
fi
command -v python3 >/dev/null || fail 'Python 3 is required.'
if ! command -v cmake >/dev/null 2>&1; then PATH="$project_dir/ai/runtime/cmake/cmake/data/bin:$PATH"; export PATH; fi
command -v cmake >/dev/null || fail 'CMake is required.'
prebuilt=
for directory in "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*; do
    [ ! -x "$directory/bin/$compiler-clang" ] || prebuilt=$directory
done
[ -n "$prebuilt" ] || fail 'No compatible NDK clang toolchain found.'
target_key=$(printf '%s' "$target" | tr '-' '_')
target_upper=$(printf '%s' "$target_key" | tr '[:lower:]' '[:upper:]')
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in /*) ;; *) target_dir="$project_dir/$target_dir" ;; esac
env "CC_$target_key=$prebuilt/bin/$compiler-clang" \
    "CXX_$target_key=$prebuilt/bin/$compiler-clang++" \
    "AR_$target_key=$prebuilt/bin/llvm-ar" \
    "CARGO_TARGET_${target_upper}_LINKER=$prebuilt/bin/$compiler-clang" \
    "CARGO_TARGET_${target_upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384" \
    "CMAKE_TOOLCHAIN_FILE=$project_dir/build/android-toolchain.cmake" "NABI_ANDROID_ABI=$abi" \
    cargo build --locked --release -p nabi-view-android --target "$target"
mkdir -p "target/android/jniLibs/$abi" target/android/assets target/android/res/mipmap-xxxhdpi
cp "$target_dir/$target/release/libnabi_view_android.so" "target/android/jniLibs/$abi/"
python3 build/notices.py
cp target/notices/THIRD_PARTY_NOTICES.txt target/android/assets/
cp LICENSE THIRD_PARTY.md target/android/assets/
cp assets/icons/png/nabi-icon-256.png target/android/res/mipmap-xxxhdpi/ic_launcher.png
[ "$mode" != native ] || exit 0
# Keep Gradle's local output and cache inside the ignored target directory.
gradle -p platforms/android --project-cache-dir "$project_dir/target/android/gradle-cache" \
    "-PnabiAbi=$abi" -PnabiNativePrepared=true :app:assembleDebug
apk="$project_dir/target/android/gradle/app/outputs/apk/debug/app-debug.apk"
printf 'APK: %s\n' "$apk"
[ "$mode" = run ] || exit 0
adb="$ANDROID_HOME/platform-tools/adb"
[ -x "$adb" ] || fail 'Install Android SDK Platform Tools.'
if [ -n "$serial" ]; then
    "$adb" -s "$serial" install -r "$apk"
    "$adb" -s "$serial" shell am start -n dev.nabi.view/.MainActivity
else
    "$adb" install -r "$apk"
    "$adb" shell am start -n dev.nabi.view/.MainActivity
fi
