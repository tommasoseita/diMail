#!/bin/bash
# Sets the version everywhere it lives.
#
#   scripts/bump-version.sh 0.2.0
#
# Updates src-tauri/tauri.conf.json, Cargo.toml (workspace) and package.json, then refreshes
# Cargo.lock and package-lock.json. Commit the result, push it, and run scripts/release.sh.
set -euo pipefail

. "$(dirname "$0")/common.sh"
VERSION="${1:?Usage: scripts/bump-version.sh MAJOR.MINOR.PATCH}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid version '$VERSION': expected MAJOR.MINOR.PATCH" >&2; exit 1; }

/usr/bin/python3 -I - "$DIMAIL_ROOT" "$VERSION" <<'PY'
import re, sys
root, version = sys.argv[1:3]

def edit(path, pattern, count=1):
    text = open(path, encoding="utf-8").read()
    new, n = re.subn(pattern, lambda m: m.group(1) + version + m.group(2), text, count=count, flags=re.M | re.S)
    if n != count:
        sys.exit(f"{path}: version not found")
    open(path, "w", encoding="utf-8").write(new)

# The first "version" key of each JSON file is the project's own.
edit(f"{root}/src-tauri/tauri.conf.json", r'("version":\s*")[^"]+(")')
edit(f"{root}/package.json", r'("version":\s*")[^"]+(")')
edit(f"{root}/Cargo.toml", r'(^\[workspace\.package\].*?^version\s*=\s*")[^"]+(")')
PY

cd "$DIMAIL_ROOT"
cargo update --workspace --offline --quiet
npm install --package-lock-only --silent
echo "diMail $(dimail_version): commit, push, then scripts/release.sh"
