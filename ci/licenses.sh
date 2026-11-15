#!/bin/sh
# Fails when a file lacks a declared copyright or license, a license is off the allow list, or LICENSES/ differs from the licenses declared.
# Reads only REUSE.toml (no in-file tags or .license files); replaces `reuse lint`, which is partly GPL.
set -eu
cd "$(dirname "$0")/.."
exec "${PY:?run through ci/check.sh}" -I - <<'PY'
"""REUSE check; see the comments above."""

import pathlib
import re
import subprocess
import sys
import tomllib

KEYS = {"path", "precedence", "SPDX-FileCopyrightText", "SPDX-License-Identifier"}
ALLOWED = {"Apache-2.0", "BSD-3-Clause", "MPL-2.0", "LLVM-exception"}
errors = []


def fail(message: str) -> None:
    """Record an error."""
    errors.append(message)


def glob(pattern: str) -> re.Pattern[str]:
    """Compile a REUSE.toml path glob."""
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


def strings(table: dict[str, object], key: str) -> list[str]:
    """Return `key` of `table` as a list of non-empty strings."""
    value = table.get(key, [])
    value = [value] if isinstance(value, str) else value
    if not isinstance(value, list) or not all(
        isinstance(v, str) and v.strip() for v in value
    ):
        fail(f"REUSE.toml: {key} must be non-empty strings")
        return []
    return value


try:
    with pathlib.Path("REUSE.toml").open("rb") as f:
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
    annotations.append(
        (
            [glob(p) for p in strings(a, "path")],
            strings(a, "SPDX-FileCopyrightText"),
            strings(a, "SPDX-License-Identifier"),
        )
    )

listed = (
    subprocess.run(
        ["git", "ls-files", "-z", "-co", "--exclude-standard"],
        check=True,
        capture_output=True,
    )
    .stdout.decode()
    .split("\0")
)
paths = sorted({p for p in listed if p})


def ignored(path: str) -> bool:
    """Return whether REUSE ignores `path`."""
    name = path.rsplit("/", 1)[-1]
    return (
        path.startswith(("LICENSES/", ".reuse/"))
        or path == "REUSE.toml"
        or re.fullmatch(r"(LICEN[CS]E|COPYING)([-.].*)?", name) is not None
        or name.endswith(".license")
        or re.search(r"\.spdx(\.|$)", name) is not None
        or pathlib.Path(path).is_symlink()
        or not pathlib.Path(path).exists()
        or pathlib.Path(path).stat().st_size == 0
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
            license_id = token.removesuffix("+")
            if license_id not in ALLOWED:
                fail(f"{path}: license {license_id} not allowed")
            used.add(license_id)

present = {p for p in paths if p.startswith("LICENSES/")}
wanted = {f"LICENSES/{i}.txt" for i in used}
for p in sorted(wanted - present):
    fail(f"{p}: missing")
for p in sorted(wanted & present):
    if not pathlib.Path(p).read_bytes().strip():
        fail(f"{p}: lacks the text")
for p in sorted(present - wanted):
    fail(f"{p}: not a declared license text")

if errors:
    sys.exit("\n".join(errors))
PY
