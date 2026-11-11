#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'EOF'
Usage: sh build/ios-store.sh archive [--unsigned | --signed]
       sh build/ios-store.sh export

archive defaults to --unsigned (compile/archive validation; not installable).
--signed and export require NABI_APPLE_TEAM_ID, NABI_IOS_BUNDLE_ID,
NABI_IOS_PROFILE (installed profile name/UUID), and NABI_BUILD_NUMBER.
NABI_APPLE_SIGN_IDENTITY defaults to Apple Distribution.
Profiles and certificates must already exist; this script never creates them.
Output: target/store/ios/NabiView.xcarchive and target/store/ios/export/.
Export creates a local IPA; nothing is uploaded or submitted.
EOF
}
action=${1:---help}
[ "$#" -eq 0 ] || shift
signing=unsigned
case "$action" in archive|export) ;; --help|-h) usage; exit 0 ;; *) usage >&2; exit 2 ;; esac
if [ "$action" = archive ]; then
    case "${1:---unsigned}" in --unsigned) ;; --signed) signing=signed ;; *) usage >&2; exit 2 ;; esac
    [ "$#" -eq 0 ] || shift
else
    signing=signed
fi
[ "$#" -eq 0 ] || fail 'Unexpected arguments.'
if [ "$signing" = signed ]; then
    : "${NABI_APPLE_TEAM_ID:?Set the Apple team ID}"
    : "${NABI_IOS_BUNDLE_ID:?Set the registered iOS bundle ID}"
    : "${NABI_IOS_PROFILE:?Set the installed App Store profile name or UUID}"
    : "${NABI_BUILD_NUMBER:?Set a new App Store build number}"
fi
build_number=${NABI_BUILD_NUMBER:-1}
case "$build_number" in ''|*[!0-9]*) fail 'NABI_BUILD_NUMBER must be a positive integer.' ;; esac
[ "$build_number" -gt 0 ] || fail 'NABI_BUILD_NUMBER must be positive.'
[ "$(uname -s)" = Darwin ] || fail 'iOS archives require macOS and Xcode.'
if [ -z "${DEVELOPER_DIR:-}" ]; then
    DEVELOPER_DIR=$(xcode-select -p)
    if [ ! -d "$DEVELOPER_DIR/Platforms/iPhoneOS.platform" ] && [ -d /Applications/Xcode.app ]; then
        DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    fi
fi
export DEVELOPER_DIR
[ -d "$DEVELOPER_DIR/Platforms/iPhoneOS.platform" ] || fail 'Install Xcode with iOS support.'
out="$project_dir/target/store/ios"
python3 - "$project_dir" "$out" "$project_dir/target/ios/derived" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
for value in [sys.argv[2], sys.argv[3], f"{sys.argv[2]}/NabiView.xcarchive", f"{sys.argv[2]}/export"]:
    path = pathlib.Path(value)
    for item in [path, *path.parents]:
        if item == root:
            break
        if item.is_symlink():
            raise SystemExit(f"Refusing symlink output: {item}")
PY
mkdir -p "$out"
staging=$(mktemp -d "$out/.stage.XXXXXXXX")
trap 'rm -rf "$staging"' 0
trap 'exit 130' INT
trap 'exit 143' TERM HUP
publish() {
    python3 - "$1" "$2" <<'PY'
import os, pathlib, plistlib, shutil, sys
staging, output = map(pathlib.Path, sys.argv[1:])
if output.exists():
    if output.suffix == ".xcarchive":
        info = plistlib.loads((output / "Info.plist").read_bytes())
        if info.get("SchemeName") != "NabiView":
            raise SystemExit("Refusing to replace an unrelated archive.")
    elif not (output / ".nabi-store-output").is_file():
        raise SystemExit("Existing export has no packaging marker.")
backup = output.with_name(f".{output.name}.previous-{os.getpid()}")
if backup.exists() or backup.is_symlink():
    raise SystemExit("Unexpected previous-generation path.")
had_output = output.exists()
if had_output:
    output.rename(backup)
try:
    staging.rename(output)
except BaseException:
    if had_output:
        backup.rename(output)
    raise
if had_output:
    shutil.rmtree(backup)
PY
}
version=$(python3 -c 'import pathlib,re; print(re.search(r"^version = \"([^\"]+)\"", pathlib.Path("Cargo.toml").read_text(), re.M)[1])')
if [ "$action" = archive ]; then
    if [ "$signing" = signed ]; then
        set -- "DEVELOPMENT_TEAM=$NABI_APPLE_TEAM_ID" "CODE_SIGN_STYLE=Manual" \
            "CODE_SIGN_IDENTITY=${NABI_APPLE_SIGN_IDENTITY:-Apple Distribution}" \
            "PROVISIONING_PROFILE_SPECIFIER=$NABI_IOS_PROFILE"
    else
        set -- CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO
    fi
    xcodebuild -project platforms/ios/NabiView.xcodeproj -scheme NabiView \
        -configuration Release -destination 'generic/platform=iOS' \
        -derivedDataPath "$project_dir/target/ios/derived" -archivePath "$staging/NabiView.xcarchive" \
        "PRODUCT_BUNDLE_IDENTIFIER=${NABI_IOS_BUNDLE_ID:-dev.nabi.view}" \
        "MARKETING_VERSION=$version" "CURRENT_PROJECT_VERSION=$build_number" "$@" archive
    publish "$staging/NabiView.xcarchive" "$out/NabiView.xcarchive"
    printf 'Archive: %s (%s)\n' "$out/NabiView.xcarchive" "$signing"
else
    [ -d "$out/NabiView.xcarchive" ] || fail 'Create a signed archive first.'
    python3 - "$out/NabiView.xcarchive" "$version" "$build_number" <<'PY'
import os, pathlib, plistlib, sys
archive = pathlib.Path(sys.argv[1])
properties = plistlib.loads((archive / "Info.plist").read_bytes())["ApplicationProperties"]
expected = {"CFBundleIdentifier": os.environ["NABI_IOS_BUNDLE_ID"],
            "CFBundleShortVersionString": sys.argv[2], "CFBundleVersion": sys.argv[3],
            "Team": os.environ["NABI_APPLE_TEAM_ID"]}
for key, value in expected.items():
    if properties.get(key) != value:
        raise SystemExit(f"Archive {key} does not match this export configuration; create a matching signed archive.")
PY
    codesign --verify --strict "$out/NabiView.xcarchive/Products/Applications/NABI VIEW.app"
    python3 - "$staging/ExportOptions.plist" <<'PY'
import os, plistlib, sys
options = {
    "method": "app-store-connect", "destination": "export", "signingStyle": "manual",
    "teamID": os.environ["NABI_APPLE_TEAM_ID"],
    "signingCertificate": os.environ.get("NABI_APPLE_SIGN_IDENTITY", "Apple Distribution"),
    "provisioningProfiles": {os.environ["NABI_IOS_BUNDLE_ID"]: os.environ["NABI_IOS_PROFILE"]},
    "manageAppVersionAndBuildNumber": False,
}
with open(sys.argv[1], "wb") as out:
    plistlib.dump(options, out)
PY
    xcodebuild -exportArchive -archivePath "$out/NabiView.xcarchive" \
        -exportPath "$staging/export" -exportOptionsPlist "$staging/ExportOptions.plist"
    printf '%s\n' 'nabi-view-store-v1' > "$staging/export/.nabi-store-output"
    publish "$staging/export" "$out/export"
fi
