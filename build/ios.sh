#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
mode=run
simulator=
simulator_family=any
xcode=no
device=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --build-only) mode=build ;;
        --list) mode=list ;;
        --xcode) xcode=yes; mode=build ;;
        --device) device=yes; mode=build ;;
        --phone) simulator_family=iPhone ;;
        --tablet) simulator_family=iPad ;;
        --help|-h)
            printf '%s\n' 'Usage: sh build/ios.sh [--build-only | --device | --list] [--simulator UUID] [--phone | --tablet]' \
                '  All builds support both iPhone and iPad; --phone/--tablet select the simulator only.' \
                '  By default, use an available iPhone or iPad simulator, preferring a booted device.' \
                '  --device builds an unsigned arm64 iPhone/iPad app; install using Xcode signing.' \
                '  Store archives/exports: sh build/ios-store.sh --help'
            exit 0 ;;
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
[ "$device" = no ] || sdk=iphoneos
case "${PLATFORM_NAME:-iphonesimulator}" in iphoneos) [ "$xcode" != yes ] || sdk=iphoneos ;; esac
if [ "$sdk" = iphoneos ]; then
    target=aarch64-apple-ios
else
    case "${NATIVE_ARCH_ACTUAL:-$(uname -m)}" in x86_64) target=x86_64-apple-ios ;; *) target=aarch64-apple-ios-sim ;; esac
fi
rustup target list --installed | grep -Fx "$target" >/dev/null || fail "Install the Rust target first: rustup target add $target"
command -v python3 >/dev/null || fail 'Python 3 is required.'
if [ "$mode" = run ]; then
    # Match device types, including simulators the user has renamed. Never create a simulator.
    selection=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
family, requested = sys.argv[1:]
def device_family(device):
    kind = device.get("deviceTypeIdentifier", "").rsplit(".", 1)[-1]
    return next((value for value in ("iPhone", "iPad")
                 if (kind or device["name"]).startswith(value)), None)
devices = [d for runtime, rows in json.load(sys.stdin)["devices"].items()
           if ".iOS-" in runtime for d in rows
           if d.get("isAvailable") and device_family(d) is not None
           and (family == "any" or device_family(d) == family)
           and (not requested or d["udid"] == requested)]
devices.sort(key=lambda d: d["state"] != "Booted")
if not devices:
    label = "iPhone/iPad" if family == "any" else family
    raise SystemExit(f"No available {label} simulator matches the selection. Add an iOS runtime/device in Xcode or use --simulator UUID.")
print(devices[0]["udid"], devices[0]["state"])
' "$simulator_family" "$simulator")
    simulator=${selection%% *}
    simulator_state=${selection#* }
fi
if ! command -v cmake >/dev/null 2>&1; then
    PATH="$project_dir/ai/runtime/cmake/cmake/data/bin:$PATH"
    export PATH
fi
command -v cmake >/dev/null || fail 'CMake is required to build the AVIF decoder.'
export IPHONEOS_DEPLOYMENT_TARGET=${IPHONEOS_DEPLOYMENT_TARGET:-15.0}
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
    /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier ${NABI_IOS_BUNDLE_ID:-dev.nabi.view}" "$app/Info.plist"
    version=$(python3 -c 'import pathlib,re; print(re.search(r"^version = \"([^\"]+)\"", pathlib.Path("Cargo.toml").read_text(), re.M)[1])')
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${NABI_BUILD_NUMBER:-1}" "$app/Info.plist"
    xcrun actool "$project_dir/platforms/ios/Assets.xcassets" --compile "$app" \
        --platform "$sdk" --minimum-deployment-target "$IPHONEOS_DEPLOYMENT_TARGET" \
        --app-icon AppIcon --target-device iphone --target-device ipad \
        --output-partial-info-plist "$app/icon-info.plist"
    python3 - "$app" <<'PY'
import pathlib, plistlib, sys
app = pathlib.Path(sys.argv[1])
info = plistlib.loads((app / "Info.plist").read_bytes())
info.update(plistlib.loads((app / "icon-info.plist").read_bytes()))
(app / "Info.plist").write_bytes(plistlib.dumps(info))
(app / "icon-info.plist").unlink()
PY
    cp platforms/ios/PrivacyInfo.xcprivacy "$app/"
    [ "$sdk" = iphoneos ] || codesign --force --sign - "$app"
fi
printf 'App: %s\n' "$app"
[ "$mode" = run ] || exit 0

[ "$simulator_state" = Booted ] || xcrun simctl boot "$simulator"
xcrun simctl bootstatus "$simulator" -b
if [ -d "$DEVELOPER_DIR/../Applications/DeviceHub.app" ]; then
    open "$DEVELOPER_DIR/../Applications/DeviceHub.app"
else
    open "$DEVELOPER_DIR/Applications/Simulator.app" --args -CurrentDeviceUDID "$simulator"
fi
xcrun simctl install "$simulator" "$app"
xcrun simctl launch --terminate-running-process "$simulator" "${NABI_IOS_BUNDLE_ID:-dev.nabi.view}"
