#!/usr/bin/env bash
# Fails when code reads the clock directly: the present comes from `common_domain::clock::now`,
# which tests can move, and the real time from `common_domain::clock::wall`. SQL that reads the
# database's clock is a direct read too, unless clock-sites.allowlist names it.
set -euo pipefail

ROOT="${1:-.}"
SOURCES=(
  "$ROOT/modules/meteroid/crates/meteroid-store/src"
  "$ROOT/modules/meteroid/crates/diesel-models/src"
  "$ROOT/modules/meteroid/crates/meteroid-workers/src"
  "$ROOT/modules/meteroid/crates/meteroid-api/src"
  "$ROOT/modules/meteroid/crates/meteroid-rest/src"
  "$ROOT/modules/meteroid/crates/meteroid-rest-common/src"
  "$ROOT/modules/meteroid/crates/meteroid-grpc-mapping/src"
  "$ROOT/modules/meteroid/crates/meteroid-app/src"
  "$ROOT/modules/meteroid/crates/meteroid-testkit/src"
)
ALLOWLIST="$ROOT/.github/scripts/clock-sites.allowlist"
SQL_CLOCK='diesel::dsl::now\b|(?<![\w:])now\(\)|\bCURRENT_(DATE|TIMESTAMP)\b|clock_timestamp\(\)|\bLOCALTIMESTAMP\b'

allowed() {
  local path=$1 text=$2 entry_path fragment
  while IFS='|' read -r entry_path fragment _; do
    entry_path=${entry_path%% } fragment=${fragment# } fragment=${fragment% }
    [[ -n "$entry_path" && "$path" == "$entry_path" && "$text" == *"$fragment"* ]] && return 0
  done < <(grep -vE '^\s*(#|$)' "$ALLOWLIST")
  return 1
}

hits="$(grep -rnP "\b(Utc|Local)::now\b|SystemTime::now\(|$SQL_CLOCK" --include='*.rs' "${SOURCES[@]}" || true)"
failures=()
while IFS= read -r hit; do
  [[ -z "$hit" ]] && continue
  path=${hit%%:*} path=${path#"$ROOT"/}
  text=${hit#*:*:}
  [[ "$text" =~ ^[[:space:]]*// || "$text" =~ fn[[:space:]]+now\( ]] && continue
  [[ "$text" =~ (Utc|Local)::now|SystemTime::now ]] || ! allowed "$path" "$text" || continue
  failures+=("$path:${hit#*:}")
done <<<"$hits"

if ((${#failures[@]})); then
  echo "❌ Read the clock through common_domain::clock::now() (or ::wall() for real time), and bind it in SQL:" >&2
  for f in "${failures[@]}"; do echo "   ${f:0:200}" >&2; done
  exit 1
fi
echo "✅ The clock is read through common_domain::clock."
