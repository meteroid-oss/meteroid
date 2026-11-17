#!/usr/bin/env bash
# Fails when a subscription's facts are written outside an allowlisted function.
#
# The facts: subscription_plan_window, subscription_trial, subscription_start_settled,
# subscription_ending, the dated minimums (subscription_adjustment and its per-product scopes,
# subscription_adjustment_component) and applied_coupon.effective_from (set when the row is
# inserted). A write site is a line of a function that:
#   - writes one of those tables: SQL INSERT INTO / UPDATE / DELETE FROM it, a diesel
#     insert_into / update / delete of its table, any diesel write in subscription_adjustments.rs,
#     an insert into applied_coupon, a write of effective_from in applied_coupons.rs;
#   - calls a `primitive` of fact-writers.allowlist (`Type::fn(`, `Self::fn(` inside an impl of
#     `Type`, or a free `fn(`);
#   - builds a row that is only ever inserted: SubscriptionPlanWindowRowNew, SubscriptionTrialRow,
#     SubscriptionStartSettledRow, SubscriptionEndingRow, SubscriptionAdjustmentRowNew,
#     SubscriptionAdjustmentComponentRow, AppliedCouponRowNew (which catches `row.insert(conn)`).
# Each such function is listed as <path>::<impl type>::<fn> (or <path>::* for a whole module) with
# a class and a reason:
#   primitive  a diesel-models query function writing a fact table; its callers are checked. A
#              call `value.fn(` is not seen, so a primitive taking `self` is refused unless its
#              type is one of the rows above, whose building is the write site
#   writer     a function allowed to write facts, directly or through a primitive
# Tests and migrations are out of scope. Limits: functions are attributed by brace depth with
# strings and comments stripped per line; a table name assembled at runtime is not seen.
set -euo pipefail

ALLOWLIST="${1:-.github/scripts/fact-writers.allowlist}"
SOURCES=(modules/meteroid/crates modules/meteroid/src)
# EE modules read facts but write none; scanned so a new writer there is seen.
if [[ -d ee/modules ]]; then
  SOURCES+=(ee/modules)
fi
CLASSES=" primitive writer "
TABLES='subscription_plan_window|subscription_trial|subscription_start_settled|subscription_ending|subscription_adjustment|subscription_adjustment_component'
ROWS='SubscriptionPlanWindowRowNew|SubscriptionTrialRow|SubscriptionStartSettledRow|SubscriptionEndingRow|SubscriptionAdjustmentRowNew|SubscriptionAdjustmentComponentRow|AppliedCouponRowNew'
DATED_QUERIES='diesel-models/src/query/subscription_adjustments[.]rs$'
COUPON_QUERIES='diesel-models/src/query/applied_coupons[.]rs$'

if [[ ! -f "$ALLOWLIST" ]]; then
  echo "allowlist not found: $ALLOWLIST" >&2
  exit 2
fi

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  printf '%s' "${value%"${value##*[![:space:]]}"}"
}

