#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
target=universal
signing=unsigned
no_build=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --target) [ "$#" -ge 2 ] || fail '--target requires a value'; target=$2; shift ;;
        --signed) signing=signed ;;
        --no-build) no_build=yes ;;
        --help|-h)
            cat <<'EOF'
Usage: sh build/macos-store.sh [--target universal|aarch64-apple-darwin|x86_64-apple-darwin] [--no-build] [--signed]
Default: ad-hoc signed sandbox candidate for testing, not a Store submission.
--signed requires NABI_APPLE_TEAM_ID, NABI_MACOS_BUNDLE_ID, NABI_BUILD_NUMBER,
NABI_MACOS_PROFILE, NABI_MACOS_THUMBNAIL_PROFILE, NABI_MACOS_PREVIEW_PROFILE (paths),
NABI_MACOS_APP_IDENTITY (Mac App Distribution or
Apple Distribution), and NABI_MACOS_INSTALLER_IDENTITY (Mac Installer Distribution).
Existing certificates and profiles only; no automatic account changes or upload.
Output: target/store/macos/<target>/NABI VIEW.app and (signed only) NABI-VIEW.pkg.
EOF
            exit 0 ;;
        *) fail "Unknown argument: $1" ;;
    esac
    shift
done
case "$target" in universal|aarch64-apple-darwin|x86_64-apple-darwin) ;; *) fail 'Unsupported target.' ;; esac
if [ "$signing" = signed ]; then
    : "${NABI_APPLE_TEAM_ID:?Set Apple team ID}"
    : "${NABI_MACOS_BUNDLE_ID:?Set registered Mac bundle ID}"
    : "${NABI_BUILD_NUMBER:?Set new Store build number}"
    : "${NABI_MACOS_PROFILE:?Set Mac App Store provisioning profile path}"
    : "${NABI_MACOS_THUMBNAIL_PROFILE:?Set Mac thumbnail extension provisioning profile path}"
    : "${NABI_MACOS_PREVIEW_PROFILE:?Set Mac preview extension provisioning profile path}"
    : "${NABI_MACOS_APP_IDENTITY:?Set Mac app distribution signing identity}"
    : "${NABI_MACOS_INSTALLER_IDENTITY:?Set Mac Installer Distribution identity}"
    [ -f "$NABI_MACOS_PROFILE" ] || fail 'Provisioning profile file not found.'
fi
build_number=${NABI_BUILD_NUMBER:-1}
case "$build_number" in ''|*[!0-9]*) fail 'NABI_BUILD_NUMBER must be a positive integer.' ;; esac
[ "$build_number" -gt 0 ] || fail 'NABI_BUILD_NUMBER must be positive.'
[ "$(uname -s)" = Darwin ] || fail 'macOS packaging requires a Mac.'
out="$project_dir/target/store/macos/$target"
python3 - "$project_dir" "$out" <<'PY'
import pathlib, sys
root, output = map(pathlib.Path, sys.argv[1:])
for path in [output, *output.parents]:
    if path == root:
        break
    if path.is_symlink():
        raise SystemExit(f"Refusing symlink output: {path}")
if output.exists() and (not output.is_dir() or not (output / ".nabi-store-output").is_file()):
    raise SystemExit("Existing Store output has no packaging marker; choose a clean output first.")
PY
if [ "$no_build" = yes ]; then set -- --no-build; else set --; fi
sh build/package-macos.sh --target "$target" "$@"
source_dir=$(sh build/package-macos.sh --target "$target" --print-output-dir)
mkdir -p "$project_dir/target/store/macos"
staging=$(mktemp -d "$project_dir/target/store/macos/.$target.stage.XXXXXXXX")
trap 'rm -rf "$staging"' 0
trap 'exit 130' INT
trap 'exit 143' TERM HUP
app="$staging/NABI VIEW.app"
ditto "$source_dir/NABI VIEW.app" "$app"
cp platforms/macos/store.entitlements "$staging/entitlements.plist"
python3 - "$app/Contents/Info.plist" <<'PY'
import os, pathlib, plistlib, re, sys
path = pathlib.Path(sys.argv[1])
info = plistlib.loads(path.read_bytes())
info["CFBundleIdentifier"] = os.environ.get("NABI_MACOS_BUNDLE_ID", "dev.nabi.NabiView")
info["CFBundleShortVersionString"] = re.search(r'^version = "([^"]+)"', pathlib.Path("Cargo.toml").read_text(), re.M)[1]
info["CFBundleVersion"] = os.environ.get("NABI_BUILD_NUMBER", "1")
info["LSApplicationCategoryType"] = "public.app-category.photography"
path.write_bytes(plistlib.dumps(info))
PY
if [ "$signing" = signed ]; then
    security cms -D -i "$NABI_MACOS_PROFILE" > "$staging/profile.plist"
    python3 - "$staging/profile.plist" "$staging/entitlements.plist" <<'PY'
import datetime, os, pathlib, plistlib, sys
profile = plistlib.loads(pathlib.Path(sys.argv[1]).read_bytes())
team = os.environ["NABI_APPLE_TEAM_ID"]
bundle = os.environ["NABI_MACOS_BUNDLE_ID"]
allowed = profile.get("Entitlements", {})
app_id = allowed.get("com.apple.application-identifier")
expected_ids = [f"{prefix}.{bundle}" for prefix in profile.get("ApplicationIdentifierPrefix", [])]
if team not in profile.get("TeamIdentifier", []) or app_id not in expected_ids:
    raise SystemExit("Profile must match the exact Mac bundle ID and team.")
if (profile.get("ProvisionedDevices") or profile.get("ProvisionsAllDevices")
        or allowed.get("get-task-allow") or allowed.get("com.apple.security.get-task-allow")):
    raise SystemExit("Use a Mac App Store distribution profile, not a development/Developer ID profile.")
if profile["ExpirationDate"] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
    raise SystemExit("Provisioning profile has expired.")
path = pathlib.Path(sys.argv[2])
entitlements = plistlib.loads(path.read_bytes())
entitlements["com.apple.application-identifier"] = app_id
entitlements["com.apple.developer.team-identifier"] = team
path.write_bytes(plistlib.dumps(entitlements))
PY
    cp "$NABI_MACOS_PROFILE" "$app/Contents/embedded.provisionprofile"
    identity=$NABI_MACOS_APP_IDENTITY
else
    identity=-
fi
if [ "$signing" = signed ]; then set -- --signed; else set --; fi
python3 build/sign-macos-previews.py "$app" --identity "$identity" "$@"
codesign --force --sign "$identity" --entitlements "$staging/entitlements.plist" "$app"
codesign --verify --strict "$app"
if [ "$signing" = signed ]; then
    productbuild --component "$app" /Applications --sign "$NABI_MACOS_INSTALLER_IDENTITY" "$staging/NABI-VIEW.pkg"
    pkgutil --check-signature "$staging/NABI-VIEW.pkg"
fi
rm -f "$staging/profile.plist" "$staging/entitlements.plist"
printf '%s\n' 'nabi-view-store-v1' > "$staging/.nabi-store-output"
# Publish one complete generation; unsigned output cannot retain an old PKG.
python3 - "$staging" "$out" <<'PY'
import os, pathlib, shutil, sys
staging, output = map(pathlib.Path, sys.argv[1:])
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
printf 'Store candidate: %s (%s)\n' "$out" "$signing"
printf '%s\n' 'Sandbox file access and persistent security-scoped bookmarks still require release qualification.'
