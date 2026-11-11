#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
platform=${1:---help}
[ "$#" -eq 0 ] || shift
case "$platform" in
    preview)
        cd "$project_dir"
        if ! command -v cmake >/dev/null 2>&1; then PATH="$project_dir/ai/runtime/cmake/cmake/data/bin:$PATH"; export PATH; fi
        exec cargo run --locked --release --bin nabi-view -- --mobile-preview "$@"
        ;;
    ios) exec sh "$project_dir/build/ios.sh" "$@" ;;
    iphone) exec sh "$project_dir/build/ios.sh" --phone "$@" ;;
    ipad) exec sh "$project_dir/build/ios.sh" --tablet "$@" ;;
    android) exec sh "$project_dir/build/android.sh" "$@" ;;
    -h|--help)
        printf '%s\n' 'NABI VIEW 모바일 실행' \
            '  ./mobile.sh preview              데스크톱에서 모바일 화면 배치 확인' \
            '  ./mobile.sh ios                  iPhone/iPad 시뮬레이터 빌드·설치·실행' \
            '  ./mobile.sh iphone               iPhone 시뮬레이터에서 실행' \
            '  ./mobile.sh ipad                 iPad 시뮬레이터에서 실행 (ios --tablet)' \
            '  ./mobile.sh ios --list           사용 가능한 시뮬레이터 목록' \
            '  ./mobile.sh ios --simulator UUID 특정 시뮬레이터에서 실행' \
            '  ./mobile.sh ios --build-only     iPhone/iPad 공용 시뮬레이터 .app 생성' \
            '  ./mobile.sh ios --device         미서명 iPhone/iPad ARM64 .app 생성' \
            '  ./mobile.sh android             연결된 휴대폰/태블릿/에뮬레이터에서 실행' \
            '  ./mobile.sh android --build-only 휴대폰/태블릿 공용 APK 생성' \
            '  ./mobile.sh android --release --build-only 배포용 APK 생성' \
            '  ./mobile.sh android --release --bundle Play 제출용 AAB 생성' \
            '  ./mobile.sh android --release --bundle --abi all 두 64비트 ABI 포함' \
            '  ./mobile.sh android --bundle --signed 외부 키로 서명한 AAB 생성' \
            '  ./mobile.sh android --abi x86_64 Intel 에뮬레이터용 APK 생성' \
            '  ./mobile.sh android --serial ID  특정 adb 기기에서 실행'
        ;;
    *) printf 'Unknown platform: %s\n' "$platform" >&2; exit 2 ;;
esac
