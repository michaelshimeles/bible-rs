#!/usr/bin/env bash
# Checks the App Store listing in apps/ios/AppStore/metadata against App Store Connect's limits,
# so a field that's too long fails here rather than in the browser on submission day.
#
#   scripts/check-app-store-metadata.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
META="$ROOT/apps/ios/AppStore/metadata"
failed=0

# Characters, not bytes: the description's bullets are three bytes each.
UTF8=C.UTF-8
[[ "$(uname -s)" == "Darwin" ]] && UTF8=en_US.UTF-8

# The file's length in characters, without its final newline (which $(...) drops).
chars() { printf '%s' "$(cat "$1")" | LC_ALL="$UTF8" wc -m | tr -d ' '; }

check() {
  local file="$META/$1" max="$2" n
  if [[ ! -s "$file" ]]; then
    echo "missing or empty: ${file#"$ROOT"/}" >&2
    failed=1
    return
  fi
  n="$(chars "$file")"
  if ((n > max)); then
    echo "${file#"$ROOT"/}: $n characters, the limit is $max" >&2
    failed=1
  fi
}

check en-US/name.txt 30
check en-US/subtitle.txt 30
check en-US/keywords.txt 100
check en-US/promotional_text.txt 170
check en-US/description.txt 4000
check en-US/release_notes.txt 4000
check review_information/notes.txt 4000
check copyright.txt 200
check primary_category.txt 40
check secondary_category.txt 40

for url in support_url marketing_url privacy_url; do
  check "en-US/$url.txt" 255
  if ! grep -Eq '^https://[^[:space:]]+$' "$META/en-US/$url.txt" 2>/dev/null; then
    echo "en-US/$url.txt is not one https:// URL" >&2
    failed=1
  fi
done

if grep -q ', \| ,' "$META/en-US/keywords.txt"; then
  echo "en-US/keywords.txt: no spaces around the commas; they count against the 100" >&2
  failed=1
fi

((failed == 0)) && echo "App Store metadata is within App Store Connect's limits"
exit "$failed"
