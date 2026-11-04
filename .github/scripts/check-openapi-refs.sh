#!/usr/bin/env bash
# Fails if a committed OpenAPI spec has a local `$ref` that resolves to nothing.
#
# utoipa emits a `$ref` for any `ToSchema` type a route uses, but only registers the component
# when it is reachable from a body or response, so a query-parameter enum can dangle. SDK
# generators then refuse the whole spec.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  set -- spec/api/v1/openapi.json spec/client/v1/openapi.json
fi

python3 - "$@" <<'PY'
import json
import sys


def unescape(token):
    return token.replace("~1", "/").replace("~0", "~")


def resolves(doc, pointer):
    node = doc
    for token in pointer.split("/")[1:]:
        token = unescape(token)
        if isinstance(node, dict) and token in node:
            node = node[token]
        elif isinstance(node, list) and token.isdigit() and int(token) < len(node):
            node = node[int(token)]
        else:
            return False
    return True


def walk(node, path, out):
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str):
                out.append((value, path))
            else:
                walk(value, f"{path}/{key}", out)
    elif isinstance(node, list):
        for i, value in enumerate(node):
            walk(value, f"{path}/{i}", out)


failed = False
for spec in sys.argv[1:]:
    try:
        with open(spec, encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError) as err:
        print(f"cannot read {spec}: {err}", file=sys.stderr)
        sys.exit(2)
    refs = []
    walk(doc, "#", refs)
    dangling = [(ref, at) for ref, at in refs if not ref.startswith("#") or not resolves(doc, ref[1:])]
    if dangling:
        failed = True
        print(f"❌ {spec}: {len(dangling)} unresolved $ref(s):", file=sys.stderr)
        for ref, at in dangling:
            print(f"  {ref}  (at {at})", file=sys.stderr)
    else:
        print(f"✅ {spec}: all {len(refs)} $ref(s) resolve.")

if failed:
    print(
        "\nRegister the missing schema (or mark the parameter `#[param(inline)]`), then run "
        "'cargo run -p meteroid-oss-openapi'.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
