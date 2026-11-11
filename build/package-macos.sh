#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: sh build/package-macos.sh [--target TARGET] [--dmg | --dmg-only] [--no-build] [--dry-run] [--print-output-dir]

TARGET: aarch64-apple-darwin | x86_64-apple-darwin | universal
Without --target, build the Rust host architecture.
Output: <Cargo target directory>/package/<target>/NABI VIEW.app.
Universal uses the universal-apple-darwin directory. CARGO_TARGET_DIR is respected.
DMG output: <Cargo target directory>/dist/NABI-VIEW-macos-{arm64,x64,universal}.dmg.

Building requires macOS, Rust, Python 3.9+, CMake, Perl and Apple Command Line Tools.
Intel/Universal compilation also requires NASM for the bundled AVIF codec.
Install missing Rust targets with rustup target add <target>.
Set PYTHON to a Python executable path to override python3/python detection.
The bundle is ad-hoc signed for local use, not notarized or App Store signed.
--dmg also creates a styled DMG in target/dist/ (or $CARGO_TARGET_DIR/dist/).
--no-build packages existing release binaries without compiling.
App packaging fetches locked crate sources as needed for complete license notices.
--dmg-only creates only the DMG from the existing app, leaving the app untouched.
DMG creation writes its layout directly; no disk mount or Finder automation is used.
--dry-run prints the build plan without building or changing files.
--print-output-dir prints only the resolved output directory, without building.
EOF
}

target=''
dry_run=false
dmg=false
dmg_only=false
no_build=false
print_output=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --target)
            [ "$#" -ge 2 ] && [ -n "$2" ] || { echo '--target requires a value.' >&2; exit 2; }
            [ -z "$target" ] || { echo 'Specify --target only once.' >&2; exit 2; }
            target=$2
            shift 2
            ;;
        --dry-run) dry_run=true; shift ;;
        --dmg) dmg=true; shift ;;
        --dmg-only) dmg=true; dmg_only=true; no_build=true; shift ;;
        --no-build) no_build=true; shift ;;
        --print-output-dir) print_output=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done
case "$target" in
    ''|aarch64-apple-darwin|x86_64-apple-darwin|universal) ;;
    *) echo "Unsupported macOS target: $target" >&2; exit 2 ;;
esac

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ -z "$target" ]; then
    command -v rustc >/dev/null 2>&1 || { echo 'Rust is required.' >&2; exit 1; }
    target=$(rustc -vV | sed -n 's/^host: //p' | tr -d '\r')
    case "$target" in
        aarch64-apple-darwin|x86_64-apple-darwin) ;;
        *) echo 'macOS is required. Use --target with --dry-run to preview a macOS build.' >&2; exit 1 ;;
    esac
fi
if [ "$target" = universal ]; then
    targets='aarch64-apple-darwin x86_64-apple-darwin'
else
    targets=$target
fi

python=${PYTHON:-}
if [ -n "$python" ]; then
    "$python" -c 'import sys; sys.exit(sys.version_info < (3, 9))' >/dev/null 2>&1 || {
        echo 'PYTHON must name a Python 3.9+ executable (without arguments).' >&2
        exit 1
    }
else
    for candidate in python3 python; do
        if command -v "$candidate" >/dev/null 2>&1 &&
            "$candidate" -c 'import sys; sys.exit(sys.version_info < (3, 9))' >/dev/null 2>&1; then
            python=$candidate
            break
        fi
    done
    [ -n "$python" ] || { echo 'Python 3.9+ is required. Set PYTHON to its executable path.' >&2; exit 1; }
fi
cargo_metadata=$(cargo metadata --offline --format-version=1 --no-deps --locked)
build_dir=$(printf '%s' "$cargo_metadata" | "$python" -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
version=$(printf '%s' "$cargo_metadata" | "$python" -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "nabi-view"))')
package_target=$target
[ "$target" != universal ] || package_target=universal-apple-darwin
output_dir="$build_dir/package/$package_target"
output="$output_dir/NABI VIEW.app"
case "$target" in
    aarch64-apple-darwin) dmg_arch=arm64 ;;
    x86_64-apple-darwin) dmg_arch=x64 ;;
    universal) dmg_arch=universal ;;
