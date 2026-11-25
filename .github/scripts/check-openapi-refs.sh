#!/usr/bin/env bash
# Fails if a committed OpenAPI spec would break SDK generation:
#
# - a local `$ref` that resolves to nothing. utoipa emits a `$ref` for any `ToSchema` type a
#   route uses, but only registers the component when it is reachable from a body or response,
#   so a query-parameter enum can dangle;
# - a discriminator mapping a tag to a missing schema, or to a Rust wrapper's name (`Box`, `Vec`…:
#   a `Box<T>` variant maps to `Box`);
# - two tags of one discriminator mapped to the same schema (two variants wrapping one struct):
#   a generator makes one type per schema and cannot tell them apart;
# - in a v2 document, a member of a discriminated union left inline, or out of its mapping
#   (v1, frozen, keeps the unit variants of its `CreditExhaustedRule` inline).
#
# Two Rust types giving one component name leave no trace in the document (a merge keeps one):
# the generators refuse to write a v2 document then (`v2::assert_one_type_per_component`), and
# `cargo test` fails (`every_component_name_is_one_type`).
#
# With no argument, checks every committed spec, the Enterprise edition's included when ee/ is
# there: those are the documents SDKs are generated from.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  for spec in \
    spec/api/v1/openapi.json spec/api/v2/openapi.json spec/client/v1/openapi.json \
    ee/spec/api/v1/openapi.json ee/spec/api/v2/openapi.json ee/spec/client/v1/openapi.json; do
    if [[ -f "$spec" ]]; then
      set -- "$@" "$spec"
    elif [[ "$spec" != ee/* ]]; then
      echo "missing $spec" >&2
      exit 2
    fi
  done
fi

python3 - "$@" <<'PY'
import json
import sys

WRAPPERS = {"Box", "Vec", "Option", "Arc", "Rc", "Cow", "HashMap", "BTreeMap"}
SCHEMAS = "#/components/schemas/"


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


def unions(node, path, out):
    """Every schema carrying a discriminator, wherever it is."""
    if isinstance(node, dict):
        if isinstance(node.get("discriminator"), dict):
            out.append((path, node))
        for key, value in node.items():
            unions(value, f"{path}/{key}", out)
    elif isinstance(node, list):
        for i, value in enumerate(node):
            unions(value, f"{path}/{i}", out)


def discriminator_problems(doc, named_members):
    found = []
    union_schemas = []
    unions(doc.get("components", {}), "#/components", union_schemas)
    for at, schema in union_schemas:
        mapping = schema["discriminator"].get("mapping") or {}
        by_target = {}
        for tag, target in mapping.items():
            name = target[len(SCHEMAS):] if target.startswith(SCHEMAS) else target
            if name in WRAPPERS:
                found.append(f"{at}: `{tag}` maps to {target}, a Rust wrapper's name")
            elif not target.startswith("#") or not resolves(doc, target[1:]):
                found.append(f"{at}: `{tag}` maps to the missing {target}")
            by_target.setdefault(target, []).append(tag)
        for target, tags in by_target.items():
            if len(tags) > 1:
                found.append(f"{at}: {', '.join(f'`{t}`' for t in tags)} all map to {target}")
        members = (schema.get("oneOf") or schema.get("anyOf") or []) if named_members else []
        mapped = set(mapping.values())
        for i, member in enumerate(members):
            reference = member.get("$ref") if isinstance(member, dict) else None
            if reference is None:
                found.append(f"{at}/oneOf/{i}: a member is inline, without a schema of its own")
            elif mapping and reference not in mapped:
                found.append(f"{at}/oneOf/{i}: {reference} is in no mapping")
    return found


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
    discriminators = discriminator_problems(doc, "/api/v2/" in spec)
    if dangling:
        failed = True
        print(f"❌ {spec}: {len(dangling)} unresolved $ref(s):", file=sys.stderr)
        for ref, at in dangling:
            print(f"  {ref}  (at {at})", file=sys.stderr)
    if discriminators:
        failed = True
        print(f"❌ {spec}: {len(discriminators)} discriminator problem(s):", file=sys.stderr)
        for problem in discriminators:
            print(f"  {problem}", file=sys.stderr)
    if not dangling and not discriminators:
        print(
            f"✅ {spec}: all {len(refs)} $ref(s) resolve; each discriminator maps every tag "
            "to a schema of its own."
        )

if failed:
    print(
        "\nRegister the missing schema (or mark the parameter `#[param(inline)]`), give each "
        "variant of a tagged enum a struct of its own (not boxed), then run "
        "'cargo run -p meteroid-oss-openapi' (and 'cargo run -p meteroid-enterprise-openapi' "
        "in ee/).",
        file=sys.stderr,
    )
    sys.exit(1)
PY
