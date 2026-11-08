#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
mode=run
simulator=
xcode=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --build-only) mode=build ;;
        --list) mode=list ;;
        --xcode) xcode=yes; mode=build ;;
        --simulator) [ "$#" -ge 2 ] || fail '--simulator requires a device UUID'; simulator=$2; shift ;;
        *) fail "Unknown option: $1" ;;
    esac
    shift
done
[ "$(uname -s)" = Darwin ] || fail 'iOS builds require macOS with Xcode.'
if [ -z "${DEVELOPER_DIR:-}" ]; then
    DEVELOPER_DIR=$(xcode-select -p)
    if [ ! -d "$DEVELOPER_DIR/Platforms/iPhoneSimulator.platform" ] && [ -d /Applications/Xcode.app ]; then
        DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    fi
fi
export DEVELOPER_DIR
[ -d "$DEVELOPER_DIR/Platforms/iPhoneSimulator.platform" ] || fail 'Install Xcode and its iOS platform support first.'
if [ "$mode" = list ]; then exec xcrun simctl list devices available; fi

sdk=iphonesimulator
case "${PLATFORM_NAME:-iphonesimulator}" in iphoneos) [ "$xcode" != yes ] || sdk=iphoneos ;; esac
if [ "$sdk" = iphoneos ]; then
    target=aarch64-apple-ios
else
    case "${NATIVE_ARCH_ACTUAL:-$(uname -m)}" in x86_64) target=x86_64-apple-ios ;; *) target=aarch64-apple-ios-sim ;; esac
fi
rustup target list --installed | grep -Fx "$target" >/dev/null || fail "Install the Rust target first: rustup target add $target"
command -v python3 >/dev/null || fail 'Python 3 is required.'
if ! command -v cmake >/dev/null 2>&1; then
    PATH="$project_dir/ai/runtime/cmake/cmake/data/bin:$PATH"
    export PATH
fi
command -v cmake >/dev/null || fail 'CMake is required to build the AVIF decoder.'
export IPHONEOS_DEPLOYMENT_TARGET=15.0
SDKROOT=$(xcrun --sdk "$sdk" --show-sdk-path)
export SDKROOT
target_dir=${CARGO_TARGET_DIR:-"$project_dir/target"}
case "$target_dir" in /*) ;; *) target_dir="$project_dir/$target_dir" ;; esac
cargo build --locked --release --bin nabi-view --target "$target"
python3 build/notices.py
if [ "$xcode" = yes ]; then
    app="$TARGET_BUILD_DIR/$WRAPPER_NAME"
else
    app="$target_dir/mobile/$target/NABI VIEW.app"
fi
mkdir -p "$app"
cp "$target_dir/$target/release/nabi-view" "$app/nabi-view"
cp target/notices/THIRD_PARTY_NOTICES.txt "$app/THIRD_PARTY_NOTICES.txt"
cp LICENSE THIRD_PARTY.md "$app/"
if [ "$xcode" != yes ]; then
    cp platforms/ios/Info.plist "$app/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable nabi-view' "$app/Info.plist"
    /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier dev.nabi.view' "$app/Info.plist"
    codesign --force --sign - "$app"
fi
printf 'App: %s\n' "$app"
[ "$mode" = run ] || exit 0

# Select an existing iPhone. Never create or replace a user's simulator.
if [ -z "$simulator" ]; then
    simulator=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = [d for key, rows in json.load(sys.stdin)["devices"].items() if ".iOS-" in key for d in rows if d.get("isAvailable") and d["name"].startswith("iPhone")]
devices.sort(key=lambda d: d["state"] != "Booted")
print(devices[0]["udid"] if devices else "")')
fi
[ -n "$simulator" ] || fail 'No iPhone simulator is installed. Add an iOS runtime and iPhone in Xcode > Open Developer Tool > Device Hub.'
state=$(xcrun simctl list devices available -j | python3 -c '
import json,sys
print(next((d["state"] for rows in json.load(sys.stdin)["devices"].values() for d in rows if d["udid"] == sys.argv[1]), "missing"))' "$simulator")
[ "$state" != missing ] || fail 'The selected simulator is unavailable.'
[ "$state" = Booted ] || xcrun simctl boot "$simulator"
xcrun simctl bootstatus "$simulator" -b
if [ -d "$DEVELOPER_DIR/../Applications/DeviceHub.app" ]; then
    open "$DEVELOPER_DIR/../Applications/DeviceHub.app"
else
    open "$DEVELOPER_DIR/Applications/Simulator.app" --args -CurrentDeviceUDID "$simulator"
fi
xcrun simctl install "$simulator" "$app"
xcrun simctl launch --terminate-running-process "$simulator" dev.nabi.view