esac
dmg_dir="$build_dir/dist"
dmg_name="NABI-VIEW-macos-$dmg_arch.dmg"
dmg_output="$dmg_dir/$dmg_name"
if [ "$print_output" = true ]; then
    printf '%s\n' "$output_dir"
    exit 0
fi
if [ "$dry_run" = true ]; then
    echo 'Build host: macOS (required for Apple SDK, signing and disk images)'
    if [ "$dmg_only" = true ]; then
        printf 'Use existing app without rebuilding or replacing it: %s\n' "$output"
    else
        if [ "$no_build" = false ]; then
            echo 'cargo fetch --locked'
            for arch in $targets; do
                printf 'cargo build --locked --release --bin nabi-view --target %s\n' "$arch"
                printf 'cargo build --locked --release -p nabi-preview --target %s\n' "$arch"
            done
        else
            for arch in $targets; do
                printf 'Use existing release binary: %s/%s/release/nabi-view\n' "$build_dir" "$arch"
                printf 'Use existing preview library: %s/%s/release/libnabi_preview.dylib\n' "$build_dir" "$arch"
            done
            echo 'cargo fetch --locked (license notice sources)'
        fi
        printf '"%s" build/notices.py\n' "$python"
        [ "$target" != universal ] || echo 'lipo -create <arm64 binary> <x86_64 binary> -output <bundle executable>'
        echo 'Build and embed Quick Look thumbnail/preview extensions with the shared decoder.'
        printf 'Bundle resources, ad-hoc sign and verify: %s\n' "$output"
    fi
    if [ "$dmg" = true ]; then
        echo 'Write Retina artwork and Finder layout; create HFS+ image without mounting; verify the embedded app; compress to UDZO.'
        printf 'Verify styled drag-to-Applications disk image: %s\n' "$dmg_output"
    fi
    exit 0
fi

[ "$(uname -s)" = Darwin ] || { echo 'macOS is required for Apple SDK, linking and codesign.' >&2; exit 1; }
required_tools='codesign'
if [ "$dmg" = true ] || [ "$target" = universal ]; then
    required_tools="$required_tools xcrun"
fi
if [ "$no_build" = false ]; then
    required_tools="$required_tools cargo rustc cmake perl xcrun"
fi
for tool in $required_tools; do
    executable=$tool
    [ "$tool" != cmake ] || executable=${CMAKE:-cmake}
    command -v "$executable" >/dev/null 2>&1 || { echo "Required tool not found: $executable" >&2; exit 1; }
done
if [ "$dmg" = true ]; then
    for tool in hdiutil; do
        command -v "$tool" >/dev/null 2>&1 || { echo "Required DMG tool not found: $tool" >&2; exit 1; }
    done
fi
if [ "$no_build" = false ] || [ "$dmg" = true ]; then
    xcrun --sdk macosx --find clang >/dev/null
fi
if [ "$target" = universal ] && [ "$dmg_only" = false ]; then
    xcrun --find lipo >/dev/null
fi
if [ "$no_build" = false ]; then
    if [ "$target" = universal ] || [ "$target" = x86_64-apple-darwin ]; then
        command -v nasm >/dev/null 2>&1 || { echo 'NASM is required for Intel AVIF builds.' >&2; exit 1; }
    fi
    if command -v rustup >/dev/null 2>&1; then
        installed_targets=$(rustup target list --installed)
        for arch in $targets; do
            printf '%s\n' "$installed_targets" | grep -Fxq "$arch" || {
                echo "Missing Rust target. Run: rustup target add $arch" >&2
                exit 1
            }
        done
    fi
    # notices.py resolves all locked platforms offline, so fetch the whole lockfile.
    cargo fetch --locked
    for arch in $targets; do
        cargo build --locked --release --bin nabi-view --target "$arch"
        cargo build --locked --release -p nabi-preview --target "$arch"
    done
