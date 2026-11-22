"""Keeps the operations this client calls from Pennylane's published Company API v2 spec.

    curl -sSL https://pennylane.readme.io/openapi/accounting.json -o /tmp/accounting.json
    python3 -I trim.py /tmp/accounting.json > pennylane-v2.json
"""
import json
import sys

OPERATIONS = {
    "/api/external/v2/me": ["get"],
    "/api/external/v2/customers": ["get"],
    "/api/external/v2/company_customers": ["post"],
    "/api/external/v2/company_customers/{id}": ["put"],
    "/api/external/v2/individual_customers": ["post"],
    "/api/external/v2/individual_customers/{id}": ["put"],
    "/api/external/v2/customer_invoices": ["get"],
    "/api/external/v2/customer_invoices/{id}": ["get"],
    "/api/external/v2/customer_invoices/import": ["post"],
    "/api/external/v2/customer_invoices/{id}/update_imported": ["put"],
    "/api/external/v2/customer_invoices/{id}/mark_as_paid": ["put"],
    "/api/external/v2/file_attachments": ["post"],
}

spec = json.load(open(sys.argv[1]))


def refs(node, found):
    if isinstance(node, dict):
        ref = node.get("$ref")
        if ref and ref not in found:
            found.add(ref)
            target = spec
            for part in ref[2:].split("/"):
                target = target[part]
            refs(target, found)
        for value in node.values():
            refs(value, found)
    elif isinstance(node, list):
        for value in node:
            refs(value, found)


paths = {
    path: {method: spec["paths"][path][method] for method in methods}
    for path, methods in OPERATIONS.items()
}
found = set()
refs(paths, found)

components = {}
for ref in sorted(found):
    _, section, name = ref[2:].split("/")
    components.setdefault(section, {})[name] = spec["components"][section][name]
components["securitySchemes"] = spec["components"]["securitySchemes"]

json.dump(
    {
        "openapi": spec["openapi"],
        "info": spec["info"],
        "servers": spec["servers"],
        "paths": paths,
        "components": components,
    },
    sys.stdout,
    indent=1,
    sort_keys=True,
)
print()
