#!/bin/sh
# Runner di cargo su macOS. Prima di avviare il binario di diMail lo firma con l'identità di
# scripts/signing-identity.sh e con l'identificativo del bundle: così il Portachiavi riconosce anche
# le build di sviluppo come la stessa app di diMail.app e non chiede di nuovo l'accesso.
# Gli altri binari (test) partono come sono.
set -e
bin="$1"
if [ "$(basename "$bin")" = "dimail" ]; then
  codesign --force --sign "$("$(dirname "$0")/signing-identity.sh")" --identifier io.github.tommasoseita.dimail "$bin"
fi
exec "$@"
