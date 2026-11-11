#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
fail() { printf '%s\n' "$*" >&2; exit 1; }
case "${1:-}" in
    --help|-h)
        printf '%s\n' 'Usage: sh build/linux-flatpak.sh [--offline]' \
            'Builds a Flatpak for the current Linux CPU; does not install or publish it.' \
            'Install Freedesktop Platform and SDK branch 25.08 first.' \
            'The manifest downloads pinned Rust build tools; --offline only controls Cargo sources.' \
            'Optional: NABI_GPG_KEY_ID and NABI_GPG_HOME sign the exported repository commit.'
        exit 0 ;;
    ''|--offline) ;;
    *) fail "Unknown option: $1" ;;
esac
[ "$#" -le 1 ] || fail 'Expected at most --offline.'
[ "$(uname -s)" = Linux ] || fail 'Flatpak packaging requires a Linux host or Linux VM.'
for tool in flatpak flatpak-builder cargo python3; do
    command -v "$tool" >/dev/null || fail "Required tool: $tool"
done
arch=$(flatpak --default-arch)
case "$arch" in x86_64|aarch64) ;; *) fail 'Supported Flatpak architectures: x86_64, aarch64' ;; esac
for ref in org.freedesktop.Platform org.freedesktop.Sdk; do
    flatpak info --user "$ref/$arch/25.08" >/dev/null 2>&1 ||
        fail "Install this user runtime first: flatpak install --user flathub $ref/$arch/25.08"
done
manifest=$(python3 build/linux-flatpak-prepare.py "$@")
stage=$(dirname "$manifest")
repo="$stage/repo"
flatpak-builder --user --disable-rofiles-fuse --state-dir="$stage/state" \
    --repo="$repo" --arch="$arch" "$stage/app" "$manifest"

# Signing is optional for local test bundles. Never create keys or export a
# private key. An operator can opt into their existing GPG agent/keyring.
if [ -n "${NABI_GPG_KEY_ID:-}" ]; then
    if [ -n "${NABI_GPG_HOME:-}" ]; then
        flatpak build-sign --arch="$arch" --gpg-sign="$NABI_GPG_KEY_ID" \
            --gpg-homedir="$NABI_GPG_HOME" "$repo" dev.nabi.NabiView stable
    else
        flatpak build-sign --arch="$arch" --gpg-sign="$NABI_GPG_KEY_ID" \
            "$repo" dev.nabi.NabiView stable
    fi
fi
version=$(cargo metadata --locked --offline --no-deps --format-version=1 | python3 -c \
    'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "nabi-view"))')
output="$project_dir/target/package/flatpak/$arch"
mkdir -p "$output"
bundle="nabi-view-$version-linux-$arch.flatpak"
flatpak build-bundle --arch="$arch" \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$repo" "$output/$bundle" dev.nabi.NabiView stable
(cd "$output" && sha256sum "$bundle" > "$bundle.sha256")
printf 'Bundle: %s\n' "$output/$bundle"
