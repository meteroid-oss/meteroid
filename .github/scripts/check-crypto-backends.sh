#!/usr/bin/env bash
# Fails when a dependency change could bring back a crypto-provider panic at runtime.
#
# ring is the only crypto backend compiled in. rustls picks it on its own only while aws-lc is
# absent; reqwest and jsonwebtoken get it from wrappers (http_client_tracing::client_builder,
# meteroid_store::jwt) that clippy enforces in our code but cannot see in a dependency's. So:
#   - aws-lc must not be in a lockfile;
#   - only allowlisted third-party crates may depend on reqwest 0.13 or jsonwebtoken, after
#     checking they never build a client or sign a token themselves.
set -euo pipefail
cd "$(dirname "$0")/../.."

status=0

for lock in Cargo.lock ee/Cargo.lock; do
  [[ -f "$lock" ]] || continue
  if grep -qE '^name = "aws-lc-(rs|sys)"' "$lock"; then
    echo "::error file=$lock::aws-lc is in $lock: rustls can no longer pick ring on its own"
    status=1
  fi
done

# <crate spec> <allowed third-party dependents...>
check_dependents() {
  local crate=$1
  shift
  local dependents
  dependents=$(cargo tree --workspace --target all -e normal,dev -i "$crate" --depth 1 --prefix none 2>/dev/null |
    tail -n +2 | awk '!/ \(\// {print $1}' | sort -u)
  for dependent in $dependents; do
    if [[ " $* " != *" $dependent "* ]]; then
      echo "::error::$dependent depends on $crate: check it never builds a client or token itself, then allowlist it in $0"
      status=1
    fi
  done
}

check_dependents reqwest@0.13 oauth2-reqwest reqwest-middleware reqwest-retry
check_dependents jsonwebtoken

exit $status
