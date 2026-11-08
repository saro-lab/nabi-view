#!/bin/sh
# One entry point for native, cross and packaging builds. Never starts the app.
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec sh "$project_dir/build/menu.sh" "$@"
