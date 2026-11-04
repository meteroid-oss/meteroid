#!/usr/bin/env bash
# Fails if two diesel migrations share the same version.
#
# Diesel keys __diesel_schema_migrations by version (the digits of the
# directory-name prefix before the first "_"). Two dirs with the same prefix
# collapse to one version row, so only the first-sorted one ever runs and the
# other is silently skipped — with no error at migrate time. This guards
# against that at CI time instead of in prod.
set -euo pipefail

MIG_DIR="${1:-modules/meteroid/migrations/diesel}"

if [[ ! -d "$MIG_DIR" ]]; then
  echo "migration dir not found: $MIG_DIR" >&2
  exit 2
fi

# version key = prefix before the first "_", digits only (matches diesel).
dupes=$(
  for d in "$MIG_DIR"/*/; do
    name="$(basename "$d")"
    version="${name%%_*}"
    version="${version//[^0-9]/}"
    echo "$version"
  done | sort | uniq -d
)

if [[ -n "$dupes" ]]; then
  echo "❌ Duplicate diesel migration version(s) detected:" >&2
  while IFS= read -r v; do
    [[ -z "$v" ]] && continue
    echo "  version $v:" >&2
    for d in "$MIG_DIR"/*/; do
      name="$(basename "$d")"
      key="${name%%_*}"; key="${key//[^0-9]/}"
      [[ "$key" == "$v" ]] && echo "    - $name" >&2
    done
  done <<< "$dupes"
  echo >&2
  echo "Rename one so each migration has a unique version prefix." >&2
  exit 1
fi

echo "✅ All migration versions are unique."
