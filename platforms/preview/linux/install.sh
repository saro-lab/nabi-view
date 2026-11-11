#!/bin/sh
# Explicit native installation. Modern GNOME's sandbox needs a system prefix.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${NABI_INSTALL_PREFIX:-/usr/local}
remove=false
case "${1:-}" in --uninstall) remove=true ;; '') ;; *) printf 'Usage: NABI_INSTALL_PREFIX=/usr/local %s [--uninstall]\n' "$0" >&2; exit 2 ;; esac
[ "$#" -le 1 ] || exit 2
case "$prefix" in /*) ;; *) echo 'Installation prefix must be absolute.' >&2; exit 2 ;; esac
bin="$prefix/bin/nabi-thumbnailer"
entry="$prefix/share/thumbnailers/nabi-view.thumbnailer"
marker="$prefix/share/thumbnailers/.nabi-view-owner"
if $remove; then
    [ -f "$marker" ] && [ "$(cat "$marker")" = "$root" ] || { echo 'This package does not own the installation.' >&2; exit 1; }
    rm -f "$bin" "$entry" "$marker"
    exit 0
fi
[ -x "$root/nabi-thumbnailer" ] || { echo 'Missing nabi-thumbnailer executable.' >&2; exit 1; }
if [ -e "$bin" ] || [ -e "$entry" ]; then
    [ -f "$marker" ] && [ "$(cat "$marker")" = "$root" ] || { echo 'An unrelated thumbnailer installation exists.' >&2; exit 1; }
fi
mkdir -p "$prefix/bin" "$prefix/share/thumbnailers"
install -m 755 "$root/nabi-thumbnailer" "$bin"
# The command stays a simple executable name; /usr/local/bin must be on the
# desktop PATH. A host sandbox may exclude arbitrary custom/home prefixes.
install -m 644 "$root/nabi-view.thumbnailer" "$entry"
printf '%s\n' "$root" > "$marker"
printf '%s\n' 'Installed native thumbnailer. Restart the file manager to reload providers.'
