#!/bin/bash
# Publishes a diMail version.
#
#   scripts/release.sh "One-line release notes"
#
# 1. tests, then a universal build (Apple silicon and Intel) signed with Developer ID
# 2. app notarization and ticket stapling
# 3. the update archive (diMail.app.tar.gz) signed with the updater key
# 4. DMG signed, notarized and stapled
# 5. latest.json for the updater, and a GitHub release v<version> of $GITHUB_REPO with
#    diMail.dmg, diMail.app.tar.gz and latest.json: created as a draft, published only once
#    the three files are uploaded. The notes are the version's section of CHANGELOG.md, if any.
#
#   scripts/release.sh --local      # only steps 1-4: notarized DMG in dist-release/<version>-local,
#                                   # nothing published (for tests)
#
# Prerequisites (once): the team's "Developer ID Application" identity in the Keychain, the
# notarytool profile and the updater key named in scripts/project.env, and `gh auth login`.
# See docs/releasing.md. The version comes from scripts/bump-version.sh: bump, commit and push
# before every release.
set -euo pipefail

. "$(dirname "$0")/common.sh"
load_project_env
ROOT="$DIMAIL_ROOT"
LOCAL=0
if [ "${1:-}" = "--local" ]; then LOCAL=1; shift; fi
NOTES="${1:-Improvements and fixes.}"
PROFILE="$NOTARY_PROFILE"
VERSION=$(dimail_version)
TAG="v$VERSION"

fail() { echo "Error: $*" >&2; exit 1; }

[ -f "$UPDATER_KEY" ] || fail "missing updater key $UPDATER_KEY (npx tauri signer generate -w $UPDATER_KEY)"
if [ "$LOCAL" = 0 ]; then
  [ -z "$(git -C "$ROOT" status --porcelain)" ] || fail "there are uncommitted changes: a release starts from a specific commit"
  command -v gh >/dev/null || fail "the GitHub CLI (gh) is missing"
  gh auth status >/dev/null 2>&1 || fail "run gh auth login first"
  ORIGIN=$(git -C "$ROOT" remote get-url origin 2>/dev/null || true)
  case "$ORIGIN" in
    *"github.com/$GITHUB_REPO"|*"github.com/$GITHUB_REPO.git"|*"github.com:$GITHUB_REPO.git") ;;
    *) fail "run releases from a clone of $GITHUB_REPO (origin is '$ORIGIN')" ;;
  esac
  git -C "$ROOT" fetch --quiet --tags origin
  SHA=$(git -C "$ROOT" rev-parse HEAD)
  [ -n "$(git -C "$ROOT" branch -r --contains "$SHA")" ] || fail "push $SHA to origin first: the release tags that commit"
  git -C "$ROOT" rev-parse "$TAG" >/dev/null 2>&1 && fail "$TAG already exists: run scripts/bump-version.sh"
  gh release view "$TAG" --repo "$GITHUB_REPO" >/dev/null 2>&1 && fail "release $TAG already exists on GitHub"
fi
IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | grep "($TEAM_ID)" | head -1 | sed -E 's/.*"(.*)"$/\1/')
[ -n "$IDENTITY" ] || fail "missing Developer ID Application identity for team $TEAM_ID"
xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1 || fail "the notarytool profile \"$PROFILE\" does not work"

OUT="$ROOT/dist-release/$VERSION"
[ "$LOCAL" = 1 ] && OUT="$ROOT/dist-release/$VERSION-local"
rm -rf "$OUT"
mkdir -p "$OUT"
APP="$ROOT/target/universal-apple-darwin/release/bundle/macos/diMail.app"
ARCHIVE="$OUT/diMail.app.tar.gz"
DMG="$OUT/diMail.dmg"

notarize() {
  local file="$1"
  local result id status
  result=$(xcrun notarytool submit "$file" --keychain-profile "$PROFILE" --wait --timeout 30m --output-format json)
  id=$(echo "$result" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
  status=$(echo "$result" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
  echo "Notarization of $(basename "$file"): $status ($id)"
  if [ "$status" != "Accepted" ]; then
    xcrun notarytool log "$id" --keychain-profile "$PROFILE" || true
    fail "notarization rejected"
  fi
}

echo "== 1. Tests and build $VERSION signed with: $IDENTITY"
cd "$ROOT"
npm run check
cargo test --workspace --quiet
# The update archive is made below, after stapling: Tauri must not build its own.
env -u TAURI_SIGNING_PRIVATE_KEY APPLE_SIGNING_IDENTITY="$IDENTITY" \
  npm run tauri build -- --target universal-apple-darwin --bundles app
BUILT=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
[ "$BUILT" = "$VERSION" ] || fail "the app says version $BUILT, expected $VERSION"
ARCHS=$(lipo -archs "$APP/Contents/MacOS/dimail")
case "$ARCHS" in *x86_64*arm64*|*arm64*x86_64*) ;; *) fail "not a universal build: $ARCHS" ;; esac

