#!/bin/sh
# Internal build engine used by the two root launchers.
set -eu

fail() { printf '%s\n' "Error: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "$1 was not found. Install it and add its executable directory to PATH."; }
run() {
    printf '  %s' "$1"
    shift
    printf ' %s' "$@"
    printf '\n'
}

[ "$#" -eq 4 ] || fail 'Use build.sh or build.bat.'
requested=$1
use_cross=$2
action=$3
dry_run=$4
case "$use_cross:$dry_run" in true:true|true:false|false:true|false:false) ;; *) fail 'Invalid build options.' ;; esac
case "$action" in binary|package|full|package-only|dmg-only) ;; *) fail "Unknown build action: $action" ;; esac

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
# Reuse an optional local CMake without changing the user's saved PATH.
if ! command -v cmake >/dev/null 2>&1; then
    local_cmake_bin="$project_dir/ai/runtime/cmake/cmake/data/bin"
    if [ -x "$local_cmake_bin/cmake" ]; then
        PATH="$local_cmake_bin:$PATH"
        export PATH
    fi
fi

need rustc
host=$(rustc -vV | sed -n 's/^host: //p' | tr -d '\r')
[ -n "$host" ] || fail 'Cannot determine the Rust host target.'
case "$requested" in
    native) target=$host ;;
    windows-x64) target=x86_64-pc-windows-msvc ;;
    windows-arm64) target=aarch64-pc-windows-msvc ;;
    windows-gnu) target=x86_64-pc-windows-gnu ;;
    linux-x64) target=x86_64-unknown-linux-gnu ;;
    linux-arm64) target=aarch64-unknown-linux-gnu ;;
    macos-arm64) target=aarch64-apple-darwin ;;
    macos-x64) target=x86_64-apple-darwin ;;
    macos-universal|universal-apple-darwin) target=universal ;;
    *) target=$requested ;;
esac
case "$target" in
    x86_64-pc-windows-msvc|aarch64-pc-windows-msvc|x86_64-pc-windows-gnu|x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|aarch64-apple-darwin|x86_64-apple-darwin|universal) ;;
    *) fail "Unsupported target: $target. Run sh build.sh --help." ;;
esac

builder=cargo
if [ "$use_cross" = true ]; then
    case "$target" in
        x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-pc-windows-gnu) ;;
        *) fail '--cross supports Linux GNU and Windows GNU. Use a macOS/Windows host or GitHub Actions for Apple/MSVC targets.' ;;
    esac
    case "$host" in
        x86_64-unknown-linux-gnu) ;;
        *) fail 'This cross-rs configuration uses an x86_64 Linux host. On Windows, run it inside WSL2 with Linux Rust and Docker integration.' ;;
    esac
    builder=cross
    CROSS_CONFIG=${CROSS_CONFIG:-"$project_dir/build/Cross.toml"}
    export CROSS_CONFIG
else
    case "$target:$host" in
        *-apple-darwin:*-apple-darwin|universal:*-apple-darwin) ;;
        *-apple-darwin:*|universal:*) fail 'Mac builds require macOS + Apple SDK in this project. Use the desktop-build GitHub Actions workflow.' ;;
        *-pc-windows-msvc:*-pc-windows-msvc) ;;
        *-pc-windows-msvc:*) fail 'MSVC builds require Windows + C++ Build Tools. Use windows-gnu --cross on Linux, or GitHub Actions.' ;;
        *) [ "$target" = "$host" ] || fail 'Use --cross for this target, or build on the matching OS/CPU.' ;;
    esac
fi

printf 'Host: %s\nTarget: %s\nAction: %s\nBuilder: %s\n' "$host" "$target" "$action" "$builder"
if [ "$action" != binary ]; then
    case "$target" in
        *-apple-darwin|universal)
            set -- --target "$target"
            case "$action" in
                full) set -- "$@" --dmg ;;
                package-only) set -- "$@" --no-build ;;
                dmg-only) set -- "$@" --dmg-only ;;
            esac
            [ "$dry_run" = false ] || set -- "$@" --dry-run
            exec sh build/package-macos.sh "$@"
            ;;
    esac
fi
[ "$action" != dmg-only ] || fail 'DMG creation is only available for macOS targets.'

targets=$target
if [ "$target" = universal ]; then
    targets='aarch64-apple-darwin x86_64-apple-darwin'
fi
if [ "$dry_run" = true ]; then
    [ "$use_cross" = false ] || printf 'Cross configuration: %s\n' "$CROSS_CONFIG"
    [ "$action" = binary ] || run cargo fetch --locked
    if [ "$action" != package-only ]; then
        for build_target in $targets; do
            run "$builder" build --locked --release --bin nabi-view --target "$build_target"
        done
    fi
    if [ "$action" != binary ]; then
        run python build/notices.py
        run python build/package-portable.py --target "$target"
        printf 'Output: <Cargo target directory>/package/%s/\n' "$target"
    fi
    exit 0
fi

need cargo
if [ "$action" != package-only ]; then
    need "$builder"
    if [ "$use_cross" = false ]; then
        need cmake
        need perl
        need rustup
        installed=$(rustup target list --installed | tr -d '\r')
        for build_target in $targets; do
            case "$build_target" in x86_64-*) need nasm ;; esac
            printf '%s\n' "$installed" | grep -Fx "$build_target" >/dev/null || fail "Install the Rust target first: rustup target add $build_target"
        done
        case "$target" in
            *-apple-darwin|universal) need xcrun; xcrun --sdk macosx --show-sdk-path >/dev/null ;;
        esac
    fi
fi

if [ "$action" != binary ]; then
    python_cmd=${PYTHON:-}
    if [ -z "$python_cmd" ]; then
        for candidate in python3 python; do
            if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import sys; sys.exit(sys.version_info < (3, 9))' >/dev/null 2>&1; then
                python_cmd=$candidate
                break
            fi
        done
    fi
    [ -n "$python_cmd" ] || fail 'Python 3.9+ is required for packaging. Set PYTHON to its executable path.'
    "$python_cmd" -c 'import sys; sys.exit(sys.version_info < (3, 9))' || fail 'Python 3.9+ is required.'
    # notices.py inventories all locked platforms, not just the selected target.
    cargo fetch --locked
fi

if [ "$action" != package-only ]; then
    for build_target in $targets; do
        "$builder" build --locked --release --bin nabi-view --target "$build_target"
    done
fi
if [ "$action" != binary ]; then
    "$python_cmd" build/notices.py
    "$python_cmd" build/package-portable.py --target "$target"
else
    for build_target in $targets; do
        printf 'Build complete. Binary: <Cargo target directory>/%s/release/nabi-view' "$build_target"
        case "$build_target" in *-windows-*) printf '.exe' ;; esac
        printf '\n'
    done
fi
