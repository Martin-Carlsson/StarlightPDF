"""Checks which areas ci/areas.sh selects for changed paths."""

import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIVE = {"advisories", "core", "dependencies", "licenses", "python"}


def areas(*paths):
    stdin = "".join(f"{path}\n" for path in paths)
    out = subprocess.run(
        ["sh", "ci/areas.sh"], cwd=ROOT, input=stdin, capture_output=True, text=True, check=True
    ).stdout
    return set(out.split())


class AreasTest(unittest.TestCase):
    def test_includes(self):
        cases = {
            "README.md": {"dependencies", "licenses"},
            "Cargo.toml": {"dependencies", "licenses"},
            "core/Cargo.toml": {"dependencies", "licenses"},
            "bindings/python/Cargo.toml": {"dependencies", "licenses"},
            "ci/dependencies.sh": {"dependencies", "licenses"},
            "core/src/lib.rs": {"core", "python", "licenses"},
            "bindings/python/src/lib.rs": {"python", "licenses"},
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertLessEqual(expected, areas(path))

    def test_all(self):
        for path in ("Cargo.lock", "ci/check.sh", ".github/workflows/ci.yml"):
            with self.subTest(path=path):
                self.assertLessEqual(FIVE, areas(path))

    def test_licenses_alone(self):
        self.assertEqual({"licenses"}, areas("GLOSSARY.md"))

    def test_python_without_core(self):
        self.assertNotIn("core", areas("bindings/python/src/lib.rs"))

    def test_no_paths(self):
        self.assertEqual(set(), areas())


if __name__ == "__main__":
    unittest.main()