echo "== 2. App notarization"
ditto -c -k --keepParent "$APP" "$OUT/diMail-notarize.zip"
notarize "$OUT/diMail-notarize.zip"
rm "$OUT/diMail-notarize.zip"
xcrun stapler staple "$APP"

echo "== 3. Update archive"
COPYFILE_DISABLE=1 tar -czf "$ARCHIVE" -C "$(dirname "$APP")" diMail.app
npx tauri signer sign -f "$UPDATER_KEY" -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" "$ARCHIVE" >/dev/null
[ -s "$ARCHIVE.sig" ] || fail "the update archive was not signed"

echo "== 4. DMG"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/diMail.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "diMail" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG" >/dev/null
codesign --force --sign "$IDENTITY" --timestamp "$DMG"
notarize "$DMG"
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature -vv "$DMG"
spctl --assess --type execute -vv "$APP"

if [ "$LOCAL" = 1 ]; then
  echo
  echo "diMail $VERSION notarized, not published: $DMG"
  exit 0
fi

echo "== 5. latest.json and GitHub release"
# Release notes: the version's section of CHANGELOG.md, else the one-line notes.
/usr/bin/python3 -I - "$ROOT/CHANGELOG.md" "$VERSION" "$NOTES" "https://github.com/$GITHUB_REPO/blob/$TAG/" > "$OUT/notes.md" <<'PY'
import re, sys
path, version, fallback, base = sys.argv[1:5]
try:
    text = open(path, encoding="utf-8").read()
except OSError:
    text = ""
match = re.search(rf"^## \[?{re.escape(version)}\]?[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
notes = match.group(1).strip() if match and match.group(1).strip() else fallback
# Relative links point into the repository at this release's tag.
print(re.sub(r"\]\((?!https?:|mailto:|#)([^)]+)\)", lambda m: f"]({base}{m.group(1)})", notes))
PY
# The universal archive serves both architectures.
/usr/bin/python3 -I - "$VERSION" "$OUT/notes.md" "$ARCHIVE.sig" \
  "https://github.com/$GITHUB_REPO/releases/download/$TAG/diMail.app.tar.gz" > "$OUT/latest.json" <<'PY'
import datetime, json, sys
version, notes, sig, url = sys.argv[1:5]
entry = {"signature": open(sig, encoding="utf-8").read().strip(), "url": url}
print(json.dumps({
    "version": version,
    "notes": open(notes, encoding="utf-8").read().strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"darwin-aarch64": entry, "darwin-x86_64": entry},
}, indent=2, ensure_ascii=False))
PY
gh release create "$TAG" --repo "$GITHUB_REPO" --target "$SHA" --draft \
  --title "diMail $VERSION" --notes-file "$OUT/notes.md" "$DMG" "$ARCHIVE" "$OUT/latest.json"
# Publish only if the three files arrived whole.
DMG_SIZE=$(stat -f%z "$DMG")
ARCHIVE_SIZE=$(stat -f%z "$ARCHIVE")
UPLOADED=$(gh release view "$TAG" --repo "$GITHUB_REPO" --json assets \
  --jq '[.assets[] | select((.name == "diMail.dmg" and .size == '"$DMG_SIZE"') or (.name == "diMail.app.tar.gz" and .size == '"$ARCHIVE_SIZE"') or .name == "latest.json")] | length')
[ "$UPLOADED" = 3 ] || fail "the draft release $TAG is missing files: check it on GitHub, then delete it and retry"
gh release edit "$TAG" --repo "$GITHUB_REPO" --draft=false --latest >/dev/null
git -C "$ROOT" fetch --quiet --tags origin
echo
echo "diMail $VERSION published: https://github.com/$GITHUB_REPO/releases/tag/$TAG"
echo "DMG for new installs: https://github.com/$GITHUB_REPO/releases/latest/download/diMail.dmg"
