"""Runs `sh ci/licenses.sh` on copies of the repository.

REUSE 3.3, "License Files" and "REUSE.toml".
"""

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable

ROOT = pathlib.Path(__file__).resolve().parents[2]
ENV = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
ANNOTATION = (
    '\n[[annotations]]\npath = "README.md"\n'
    'SPDX-FileCopyrightText = "Copyright 2026 Martin Carlsson"\n'
    'SPDX-License-Identifier = "Apache-2.0"\n'
)


def edit(path: pathlib.Path, old: str, new: str) -> None:
    """Replace the first `old` in `path` with `new`."""
    text = path.read_text()
    if old not in text:
        msg = f"{old!r} not in {path}"
        raise AssertionError(msg)
    path.write_text(text.replace(old, new, 1))


def append(path: pathlib.Path, text: str) -> None:
    """Append `text` to `path`."""
    path.write_text(path.read_text() + text)


def disallowed(root: pathlib.Path) -> None:
    """Add a file under a license outside the allow list."""
    (root / "x.txt").write_text("x\n")
    (root / "LICENSES/MIT.txt").write_text("MIT License\n")
    append(
        root / "REUSE.toml",
        ANNOTATION.replace("README.md", "x.txt").replace('"Apache-2.0"', '"MIT"'),
    )


CASES = {
    "license id outside the allow list": disallowed,
    "whitespace-only text": lambda r: (r / "LICENSES/MPL-2.0.txt").write_text(
        " \n\t\n"
    ),
    "no REUSE.toml": lambda r: (r / "REUSE.toml").unlink(),
    "invalid TOML": lambda r: append(r / "REUSE.toml", "[[annotations\n"),
    "version other than 1": lambda r: edit(
        r / "REUSE.toml", "version = 1", "version = 2"
    ),
    "annotation without path": lambda r: append(
        r / "REUSE.toml", ANNOTATION.replace('path = "README.md"\n', "")
    ),
    "annotation with unknown key": lambda r: append(
        r / "REUSE.toml", ANNOTATION + 'license = "Apache-2.0"\n'
    ),
    "undefined precedence": lambda r: edit(
        r / "REUSE.toml", 'precedence = "override"', 'precedence = "nearest"'
    ),
    "file no annotation covers": lambda r: edit(
        r / "REUSE.toml", 'path = "**"', 'path = "*"'
    ),
    "annotation without copyright": lambda r: edit(
        r / "REUSE.toml",
        'SPDX-FileCopyrightText = "Copyright 2026 Martin Carlsson"\n',
        "",
    ),
    "annotation without license": lambda r: edit(
        r / "REUSE.toml", 'SPDX-License-Identifier = "Apache-2.0"\n', ""
    ),
    "license without its text": lambda r: (r / "LICENSES/BSD-3-Clause.txt").unlink(),
    "text no file uses": lambda r: edit(
        r / "REUSE.toml", "Apache-2.0 AND BSD-3-Clause AND", "Apache-2.0 AND"
    ),
    "empty text": lambda r: (r / "LICENSES/MPL-2.0.txt").write_text(""),
}


def check(mutate: Callable[[pathlib.Path], object] | None = None) -> int:
    """Return the exit code of ci/licenses.sh on a copy changed by `mutate`."""
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
        root = pathlib.Path(tmp)
        for name in filter(None, files):
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, root / name, follow_symlinks=False)
        subprocess.run(["git", "init", "-q"], cwd=root, env=ENV, check=True)
        if mutate:
            mutate(root)
        return subprocess.run(
            ["sh", "ci/licenses.sh"],
            cwd=root,
            env={**ENV, "PY": sys.executable},
            capture_output=True,
            check=False,
        ).returncode


class Licenses(unittest.TestCase):
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
