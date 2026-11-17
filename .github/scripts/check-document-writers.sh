#!/usr/bin/env bash
# Fails when an invoice or a credit note is written outside an allowlisted function.
#
# A document is written through `insert_invoice_tx` (invoices) or `create_credit_note_tx` (credit
# notes). Each function calling one is listed in document-writers.allowlist as
# <path>::<impl type>::<fn> | class | reason:
#   primitive  builds one kind of document; its callers are checked, by its name, in turn
#   writer     decides to write the document: the billing controller (lifecycle::derived) for a
#              subscription's period, final and consolidated documents, a change's path for its own
#              adjustment invoice and notes, an operator's or a checkout's path for theirs
# Tests are out of scope. Functions are attributed by brace depth, strings and comments stripped
# per line; a primitive's callers are matched by its name.
set -euo pipefail

ALLOWLIST="${1:-.github/scripts/document-writers.allowlist}"
SOURCES=(modules/meteroid/crates modules/meteroid/src)
if [[ -d ee/modules ]]; then
  SOURCES+=(ee/modules)
fi
NAMES='insert_invoice_tx|create_credit_note_tx'

if [[ ! -f "$ALLOWLIST" ]]; then
  echo "allowlist not found: $ALLOWLIST" >&2
  exit 2
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  printf '%s' "${value%"${value##*[![:space:]]}"}"
}

declare -A listed
failed=0
while IFS= read -r raw; do
  [[ -z "${raw// /}" || "$raw" == \#* ]] && continue
  IFS='|' read -r key class reason <<< "$raw"
  key="$(trim "$key")"; class="$(trim "$class")"; reason="$(trim "${reason:-}")"
  if [[ "$class" != primitive && "$class" != writer ]]; then
    echo "❌ $ALLOWLIST: unknown class '$class' for $key" >&2; failed=1
  fi
  if [[ "$class" == primitive ]]; then
    NAMES+="|${key##*::}"
  fi
  if [[ -z "$reason" ]]; then
    echo "❌ $ALLOWLIST: $key has no reason" >&2; failed=1
  fi
  if [[ -n "${listed[$key]:-}" ]]; then
    echo "❌ $ALLOWLIST: $key is listed twice" >&2; failed=1
  fi
  listed[$key]=1
done < "$ALLOWLIST"

CALLS="(^|[^A-Za-z0-9_])(${NAMES})[(]"
DEFS="(^|[^A-Za-z0-9_])fn (${NAMES})[<(]"

sites=$(
  find "${SOURCES[@]}" -name '*.rs' -not -path '*/target/*' -not -path '*/tests/*' -not -path '*/meteroid-testkit/*' -print0 | sort -z |
    xargs -0 awk -v calls="$CALLS" -v defs="$DEFS" '
      function key_of(name,    k, i) {
        k = FILENAME
        for (i = 1; i <= sp; i++) if (kind[i] == "type") k = k "::" scope[i]
        return k "::" name
      }
      function innermost_fn(    i) {
        for (i = sp; i >= 1; i--) if (kind[i] == "fn") return fnkey[i]
        return ""
      }
      FNR == 1 { depth = 0; sp = 0; pending = ""; pkind = ""; intest = 0 }
      {
        line = $0
        code = line; sub(/^[[:space:]]+/, "", code)
        if (code ~ /^\/\//) next
        if (code ~ /^#\[cfg\(test\)\]/) { intest = 1 }
        bare = line
        gsub(/\\\\/, "", bare); gsub(/\\"/, "", bare); gsub(/"[^"]*"/, "\"\"", bare)
        sub(/\/\/.*$/, "", bare)
        if (match(bare, /(^|[^A-Za-z0-9_])fn [A-Za-z0-9_]+/)) {
          s = substr(bare, RSTART, RLENGTH); sub(/^[^f]*fn /, "", s)
          pending = key_of(s); pkind = "fn"
        } else if (bare ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)([[:space:]<]|$)/) {
          t = bare
          sub(/[[:space:]]*\{.*$/, "", t); sub(/[[:space:]]+where[[:space:]].*$/, "", t)
          if (t ~ /[[:space:]]for[[:space:]]/) sub(/^.*[[:space:]]for[[:space:]]+/, "", t)
          else { sub(/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)(<[^>]*>)?[[:space:]]*/, "", t) }
          sub(/<.*$/, "", t); n = split(t, parts, "::"); t = parts[n]; gsub(/[^A-Za-z0-9_]/, "", t)
          pending = t; pkind = "type"
        }
        if (!intest && bare ~ calls && bare !~ defs) {
          item = innermost_fn()
          if (item == "") item = FILENAME "::<top level>"
          if (!(item in seen)) { seen[item] = 1; order[++count] = item }
        }
        for (i = 1; i <= length(bare); i++) {
          c = substr(bare, i, 1)
          if (c == "{") {
            depth++
            if (pkind != "") {
              sp++; kind[sp] = pkind; scope[sp] = pending; fnkey[sp] = pending; at[sp] = depth
              pkind = ""
            }
          } else if (c == "}") {
            if (sp > 0 && at[sp] == depth) sp--
            depth--
          }
        }
        if (pkind == "fn" && index(bare, ";")) pkind = ""
      }
      END { for (i = 1; i <= count; i++) print order[i] }
    '
)

declare -A used
while IFS= read -r key; do
  [[ -z "$key" ]] && continue
  if [[ -n "${listed[$key]:-}" ]]; then
    used[$key]=1
    continue
  fi
  echo "❌ $key writes a document, and $ALLOWLIST does not list it" >&2
  echo "   A subscription's documents are the billing controller's (lifecycle::derived); list a" >&2
  echo "   change's own adjustment or note writer, or an operator path, with the reason." >&2
  failed=1
done <<< "$sites"

for key in "${!listed[@]}"; do
  if [[ -z "${used[$key]:-}" ]]; then
    echo "❌ $ALLOWLIST lists $key, which writes no document any more: remove the entry" >&2
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
echo "✅ Every document write is in an allowlisted function ($(grep -c . <<< "$sites") functions)."
