#!/bin/bash
# Checks a tree before it is published as the public repository.
#
#   scripts/check-public.sh <dir>
#
# Fails when LICENSE is missing, a file looks like a secret, a private document is still there,
# or any text file mentions private names, accounts, ids or keys. The only allowed exception is
# the literal "app.wibo.dimail" (the identifier of the versions before 9 November 2026, which
# crates/core/src/migrate.rs still migrates from). Every hit is printed as file:line.
set -euo pipefail

DIR="${1:?Usage: scripts/check-public.sh <dir>}"
[ -d "$DIR" ] || { echo "Not a directory: $DIR" >&2; exit 2; }

exec /usr/bin/python3 -I - "$DIR" <<'PY'
import os
import re
import sys

root = os.path.abspath(sys.argv[1])
problems = []

ALLOWED = "app.wibo.dimail"
PATTERNS = [
    ("private name", re.compile(r"wibo", re.I)),
    ("Google OAuth client id", re.compile(r"\d{6,}-[a-z0-9]{20,}\.apps\.googleusercontent\.com")),
    ("Google OAuth client secret", re.compile(r"GOCSPX-[A-Za-z0-9_-]{10,}")),
    ("private key", re.compile(r"PRIVATE KEY")),
    ("updater private key", re.compile(r"untrusted comment: (rsign|minisign) encrypted secret key")),
    ("organization token", re.compile(r"\bd[vm]_[A-Za-z0-9_-]{20,}")),
    ("API key", re.compile(r"\bsk-[A-Za-z0-9_-]{20,}")),
    ("GitHub token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,}")),
]
SECRET_LABELS = {"Google OAuth client secret", "private key", "updater private key", "organization token", "API key", "GitHub token"}
SECRET_SUFFIXES = (".p8", ".p12", ".key", ".pem", ".cer", ".certsigningrequest", ".db", ".sqlite", ".sqlite3")
SECRET_NAMES = re.compile(r"^(\.dev\.vars|\.env)(\..+)?$")
# Written for the author, in Italian, with private context: they stay out of the public tree.
PRIVATE_DOCS = {
    os.path.join("docs", name)
    for name in ("DECISIONS.md", "SPEC-UX.md", "ORGANIZZAZIONI.md", "RELEASE.md")
}
SKIP_DIRS = {".git", "node_modules", "target", "dist", "dist-release"}

if not os.path.exists(os.path.join(root, "LICENSE")):
    problems.append("LICENSE: missing")

for dirpath, dirnames, filenames in os.walk(root):
    dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
    rel_dir = os.path.relpath(dirpath, root)
    if rel_dir.split(os.sep)[0] == "secrets":
        problems.append(f"{rel_dir}: secrets directory")
        dirnames[:] = []
        continue
    for name in filenames:
        path = os.path.join(dirpath, name)
        rel = os.path.relpath(path, root)
        lower = name.lower()
        if lower.endswith(SECRET_SUFFIXES) or (SECRET_NAMES.match(name) and not lower.endswith(".example")):
            problems.append(f"{rel}: looks like a secret file")
            continue
        if rel in PRIVATE_DOCS:
            problems.append(f"{rel}: private document")
            continue
        if rel == os.path.join("scripts", "check-public.sh"):
            continue  # this script names what it looks for
        try:
            data = open(path, "rb").read()
        except OSError as err:
            problems.append(f"{rel}: unreadable ({err})")
            continue
        if b"\0" in data[:8192]:
            continue  # binary file
        text = data.decode("utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            stripped = line.replace(ALLOWED, "")
            for label, pattern in PATTERNS:
                if pattern.search(stripped):
                    # Never echo a line that may contain a secret.
                    shown = "" if label in SECRET_LABELS else f": {line.strip()[:160]}"
                    problems.append(f"{rel}:{number}: {label}{shown}")

if problems:
    print("Not ready to publish:")
    for problem in problems:
        print(f"  {problem}")
    sys.exit(1)
print(f"{root}: clean")
PY
