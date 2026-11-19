"""Runs `sh ci/dependencies.sh` on copies of the repository.

A `cargo` stub declares each README row's license.
"""

import os
import pathlib
import re
import shlex
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from collections.abc import Callable

ROOT = pathlib.Path(__file__).resolve().parents[2]
ENV = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}


def rows() -> dict[str, str]:
    """Return the license of each crates.io row in the README's Dependencies section."""
    section = (ROOT / "README.md").read_text().split("\n### Dependencies\n", 1)[1]
    return dict(
        re.findall(
            r"^\| (\S+) \| ([^|]+) \| [^|]*\(https://crates\.io/crates/",
            section,
            re.MULTILINE,
        )
    )


ROWS = rows()


def package() -> tuple[str, str]:
    """Return the name and Cargo.lock lines of a crates.io package with a README row."""
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    p = next(p for p in lock if p["name"] in ROWS and "source" in p)
    return p["name"], "".join(
        f'{k} = "{p[k]}"\n' for k in ("name", "version", "source")
    )


NAME, LINES = package()


def edit(path: pathlib.Path, old: str, new: str) -> None:
    """Replace the first `old` in `path` with `new`."""
    text = path.read_text()
    if old not in text:
        msg = f"{old!r} not in {path}"
        raise AssertionError(msg)
    path.write_text(text.replace(old, new, 1))


def drop_row(root: pathlib.Path) -> None:
    """Drop NAME's row from the README."""
    lines = (root / "README.md").read_text().splitlines(keepends=True)
    (root / "README.md").write_text(
        "".join(line for line in lines if not line.startswith(f"| {NAME} |"))
    )


CASES = {
    "package from git": lambda r: edit(
        r / "Cargo.lock",
        LINES,
        LINES.rsplit("source", 1)[0] + 'source = "git+https://example.invalid/x#0"\n',
    ),
    "package from a path outside the workspace": lambda r: edit(
        r / "Cargo.lock", LINES, LINES.rsplit("source", 1)[0]
    ),
    "package without a row": drop_row,
    "row license differs from the declared one": lambda r: edit(
        r / "README.md", f"| {NAME} | ", f"| {NAME} | MIT AND "
    ),
    "tool link version differs from ci/setup.sh": lambda r: edit(
        r / "README.md",
        "crates.io/crates/cargo-deny/",
        "crates.io/crates/cargo-deny/0.",
    ),
}


def cargo(path: pathlib.Path) -> None:
    """Write a `cargo info NAME@VERSION` printing the license of NAME's README row."""
    lines = (
        shlex.quote(name)
        + ") echo "
        + shlex.quote("license: " + re.sub(r"\[([^]]*)\]\([^)]*\)", r"\1", lic))
        for name, lic in ROWS.items()
    )
    cases = "".join(f"  {line} ;;\n" for line in lines)
    path.write_text(f'#!/bin/sh\ncase "${{2%@*}}" in\n{cases}  *) exit 1 ;;\nesac\n')
    path.chmod(0o755)


def check(mutate: Callable[[pathlib.Path], object] | None = None) -> int:
    """Return the exit code of ci/dependencies.sh on a copy changed by `mutate`."""
    files = (
        subprocess.run(
            ["git", "ls-files", "-z", "-co", "--exclude-standard"],
            cwd=ROOT,
            env=ENV,
            check=True,
            capture_output=True,
        )
        .stdout.decode()
        .split("\0")
    )
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp) / "repo"
        for name in filter(None, files):
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, root / name, follow_symlinks=False)
        cargo(pathlib.Path(tmp) / "cargo")
        if mutate:
            mutate(root)
        path = f"{tmp}{os.pathsep}{ENV['PATH']}"
        return subprocess.run(
            ["sh", "ci/dependencies.sh"],
            cwd=root,
            env={**ENV, "PATH": path},
            capture_output=True,
            check=False,
        ).returncode


class Dependencies(unittest.TestCase):
    """The repository passes and each mistake fails."""

    def test_repository_passes(self) -> None:
        """The repository passes."""
        self.assertEqual(check(), 0)

    def test_each_mistake_fails(self) -> None:
        """Each mistake fails."""
        for name, mutate in CASES.items():
            with self.subTest(name):
                self.assertNotEqual(check(mutate), 0)


if __name__ == "__main__":
    unittest.main()
