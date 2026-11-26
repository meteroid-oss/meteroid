#!/usr/bin/env bash
# Fails when a write that derived state follows happens outside an allowlisted function.
#
# No trigger derives anything: the diesel-models write functions do it in the same transaction.
# Every write of these kinds must go
# through one of the allowlisted functions, <path>::<impl type>::<fn> | <rule> | <reason>. Rules:
#   entitlement-insert      inserts into `entitlement` (its lineage anchor)
# A pragmatic lint: diesel DSL (aliases of tables and columns included) and raw SQL (any schema
# prefix, any case) are read over the whole repository, tests and the testkit aside.
set -euo pipefail

ALLOWLIST="${1:-.github/scripts/claim-sites.allowlist}"

python3 - "$ALLOWLIST" <<'PY'
import os, re, sys

allowlist = sys.argv[1]
TABLES = ("entitlement",)

def skipped(path):
    return any(part in path for part in (
        "/tests/", "/benches/", "meteroid-testkit", "/target/", "node_modules", "/.git/"))

def changesets(table, files):
    """AsChangeset structs of `table` with a `status` field."""
    names = set()
    for text in files.values():
        for m in re.finditer(
                r'#\[derive\([^)]*AsChangeset[^)]*\)\]\s*#\[diesel\(table_name\s*=\s*'
                r'(?:(?:crate|diesel_models|diesel_schema)::schema::)?%s\)\][^{]*?struct\s+(\w+)'
                r'[^{]*\{([^}]*)\}' % table, text, re.S):
            if re.search(r'\bstatus\s*:', m.group(2)):
                names.add(m.group(1))
    return names

def aliases(text, name):
    """Names `name` is imported or bound as in a file: `name as x`, `name::table as t`."""
    found = {name}
    for m in re.finditer(r'\b%s(?:::(?:table|dsl))?\s+as\s+(\w+)' % name, text):
        found.add(m.group(1))
    return found

def functions(path, text):
    """(key, text) per fn, its signature included, outside `mod tests`."""
    text = re.sub(r'(?m)^\s*//.*$', '', text)
    out, stack, i, n, pending = [], [], 0, len(text), None
    while i < n:
        c = text[i]
        if c == '"' or (c == 'r' and re.match(r'r#*"', text[i:i + 4])):
            m = re.match(r'r(#*)"', text[i:]) if c == 'r' else None
            if m:
                end = text.find('"' + m.group(1), i + len(m.group(0)))
                i = (end if end >= 0 else n) + 1 + len(m.group(1))
            else:
                j = i + 1
                while j < n and text[j] != '"':
                    j += 2 if text[j] == '\\' else 1
                i = j + 1
            continue
        if c == "'" and re.match(r"'(\\.|[^\\'])'", text[i:i + 4]):
            i += len(re.match(r"'(\\.|[^\\'])'", text[i:i + 4]).group(0))
            continue
        m = re.match(r'(?:mod\s+(\w+)|impl(?:<[^>{]*>)?\s+([^{;]+?)\s*(?:where[^{]*)?(?=\{)|fn\s+(\w+))',
                     text[i:])
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == '_')):
            if m.group(1):
                pending = ("mod", m.group(1), i)
            elif m.group(2):
                t = m.group(2)
                t = t.split(" for ")[-1] if " for " in t else t
                t = re.sub(r'<.*', '', t).split("::")[-1].strip()
                pending = ("type", re.sub(r'\W', '', t), i)
            else:
                pending = ("fn", m.group(3), i)
            i += len(m.group(0))
            continue
        if c == ';' and pending and pending[0] in ("fn", "mod"):
            pending = None
        elif c == '{':
            stack.append(pending)
            pending = None
        elif c == '}' and stack:
            opened = stack.pop()
            if opened and opened[0] == "fn":
                scopes = [p for p in stack if p]
                if not any(p[0] == "mod" and p[1] == "tests" for p in scopes):
                    types = [p[1] for p in scopes if p[0] == "type"]
                    key = "::".join([path] + types[-1:] + [opened[1]])
                    out.append((key, text[opened[2]:i + 1]))
        i += 1
    return out

files = {}
for root, dirs, names in os.walk("."):
    dirs[:] = [d for d in dirs if d not in ("target", "node_modules", ".git")]
    for name in names:
        path = os.path.join(root, name)[2:]
        if name.endswith(".rs") and not skipped("/" + path):
            files[path] = open(path, encoding="utf-8").read()

STATUS_CHANGESETS = {t: changesets(t, files) for t in TABLES}

def table_re(names):
    alt = "|".join(sorted(map(re.escape, names)))
    return alt

def rules_for(text):
    t = {name: aliases(text, name) for name in TABLES}
    status = table_re(aliases(text, "status"))
    def insert(table):
        return re.compile(
            r'insert_into\(\s*(?:\w+::)*(?:%s)(?:::table)?\s*\)|INSERT\s+INTO\s+(?:"?\w+"?\.)?"?%s"?(?![\w])'
            % (table_re(t[table]), table), re.I)
    def update(table):
        return re.compile(
            r'update\(\s*&?(?:\w+::)*(?:%s)\b(?![_\w])|UPDATE\s+(?:ONLY\s+)?(?:"?\w+"?\.)?"?%s"?\s'
            % (table_re(t[table]), table), re.I)
    names_status = re.compile(r'\b(?:%s)\b' % status, re.I)
    sets_ident = re.compile(r'\.set\(\s*&?[a-z_]\w*\s*\)')
    def touches_status(table, key, fn):
        if names_status.search(fn):
            return True
        return bool(sets_ident.search(fn)) and (
            any(re.search(r'\b%s\b' % name, key + fn) for name in STATUS_CHANGESETS[table])
            or re.search(r':\s*&?\s*(?:\w+::)*\w*(?:Patch|Changeset)\b', fn))
    return {
        "entitlement-insert": lambda key, fn: bool(insert("entitlement").search(fn)),
    }

RULE_NAMES = ("entitlement-insert",)

allowed, failed = {}, False
for raw in open(allowlist, encoding="utf-8"):
    if not raw.strip() or raw.lstrip().startswith("#"):
        continue
    parts = [p.strip() for p in raw.split("|")]
    if len(parts) != 3 or not parts[2] or parts[1] not in RULE_NAMES:
        print(f"❌ {allowlist}: malformed entry: {raw.strip()}", file=sys.stderr)
        failed = True
        continue
    allowed.setdefault(parts[0], set()).add(parts[1])

found = set()
for path in sorted(files):
    rules = rules_for(re.sub(r'(?m)^\s*//.*$', '', files[path]))
    for key, fn in functions(path, files[path]):
        for rule, hit in rules.items():
            if hit(key, fn):
                found.add((key, rule))
                if rule not in allowed.get(key, set()):
                    print(f"❌ {key}: {rule} outside the allowlisted write functions",
                          file=sys.stderr)
                    failed = True

for key, rules in allowed.items():
    for rule in rules - {r for k, r in found if k == key}:
        print(f"❌ {allowlist}: {key} | {rule} no longer writes: remove it", file=sys.stderr)
        failed = True

if failed:
    sys.exit(1)
print(f"✅ {len(found)} write site(s), all allowlisted")
PY