fi

validate_app() {
    [ -x "$1/Contents/MacOS/nabi-view" ] || {
        echo "Missing app executable: $1. Build the app first." >&2
        exit 1
    }
    app_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$1/Contents/Info.plist")
    [ "$app_id" = dev.nabi.NabiView ] || { echo "Unexpected bundle at $1" >&2; exit 1; }
    codesign --verify --strict "$1"
}
if [ "$dmg_only" = true ]; then
    validate_app "$output"
else
    for arch in $targets; do
        [ -f "$build_dir/$arch/release/nabi-view" ] || {
            echo "Missing release binary: $build_dir/$arch/release/nabi-view. Build this target first." >&2
            exit 1
        }
    done
    # Even native-only builds can lack the other locked platforms' sources.
    # Fetch those for complete license notices without recompiling anything.
    [ "$no_build" = false ] || cargo fetch --locked
    "$python" build/notices.py
fi
mkdir -p "$output_dir"
staging=$(mktemp -d "$output_dir/.nabi-package.XXXXXXXX")
dmg_staging=''
replacing=false
committed=false
run_dmg_step() {
    dmg_step=$1
    shift
    printf '\n[DMG] %s\n' "$dmg_step"
    if "$@"; then
        return 0
    else
        dmg_status=$?
        printf '\nDMG 생성 실패: %s (종료 코드 %s)\n' "$dmg_step" "$dmg_status" >&2
        printf '실행 명령:' >&2
        printf ' <%s>' "$@" >&2
        printf '\n기존 앱과 DMG는 교체하지 않았습니다.\n' >&2
        exit "$dmg_status"
    fi
}
restore_output() {
    restore_candidate=$1
    restore_backup=$2
    restore_destination=$3
    # A missing candidate means it has already been installed. Otherwise the
    # destination, if present, is still the original and must not be removed.
    if [ ! -e "$restore_candidate" ] && [ -e "$restore_destination" ]; then
        rm -rf "$restore_destination" || return 1
    fi
    if [ -e "$restore_backup" ]; then
        mv "$restore_backup" "$restore_destination" || {
            echo "Previous artifact retained at $restore_backup" >&2
            return 1
        }
    fi
}
cleanup() {
    if [ "$replacing" = true ] && [ "$committed" = false ]; then
        if [ "$dmg_only" = false ]; then
            restore_output "$staging/NABI VIEW.app" "$staging/previous.app" "$output" || return
        fi
        if [ "$dmg" = true ]; then
            restore_output "$dmg_staging/$dmg_name" "$dmg_staging/previous.dmg" "$dmg_output" || return
            restore_output "$staging/dmg-preview.png" "$staging/previous-preview.png" "$output_dir/dmg-preview.png" || return
        fi
    fi
    [ -z "$dmg_staging" ] || rm -rf "$dmg_staging"
    rm -rf "$staging"
}
trap cleanup 0
trap 'exit 130' INT
trap 'exit 143' TERM
if [ "$dmg_only" = true ]; then
    app=$output
else
    app="$staging/NABI VIEW.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 644 platforms/macos/Info.plist "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
    if [ "$target" = universal ]; then
        xcrun lipo -create \
            "$build_dir/aarch64-apple-darwin/release/nabi-view" \
            "$build_dir/x86_64-apple-darwin/release/nabi-view" \
            -output "$app/Contents/MacOS/nabi-view"
        chmod 755 "$app/Contents/MacOS/nabi-view"
        xcrun lipo -verify_arch arm64 x86_64 "$app/Contents/MacOS/nabi-view"
    else
        install -m 755 "$build_dir/$target/release/nabi-view" "$app/Contents/MacOS/nabi-view"
    fi
    "$python" build/package_resources.py --macos "$app/Contents/Resources"
    install -m 644 target/notices/THIRD_PARTY_NOTICES.txt "$app/Contents/Resources/THIRD_PARTY_NOTICES.txt"
    install -m 644 LICENSE THIRD_PARTY.md "$app/Contents/Resources/"
    "$python" build/package-previews.py macos --target "$target" --app "$app" --no-build
    codesign --force --sign - "$app"
    codesign --verify --strict "$app"
