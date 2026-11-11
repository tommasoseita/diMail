#!/bin/sh
# Build di release per questo Mac: target/release/bundle/macos/diMail.app, firmata con
# scripts/signing-identity.sh. Le release pubbliche le fa scripts/release.sh (docs/releasing.md).
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
APPLE_SIGNING_IDENTITY="$(scripts/signing-identity.sh)"
export APPLE_SIGNING_IDENTITY
echo "Firma: $APPLE_SIGNING_IDENTITY"
npm run tauri build