declare -A class_of
failed=0
calls=""
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
  if [[ "$class" == primitive ]]; then
    if [[ "$key" != modules/meteroid/crates/diesel-models/src/query/*.rs::* || "$key" == *'::*' ]]; then
      echo "❌ $ALLOWLIST: primitive $key is not one diesel-models query function" >&2; failed=1
      continue
    fi
    callee="${key#*.rs::}"
    if [[ "$callee" == *::* ]]; then
      calls+="${calls:+|}${callee}[(]"
    else
      calls+="${calls:+|}(^|[^A-Za-z0-9_])${callee}[(]"
    fi
  fi
done < "$ALLOWLIST"

# One line per item holding a write site: "<path>::<item>".
sites=$(
  find "${SOURCES[@]}" -name '*.rs' -not -path '*/target/*' -not -path '*/tests/*' \
    -not -path '*/diesel-schema/*' -print0 | sort -z |
    xargs -0 awk -v tables="$TABLES" -v rows="$ROWS" -v calls="$calls" -v dated="$DATED_QUERIES" -v coupons="$COUPON_QUERIES" '
      function key_of(name,    k, i) {
        k = FILENAME
        for (i = 1; i <= sp; i++) if (kind[i] == "type") k = k "::" scope[i]
        return k "::" name
      }
      function innermost_fn(    i) {
        for (i = sp; i >= 1; i--) if (kind[i] == "fn") return fnkey[i]
        return ""
      }
      function innermost_type(    i) {
        for (i = sp; i >= 1; i--) if (kind[i] == "type") return scope[i]
        return ""
      }
      BEGIN {
        sql = "(INSERT INTO|UPDATE|DELETE FROM)[[:space:]]+(" tables ")([^A-Za-z0-9_]|$)|INSERT INTO[[:space:]]+applied_coupon([^A-Za-z0-9_]|$)"
        dsl = "(insert_into|update|delete)\\([[:space:]]*([A-Za-z0-9_]+::)*(" tables ")([^A-Za-z0-9_]|$)"
        coupon = "insert_into\\([[:space:]]*([A-Za-z0-9_]+::)*applied_coupon([^A-Za-z0-9_]|$)"
        built = "(^|[^A-Za-z0-9_])(" rows ")[[:space:]]*\\{"
        anywrite = "diesel::(insert_into|update|delete)\\("
        fndef = "(^|[^A-Za-z0-9_])fn [A-Za-z0-9_]+[<(]"
        couponset = "effective_from[[:space:]]*=[^=>]|effective_from[.]eq[(]"
        receiver = "^[[:space:]]*(&[[:space:]]*(\047[A-Za-z_]+[[:space:]]+)?)?(mut[[:space:]]+)?self([^A-Za-z0-9_]|$)"
      }
      FNR == 1 {
        depth = 0; sp = 0; pending = ""; pkind = ""; sig = ""
        indated = (FILENAME ~ dated); incoupons = (FILENAME ~ coupons)
      }
      {
        line = $0
        code = line; sub(/^[[:space:]]+/, "", code)
        if (code ~ /^\/\//) next
        bare = line
        gsub(/\\\\/, "", bare); gsub(/\\"/, "", bare); gsub(/"[^"]*"/, "\"\"", bare)
        sub(/\/\/.*$/, "", bare)

        if (match(bare, /(^|[^A-Za-z0-9_])fn [A-Za-z0-9_]+/)) {
          s = substr(bare, RSTART, RLENGTH); sub(/^[^f]*fn /, "", s)
          pending = key_of(s); pkind = "fn"
          sig = substr(bare, RSTART + RLENGTH) " "; sigkey = pending; sigline = FNR
        } else if (bare ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)([[:space:]<]|$)/) {
          t = bare
          sub(/[[:space:]]*\{.*$/, "", t); sub(/[[:space:]]+where[[:space:]].*$/, "", t)
          if (t ~ /[[:space:]]for[[:space:]]/) sub(/^.*[[:space:]]for[[:space:]]+/, "", t)
          else { sub(/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?(impl|trait)(<[^>]*>)?[[:space:]]*/, "", t) }
          sub(/<.*$/, "", t); n = split(t, parts, "::"); t = parts[n]; gsub(/[^A-Za-z0-9_]/, "", t)
          pending = t; pkind = "type"
        }

        if (sig != "" && sigline != FNR) sig = sig " " bare
        # A function whose first parameter is self: its callers name the value, not its type.
        if (sig != "" && (index(sig, "{") || index(sig, ";"))) {
          params = sig; sub(/^[^(]*[(]/, "", params)
          if (params ~ receiver) print "receiver " sigkey
          sig = ""
        }

        hit = (line ~ sql) || (bare ~ dsl) || (bare ~ coupon) || (indated && bare ~ anywrite) ||
          (incoupons && line ~ couponset)
        # Inside an impl, `Self::fn(` is a call of `Type::fn(`.
        called = bare
        if (innermost_type() != "") gsub(/Self::/, innermost_type() "::", called)
        if (!hit && calls != "" && bare !~ fndef && called ~ calls) hit = 1
        if (!hit && bare ~ built && bare !~ fndef && bare !~ /(^|[^A-Za-z0-9_])(struct|impl|enum)[[:space:]]/) hit = 1
        if (hit) {
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
      END { for (i = 1; i <= count; i++) print "site " order[i] }
    '
)

declare -A takes_self
while IFS= read -r line; do
  [[ "$line" == "receiver "* ]] && takes_self[${line#receiver }]=1
done <<< "$sites"
for key in "${!class_of[@]}"; do
  callee="${key#*.rs::}"
  if [[ "${class_of[$key]}" == primitive && -n "${takes_self[$key]:-}" &&
        ! "${callee%%::*}" =~ ^($ROWS)$ ]]; then
    echo "❌ $ALLOWLIST: primitive $key takes self: a call value.${callee##*::}( is not seen" >&2
    echo "   Take the value as an argument, ${callee}(&value, ...), which its callers name." >&2
    failed=1
  fi
done
sites=$(sed -n 's/^site //p' <<< "$sites")

declare -A used
while IFS= read -r key; do
  [[ -z "$key" ]] && continue
  if [[ -n "${class_of[$key]:-}" ]]; then
    used[$key]=1
    continue
  fi
  module="${key%%.rs::*}.rs::*"
  if [[ -n "${class_of[$module]:-}" ]]; then
    used[$module]=1
    continue
  fi
  echo "❌ $key writes a subscription's facts, and $ALLOWLIST does not list it" >&2
  echo "   Record facts through services::facts (or a primitive it calls), or list the function" >&2
  echo "   with its class and the reason it decides the fact." >&2
  failed=1
done <<< "$sites"

for key in "${!class_of[@]}"; do
  if [[ -z "${used[$key]:-}" ]]; then
    echo "❌ $ALLOWLIST lists $key, which writes no fact any more: remove the entry" >&2
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
echo "✅ Every fact write is in an allowlisted function ($(grep -c . <<< "$sites") functions)."
