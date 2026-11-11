# Shared helpers for the release scripts. Source it, don't run it.
#
#   . "$(dirname "$0")/common.sh"

DIMAIL_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"

# Settings from scripts/project.env; variables already set in the environment win.
load_project_env() {
  local line key value
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    key=${line%%=*}
    value=${line#*=}
    value=${value/#\~/$HOME}
    if [ -z "${!key+x}" ]; then export "$key=$value"; fi
  done < "$DIMAIL_ROOT/scripts/project.env"
}

# The version, from src-tauri/tauri.conf.json. Fails when Cargo.toml or package.json disagree:
# scripts/bump-version.sh keeps the three in step.
dimail_version() {
  /usr/bin/python3 -I - "$DIMAIL_ROOT" <<'PY'
import json, re, sys
root = sys.argv[1]
tauri = json.load(open(f"{root}/src-tauri/tauri.conf.json", encoding="utf-8"))["version"]
npm = json.load(open(f"{root}/package.json", encoding="utf-8"))["version"]
cargo = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', open(f"{root}/Cargo.toml", encoding="utf-8").read()).group(1)
if not re.fullmatch(r"\d+\.\d+\.\d+", tauri):
    sys.exit(f"Invalid version '{tauri}': expected MAJOR.MINOR.PATCH")
if not (tauri == npm == cargo):
    sys.exit(f"Versions differ: tauri.conf.json {tauri}, package.json {npm}, Cargo.toml {cargo}. Run scripts/bump-version.sh")
print(tauri)
PY
}
