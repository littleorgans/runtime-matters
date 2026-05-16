#!/usr/bin/env bash
set -euo pipefail

limit=700
status=0

while IFS= read -r file; do
  lines="$(wc -l < "$file" | tr -d ' ')"
  if (( lines > limit )); then
    printf '%s has %s lines, over limit %s\n' "$file" "$lines" "$limit" >&2
    status=1
  fi
done < <(find crates -type f \( -name '*.rs' -o -name '*.toml' \) | sort)

exit "$status"

