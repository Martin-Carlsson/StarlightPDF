#!/bin/sh
# Fails when a file lacks a declared copyright or license, or LICENSES/ differs from the licenses declared.
# Reads only REUSE.toml (no in-file tags or .license files); replaces `reuse lint`, which is partly GPL.
set -eu
cd "$(dirname "$0")/.."
exec "${PY:?run through ci/check.sh}" -I - <<'PY'
import os, re, subprocess, sys, tomllib

KEYS = {"path", "precedence", "SPDX-FileCopyrightText", "SPDX-License-Identifier"}
errors = []


def fail(message):
    errors.append(message)


def glob(pattern):
    out, i = [], 0
    while i < len(pattern):
        if pattern[i] == "\\" and i + 1 < len(pattern):
            out.append(re.escape(pattern[i + 1]))
            i += 2
        elif pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("".join(out))


def strings(table, key):
    value = table.get(key, [])
    value = [value] if isinstance(value, str) else value
    if not isinstance(value, list) or not all(isinstance(v, str) and v.strip() for v in value):
        fail(f"REUSE.toml: {key} must be non-empty strings")
        return []
    return value


try:
    with open("REUSE.toml", "rb") as f:
        config = tomllib.load(f)
except (OSError, tomllib.TOMLDecodeError) as e:
    sys.exit(f"REUSE.toml: {e}")
if config.get("version") != 1:
    fail("REUSE.toml: version must be 1")
annotations = []
for a in config.get("annotations", []):
    if not set(a) <= KEYS:
        fail(f"REUSE.toml: unsupported keys {sorted(set(a) - KEYS)}")
    if not a.get("path"):
        fail("REUSE.toml: annotation lacks path")
    if a.get("precedence", "closest") not in ("closest", "override"):
        fail("REUSE.toml: precedence must be closest or override")
    annotations.append((
        [glob(p) for p in strings(a, "path")],
        strings(a, "SPDX-FileCopyrightText"),
        strings(a, "SPDX-License-Identifier"),
    ))

listed = subprocess.run(
    ["git", "ls-files", "-z", "-co", "--exclude-standard"],
    check=True, capture_output=True,
).stdout.decode().split("\0")
paths = sorted({p for p in listed if p})


def ignored(path):
    name = path.rsplit("/", 1)[-1]
    return (
        path.startswith(("LICENSES/", ".reuse/"))
        or path == "REUSE.toml"
        or re.fullmatch(r"(LICEN[CS]E|COPYING)([-.].*)?", name)
        or name.endswith(".license")
        or re.search(r"\.spdx(\.|$)", name)
        or os.path.islink(path)
        or not os.path.exists(path)
        or os.path.getsize(path) == 0
    )


used = set()
for path in paths:
    if ignored(path):
        continue
    last = None
    for patterns, copyrights, licenses in annotations:
        if any(p.fullmatch(path) for p in patterns):
            last = (copyrights, licenses)
    if last is None or not last[0]:
        fail(f"{path}: no copyright notice")
    if last is None or not last[1]:
        fail(f"{path}: no license")
    for expression in last[1] if last else []:
        for token in re.findall(r"[^\s()]+", expression):
            if token.upper() in ("AND", "OR", "WITH"):
                continue
            token = token.removesuffix("+")
            if re.fullmatch(r"[A-Za-z0-9.-]+", token):
                used.add(token)
            else:
                fail(f"{path}: invalid license identifier {token}")

present = {p for p in paths if p.startswith("LICENSES/")}
wanted = {f"LICENSES/{i}.txt" for i in used}
for p in sorted(wanted - present):
    fail(f"{p}: missing")
for p in sorted(wanted & present):
    if os.path.getsize(p) == 0:
        fail(f"{p}: lacks the text")
for p in sorted(present - wanted):
    fail(f"{p}: not a declared license text")

if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
PY
