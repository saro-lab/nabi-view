#!/bin/sh
# Interactive selection and command-line parsing share one build implementation.
set -eu

usage() {
    cat <<'EOF'
Usage: ./build.sh [TARGET] [ACTION] [OPTIONS]
       build.bat [TARGET] [ACTION] [OPTIONS]

No arguments: choose a target and build steps; building starts immediately.
With arguments: build directly (default: native --full). Never starts the app.

Actions (choose one):
  --full          Build + package; also create a DMG on macOS (default).
  --binary        Compile release executable(s) only.
  --package       Build + package (.app on macOS; ZIP/tar.gz elsewhere).
  --package-only  Package existing release executable(s), without compiling.
  --dmg-only      Create a DMG from an existing macOS .app, without compiling.

Targets (or --target TARGET; Rust triples are also accepted):
  native, macos-arm64, macos-x64, macos-universal,
  windows-x64, windows-arm64, windows-gnu, linux-x64, linux-arm64

Options:
  --cross         Use cross-rs on x64 Linux/WSL2 for Linux/Windows GNU.
  --dry-run       Show the plan without building or writing files.
  --interactive   Show the menu; supplied target/action skip their questions.
  -y, --yes       Accepted for compatibility; no confirmation is required.
  -h, --help      Show this help.

Outputs: <Cargo target directory>/<target>/release/ and package/<target>/.
Finished macOS DMGs: <Cargo target directory>/dist/NABI-VIEW-macos-{arm64,x64,universal}.dmg.
Native builds require Rust, C/C++, CMake, Perl; x64 also requires NASM.
Packaging requires Python 3.9+ (override with PYTHON=/path/to/python).
Mac builds require macOS + Xcode tools; MSVC builds require Windows tools.
DMG creation does not mount a disk image or automate Finder.
EOF
}

fail() { printf 'Error: %s\n' "$*" >&2; exit 2; }
cancel() { printf '\n취소했습니다.\n'; exit 0; }
set_action() {
    [ -z "$action" ] || fail 'Choose only one build action.'
    action=$1
}
set_target() {
    [ -z "$requested" ] || fail 'Choose only one target.'
    requested=$1
}

interactive=false
[ "$#" -ne 0 ] || interactive=true
requested=
action=
use_cross=false
dry_run=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --full|--dmg) set_action full ;;
        --binary) set_action binary ;;
        --package) set_action package ;;
        --package-only) set_action package-only ;;
        --dmg-only) set_action dmg-only ;;
        --cross) use_cross=true ;;
        --dry-run) dry_run=true ;;
        --interactive) interactive=true ;;
        -y|--yes) : ;; # Compatibility with older build invocations.
        --target)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail '--target requires a value.'
            case "$2" in -*) fail '--target requires a value.' ;; esac
            set_target "$2"
            shift
            ;;
        -*) fail "Unknown option: $1. Use --help." ;;
        *) set_target "$1" ;;
    esac
    shift
done

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ "$interactive" = true ]; then
    command -v rustc >/dev/null 2>&1 || fail 'Rust is required. Install it and reopen the terminal.'
    host=$(rustc -vV | sed -n 's/^host: //p' | tr -d '\r')
    [ -n "$host" ] || fail 'Cannot determine the Rust host target.'
    printf '\nNABI VIEW 빌드\n현재 환경: %s\n' "$host"
    if [ -z "$requested" ]; then
        printf '\n빌드 대상\n  1) 현재 OS / CPU (기본)\n'
        case "$host" in
            *-apple-darwin)
                printf '  2) macOS Apple Silicon\n  3) macOS Intel\n  4) macOS Universal (Apple Silicon + Intel)\n'
                ;;
            *-pc-windows-*)
                printf '  2) Windows x64 MSVC\n  3) Windows ARM64 MSVC\n'
                ;;
            x86_64-unknown-linux-gnu)
                printf '  2) Linux ARM64 (cross-rs + 컨테이너 필요)\n  3) Windows x64 GNU (cross-rs + 컨테이너 필요)\n'
                ;;
        esac
        printf '  0) 취소\n'
        while [ -z "$requested" ]; do
            printf '선택 [1]: '
            IFS= read -r choice || cancel
            case "$choice" in
                0|q|Q) cancel ;;
                ''|1) requested=native ;;
                *)
                    case "$host:$choice" in
                        *-apple-darwin:2) requested=macos-arm64 ;;
                        *-apple-darwin:3) requested=macos-x64 ;;
                        *-apple-darwin:4) requested=macos-universal ;;
                        *-pc-windows-*:2) requested=windows-x64 ;;
                        *-pc-windows-*:3) requested=windows-arm64 ;;
                        x86_64-unknown-linux-gnu:2) requested=linux-arm64; use_cross=true ;;
                        x86_64-unknown-linux-gnu:3) requested=windows-gnu; use_cross=true ;;
                        *) printf '표시된 번호를 선택해 주세요.\n' ;;
                    esac
                    ;;
            esac
        done
    fi
    selected_target=$requested
    [ "$selected_target" != native ] || selected_target=$host
    is_mac=false
    case "$selected_target" in macos-*|*-apple-darwin) is_mac=true ;; esac
    if [ -z "$action" ]; then
        printf '\n빌드 범위 (선택한 대상: %s)\n' "$selected_target"
        if [ "$is_mac" = true ]; then
            printf '  1) 전체 빌드: 실행 파일 + .app + DMG (기본)\n'
            printf '  2) 실행 파일만 빌드\n  3) 실행 파일 + .app 빌드\n'
            printf '  4) 기존 실행 파일로 .app만 다시 만들기\n  5) 기존 .app으로 DMG만 만들기\n'
        else
            printf '  1) 전체 빌드: 실행 파일 + 배포 압축 파일 (기본)\n'
            printf '  2) 실행 파일만 빌드\n  3) 기존 실행 파일로 배포 압축 파일만 다시 만들기\n'
        fi
        printf '  0) 취소\n'
        while [ -z "$action" ]; do
            printf '선택 [1]: '
            IFS= read -r choice || cancel
            case "$choice" in
                0|q|Q) cancel ;;
                ''|1) action=full ;;
                2) action=binary ;;
                *)
                    case "$is_mac:$choice" in
                        true:3) action=package ;;
                        true:4|false:3) action=package-only ;;
                        true:5) action=dmg-only ;;
                        *) printf '표시된 번호를 선택해 주세요.\n' ;;
                    esac
                    ;;
            esac
        done
    fi
    case "$action" in
        full) action_label='전체 빌드' ;;
        binary) action_label='실행 파일 빌드' ;;
        package) action_label='실행 파일 + 패키지 빌드' ;;
        package-only) action_label='기존 실행 파일로 패키지 생성' ;;
        dmg-only) action_label='기존 .app으로 DMG 생성' ;;
    esac
    printf '\n대상: %s\n작업: %s\n' "$selected_target" "$action_label"
    [ "$use_cross" = false ] || printf '방식: cross-rs\n'
    [ "$dry_run" = false ] || printf '계획만 표시합니다 (--dry-run).\n'
fi

exec sh "$project_dir/build/desktop.sh" "${requested:-native}" "$use_cross" "${action:-full}" "$dry_run"
