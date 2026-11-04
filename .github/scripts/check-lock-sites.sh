#!/usr/bin/env bash
# Fails when a Postgres lock is taken outside an allowlisted function.
#
# Row locks follow one order (diesel_models::lock): the customer row first, then the rows that
# belong to it. Locking a customer's row needs a `CustomerLock`, which only taking the customer
# lock produces, so the compiler enforces the order; this check keeps every lock site in a
# reviewed function. A function or const holding a lock clause (FOR UPDATE, FOR NO KEY UPDATE,
# FOR SHARE, FOR KEY SHARE, SKIP LOCKED, NOWAIT, their diesel builders, LOCK TABLE, or an
# advisory lock) must appear in the allowlist as <path>::<impl type>::<fn> with a class and reason:
#   customer   takes the customer lock first in its transaction; returns the CustomerLock
#   derive     reads a row's customer unlocked, locks it, then the row; returns the CustomerLock
#   dependent  locks rows of one customer: takes the CustomerLock and checks the rows belong to it
#              (calls `covers` or filters on the lock's `customer_id()`)
#   tenant     a tenant-wide row (entity, coupon, connector, sequence): the reason says where it sits
#   claim      a SKIP LOCKED claim or queue: never waits, committed before any customer lock
#   standalone locks rows of its own kind only, in a transaction that locks no customer
#   advisory   an advisory lock: the reason says what it serializes and where it sits
# Limits: functions are attributed by brace depth with strings and comments stripped per line; a
# lock clause assembled at runtime (format!, concat) is seen only if its literal appears in the code.
set -euo pipefail

ALLOWLIST="${1:-.github/scripts/lock-sites.allowlist}"
SOURCES=(modules/meteroid/crates modules/meteroid/src)
LISTS=("$ALLOWLIST")
# EE modules hold queries too, listed in their own allowlist; the OSS export has neither.
if [[ -d ee/modules ]]; then
  SOURCES+=(ee/modules)
  LISTS+=(ee/lock-sites.allowlist)
fi
CLASSES=" customer derive dependent tenant claim standalone advisory "

if [[ ! -f "$ALLOWLIST" ]]; then
  echo "allowlist not found: $ALLOWLIST" >&2
  exit 2
fi

