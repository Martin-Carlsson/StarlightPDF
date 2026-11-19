"""Checks which areas ci/areas.sh selects for changed paths."""

import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


def areas(*paths: str) -> set[str]:
    """Return the areas ci/areas.sh selects for `paths`."""
    stdin = "".join(f"{path}\n" for path in paths)
    out = subprocess.run(
        ["sh", "ci/areas.sh"],
        cwd=ROOT,
        input=stdin,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return set(out.split())


class AreasTest(unittest.TestCase):
    """Areas selected for changed paths."""

    def accepted(self) -> set[str]:
        """Return the areas ci/check.sh accepts."""
        run = subprocess.run(
            ["sh", "ci/check.sh"], cwd=ROOT, capture_output=True, text=True, check=False
        )
        self.assertEqual(2, run.returncode)
        return set(run.stderr.strip().removeprefix("usage: sh ci/check.sh ").split("|"))

    def test_includes(self) -> None:
        """Each path selects at least its areas."""
        cases = {
            "Cargo.lock": {"advisories", "core", "dependencies", "licenses", "python"},
            "README.md": {"dependencies", "licenses"},
            "Cargo.toml": {"dependencies", "licenses"},
            "core/Cargo.toml": {"dependencies", "licenses"},
            "bindings/python/Cargo.toml": {"dependencies", "licenses"},
            "ci/dependencies.sh": {"dependencies", "licenses"},
            "ci/workflows.sh": {"licenses", "workflows"},
            "core/src/lib.rs": {"core", "fuzz", "python", "licenses"},
            "tests/corpus/header/empty.pdf": {"core", "fuzz", "python", "licenses"},
            "fuzz/fuzz_targets/page_count.rs": {"fuzz", "licenses"},
            "bindings/python/src/lib.rs": {"python", "licenses"},
            "ci/licenses.sh": {"licenses", "ruff", "shell"},
            "tests/ci/test_areas.py": {"licenses", "ruff"},
            "bindings/python/starlightpdf/__init__.pyi": {"licenses", "python", "ruff"},
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertLessEqual(expected, areas(path))

    def test_all(self) -> None:
        """CI's own files select every accepted area."""
        for path in ("ci/check.sh", ".github/workflows/ci.yml"):
            with self.subTest(path=path):
                self.assertEqual(self.accepted(), areas(path))

    def test_every_area_reachable_and_accepted(self) -> None:
        """Tracked files select exactly the accepted areas."""
        paths = subprocess.run(
            ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout.split("\n")
        own = {"", "ci/check.sh", ".github/workflows/ci.yml"}
        self.assertEqual(self.accepted(), areas(*(p for p in paths if p not in own)))

    def test_licenses_alone(self) -> None:
        """Prose selects only licenses."""
        self.assertEqual({"licenses"}, areas("GLOSSARY.md"))

    def test_python_without_core(self) -> None:
        """The binding alone skips core and fuzz."""
        self.assertEqual(set(), {"core", "fuzz"} & areas("bindings/python/src/lib.rs"))

    def test_no_paths(self) -> None:
        """No paths select nothing."""
        self.assertEqual(set(), areas())


if __name__ == "__main__":
    unittest.main()
