#!/usr/bin/env bash
# Fails when the live-credit version tokens or their change feed are written outside the bump and
# record functions of diesel_models::query::credit_inputs (live-recompute v2, rev 6 4.2):
#   - SQL writing credit_customer_inputs, credit_tenant_inputs or credit_input_change(_retention)
#     (INSERT, UPDATE, DELETE, TRUNCATE, ALTER TABLE, COPY), outside that file;
#   - diesel DSL use of those tables (schema paths), outside that file and schema.rs;
#   - bump_tenant / bump_tenants called anywhere but StoreInternal::transaction_bumping_tenants,
#     the only place that makes the tenant bump its transaction's last statement.
# A customer bump needs a CustomerLock, which the compiler checks. Tests and migrations are out of
# scope. Limits: a table name assembled at runtime (format!, concat) is not seen.
set -euo pipefail

ROOT="${1:-.}"
SOURCES=("$ROOT/modules/meteroid/crates" "$ROOT/modules/meteroid/src")
# EE modules (the usage controls' engine) write through the same functions; the OSS export has none.
if [[ -d "$ROOT/ee/modules" ]]; then
  SOURCES+=("$ROOT/ee/modules")
fi
OWNER="modules/meteroid/crates/diesel-models/src/query/credit_inputs.rs"
SCHEMA="modules/meteroid/crates/diesel-schema/src/schema.rs"
FINISHER="modules/meteroid/crates/meteroid-store/src/store.rs"
TABLES='credit_(customer_inputs|tenant_inputs|input_change(_retention)?)'

failed=0
report() {
  echo "❌ $1" >&2
  failed=1
}

rel() { printf '%s' "${1#"$ROOT"/}"; }

while IFS= read -r -d '' file; do
  path="$(rel "$file")"
  [[ "$path" == "$OWNER" ]] && continue

  while IFS= read -r hit; do
    report "$path:$hit writes a credit input table outside $OWNER"
  done < <(grep -nEi "(insert[[:space:]]+into|update|delete[[:space:]]+from|truncate([[:space:]]+table)?|alter[[:space:]]+table|copy)[[:space:]]+(only[[:space:]]+)?\"?${TABLES}\b" "$file" | cut -d: -f1 || true)

  if [[ "$path" != "$SCHEMA" ]]; then
    while IFS= read -r hit; do
      report "$path:$hit uses the diesel DSL of a credit input table outside $OWNER"
    done < <(grep -nE "(schema::${TABLES}\b|\b${TABLES}::)" "$file" | cut -d: -f1 || true)
  fi

  while IFS= read -r hit; do
    line="${hit%%:*}"
    if [[ "$path" != "$FINISHER" ]]; then
      report "$path:$line calls bump_tenant(s): end the transaction with transaction_bumping_tenant(s) instead"
      continue
    fi
    fn=$(awk -v target="$line" 'NR <= target && /fn [A-Za-z0-9_]+/ { match($0, /fn [A-Za-z0-9_]+/); name = substr($0, RSTART + 3, RLENGTH - 3) } NR == target { print name; exit }' "$file")
    if [[ "$fn" != "transaction_bumping_tenants" ]]; then
      report "$path:$line calls bump_tenant(s) from $fn, not transaction_bumping_tenants"
    fi
  done < <(grep -nE "\bbump_tenants?\(" "$file" | grep -vE "^[0-9]+:[[:space:]]*//" || true)
done < <(find "${SOURCES[@]}" -name '*.rs' -not -path '*/target/*' -not -path '*/ee/modules/*/tests/*' -print0 | sort -z)

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
echo "✅ Credit input tokens and feed are written only by diesel_models::query::credit_inputs."