# One line per item holding a lock clause:
# "<path>::<item><TAB><signature names CustomerLock><TAB><body checks the customer>".
sites=$(
  find "${SOURCES[@]}" -name '*.rs' -not -path '*/target/*' -not -path 'ee/modules/*/tests/*' -print0 | sort -z |
    xargs -0 awk '
      function key_of(name,    k, i) {
        k = FILENAME
        for (i = 1; i <= sp; i++) if (kind[i] == "type") k = k "::" scope[i]
        return k "::" name
      }
      function innermost_fn(    i) {
        for (i = sp; i >= 1; i--) if (kind[i] == "fn") return fnkey[i]
        return ""
      }
      FNR == 1 { depth = 0; sp = 0; pending = ""; pkind = ""; constkey = ""; insig = 0 }
      {
        line = $0
        code = line; sub(/^[[:space:]]+/, "", code)
        if (code ~ /^\/\//) next
        # Strip string contents and trailing comments before counting braces.
        bare = line
        gsub(/\\\\/, "", bare); gsub(/\\"/, "", bare); gsub(/"[^"]*"/, "\"\"", bare)
        sub(/\/\/.*$/, "", bare)

        if (match(bare, /(^|[^A-Za-z0-9_])fn [A-Za-z0-9_]+/)) {
          s = substr(bare, RSTART, RLENGTH); sub(/^[^f]*fn /, "", s)
          pending = key_of(s); pkind = "fn"; sig = ""; insig = 1
        } else if (bare ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)([[:space:]<]|$)/) {
          t = bare
          sub(/[[:space:]]*\{.*$/, "", t); sub(/[[:space:]]+where[[:space:]].*$/, "", t)
          if (t ~ /[[:space:]]for[[:space:]]/) sub(/^.*[[:space:]]for[[:space:]]+/, "", t)
          else { sub(/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)(<[^>]*>)?[[:space:]]*/, "", t) }
          sub(/<.*$/, "", t); n = split(t, parts, "::"); t = parts[n]; gsub(/[^A-Za-z0-9_]/, "", t)
          pending = t; pkind = "type"
        } else if (sp == 0 && match(bare, /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(const|static)[[:space:]]+[A-Z0-9_]+/)) {
          s = substr(bare, RSTART, RLENGTH); sub(/.*(const|static)[[:space:]]+/, "", s)
          constkey = FILENAME "::" s
        }

        if (insig) {
          sig = sig line
          if (index(bare, "{") || index(bare, ";")) {
            insig = 0
            token[pending] = (sig ~ /CustomerLock/) ? "yes" : "no"
          }
        }

        item = innermost_fn()
        if (item == "" && constkey != "") item = constkey
        if (item != "" && line ~ /covers\(|customer_id\(\)/) checked[item] = "yes"
        if (line ~ /for_update\(\)|for_no_key_update\(\)|for_share\(\)|for_key_share\(\)|skip_locked\(\)|no_wait\(\)|FOR (NO KEY )?UPDATE|FOR (KEY )?SHARE|SKIP LOCKED|NOWAIT|LOCK TABLE|pg_(try_)?advisory(_xact)?_lock(_shared)?\(/) {
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
        if (pkind == "fn" && index(bare, ";") && !insig) pkind = ""
        if (constkey != "" && sp == 0 && bare ~ /;[[:space:]]*$/) constkey = ""
      }
      END {
        for (i = 1; i <= count; i++) {
          k = order[i]
          print k "\t" ((k in token) ? token[k] : "no") "\t" ((k in checked) ? "yes" : "no")
        }
      }
    '
)

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  printf '%s' "${value%"${value##*[![:space:]]}"}"
}

declare -A class_of
failed=0
while IFS= read -r raw; do
  [[ -z "${raw// /}" || "$raw" == \#* ]] && continue
  IFS='|' read -r key class reason <<< "$raw"
  key="$(trim "$key")"; class="$(trim "$class")"; reason="$(trim "${reason:-}")"
  if [[ "$CLASSES" != *" $class "* ]]; then
    echo "❌ $ALLOWLIST: unknown class '$class' for $key" >&2; failed=1
  fi
  if [[ -z "$reason" ]]; then
    echo "❌ $ALLOWLIST: $key has no reason" >&2; failed=1
  fi
  if [[ -n "${class_of[$key]:-}" ]]; then
    echo "❌ $ALLOWLIST: $key is listed twice" >&2; failed=1
  fi
  class_of[$key]="$class"
done < <(cat "${LISTS[@]}")

declare -A found
while IFS=$'\t' read -r key token checked; do
  [[ -z "$key" ]] && continue
  found[$key]=1
  class="${class_of[$key]:-}"
  if [[ -z "$class" ]]; then
    echo "❌ lock in $key, which $ALLOWLIST does not list" >&2
    echo "   Lock a customer's rows behind its CustomerLock (diesel_models::lock), in diesel-models," >&2
    echo "   and list the function with its class and the reason." >&2
    failed=1
    continue
  fi
  case "$class" in
    customer | derive | dependent)
      if [[ "$token" != "yes" ]]; then
        echo "❌ $key is '$class' but its signature does not carry the CustomerLock" >&2
        failed=1
      fi
      ;;
  esac
  if [[ "$class" == dependent && "$checked" != "yes" ]]; then
    echo "❌ $key is 'dependent' but never checks its rows' customer (covers / customer_id())" >&2
    failed=1
  fi
done <<< "$sites"

for key in "${!class_of[@]}"; do
  if [[ -z "${found[$key]:-}" ]]; then
    echo "❌ $ALLOWLIST lists $key, which takes no lock any more: remove the entry" >&2
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
echo "✅ Every lock is in an allowlisted function ($(grep -c . <<< "$sites") sites)."
