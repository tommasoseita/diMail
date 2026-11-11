#!/bin/sh
# Stampa l'identità con cui firmare diMail in locale: DIMAIL_SIGNING_IDENTITY se c'è, altrimenti il
# primo «Developer ID Application» del Portachiavi, altrimenti "-" (firma ad hoc). Con una firma
# stabile il Portachiavi riconosce ogni build come la stessa app e non richiede l'accesso.
if [ -n "${DIMAIL_SIGNING_IDENTITY:-}" ]; then
  echo "$DIMAIL_SIGNING_IDENTITY"
  exit 0
fi
id=$(security find-identity -v -p codesigning 2>/dev/null | grep "Developer ID Application" | head -1 | sed -E 's/.*"(.*)"$/\1/')
echo "${id:--}"