fi
if [ "$dmg" = true ]; then
    # Publish from a temporary directory beside the final DMG, so replacing
    # it is an atomic rename even when CARGO_TARGET_DIR is on another disk.
    mkdir -p "$dmg_dir"
    dmg_staging=$(mktemp -d "$dmg_dir/.nabi-dmg.XXXXXXXX")
    mkdir -p "$staging/dmg/.background" "$staging/verification"
    ditto "$app" "$staging/dmg/NABI VIEW.app"
    ln -s /Applications "$staging/dmg/Applications"
    xcrun clang -fobjc-arc -framework Cocoa build/macos/dmg-artwork.m -o "$staging/dmg-artwork"
    "$staging/dmg-artwork" "$staging/dmg/.background/background.tiff" \
        "$staging/dmg-preview.png" assets/icons/png/nabi-icon-1024.png
    # Build a file-backed HFS+ image directly through DiscRecording. Nothing
    # is attached as a device, and the Finder layout is written in advance.
    run_dmg_step '1/6 설치 화면 구성 파일 생성' \
        "$python" build/macos/dmg-layout.py "$staging/dmg"
    run_dmg_step '2/6 디스크 연결 없이 이미지 생성' \
        hdiutil makehybrid -hfs -default-volume-name 'NABI VIEW' \
        -o "$staging/uncompressed.dmg" "$staging/dmg"
    # DiscRecording can synthesize Finder metadata for files in the signed
    # bundle. Normalize it and verify the actual image contents before signing
    # off on the extracted app, rather than checking only the source bundle.
    run_dmg_step '3/6 이미지 내부 앱 검증' \
        "$python" build/macos/hfs-image.py "$staging/uncompressed.dmg" \
        "$staging/dmg/NABI VIEW.app" "$staging/verification/NABI VIEW.app"
    run_dmg_step '4/6 이미지에서 꺼낸 앱 서명 검증' \
        codesign --verify --strict "$staging/verification/NABI VIEW.app"
    run_dmg_step '5/6 최종 DMG 압축' \
        hdiutil convert "$staging/uncompressed.dmg" -format UDZO -o "$dmg_staging/$dmg_name"
    run_dmg_step '6/6 최종 DMG 무결성 검사' hdiutil verify "$dmg_staging/$dmg_name"
fi
if [ "$dmg_only" = false ]; then
    if [ -e "$output" ]; then
        # Replace only a previous bundle generated at our output path.
        old_id=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$output/Contents/Info.plist")
        [ "$old_id" = dev.nabi.NabiView ] || { echo "Unexpected bundle at $output" >&2; exit 1; }
    fi
fi
# Keep all previous outputs until the entire replacement succeeds. The trap
# rolls back installed candidates if a later move fails or is interrupted.
replacing=true
if [ "$dmg_only" = false ] && [ -e "$output" ]; then
    mv "$output" "$staging/previous.app"
fi
if [ "$dmg" = true ]; then
    [ ! -e "$dmg_output" ] || mv "$dmg_output" "$dmg_staging/previous.dmg"
    [ ! -e "$output_dir/dmg-preview.png" ] || mv "$output_dir/dmg-preview.png" "$staging/previous-preview.png"
fi
if [ "$dmg_only" = false ]; then
    mv "$app" "$output"
fi
if [ "$dmg" = true ]; then
    mv "$dmg_staging/$dmg_name" "$dmg_output"
    mv "$staging/dmg-preview.png" "$output_dir/dmg-preview.png"
fi
committed=true
if [ "$dmg_only" = false ]; then
    printf '%s\n' "$output"
fi
if [ "$dmg" = true ]; then
    printf 'DMG: %s\n' "$dmg_output"
fi
