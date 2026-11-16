"""Runs `ci/workflows.sh` on workflow files."""

import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
VALID = """\
name: Check pull requests # y
on:
  pull_request:
env:
  name: Decoy
jobs:
  build:
    name: Build and test
    runs-on: ubuntu-latest
    env:
      name: Decoy
    steps: # x
      - name: Check out
        uses: actions/checkout@v4
      - run: echo hi
        name: Say hi
"""


def mutated(old: str, new: str) -> str:
    """Return VALID with the first `old` replaced by `new`."""
    if old not in VALID:
        msg = f"{old!r} not in VALID"
        raise AssertionError(msg)
    return VALID.replace(old, new, 1)


def check(text: str) -> int:
    """Return the exit code with `text` as `ci.yml` next to a valid file."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        (root / "other.yml").write_text(VALID)
        (root / "ci.yml").write_text(text)
        (root / "workflows.sh").write_bytes((ROOT / "ci" / "workflows.sh").read_bytes())
        return subprocess.run(
            ["sh", "workflows.sh", "ci.yml", "other.yml"],
            cwd=root,
            capture_output=True,
            check=False,
        ).returncode


class Workflows(unittest.TestCase):
    """A named workflow passes and each missing or default name fails."""

    def assert_fails(self, cases: dict[str, str]) -> None:
        """Assert that VALID passes and each case fails."""
        self.assertEqual(check(VALID), 0)
        for case, text in cases.items():
            with self.subTest(case):
                self.assertNotEqual(check(text), 0)

    def test_workflow_name(self) -> None:
        """Workflow syntax, `name`."""
        self.assert_fails(
            {
                "missing": mutated("name: Check pull requests # y\n", ""),
                "file name": mutated("Check pull requests", "ci"),
                "file name with extension": mutated("Check pull requests", "ci.yml"),
            }
        )

    def test_job_name(self) -> None:
        """Workflow syntax, `jobs.<job_id>.name`."""
        self.assert_fails(
            {
                "missing": mutated("    name: Build and test\n", ""),
                "job id": mutated("Build and test", "build"),
                "jobs at 4 spaces": mutated("  build:\n", "    build:\n"),
                "jobs inline": mutated("jobs:\n", "jobs: {}\n"),
            }
        )

    def test_step_name(self) -> None:
        """Workflow syntax, `jobs.<job_id>.steps[*].name`."""
        self.assert_fails(
            {
                "run step": mutated("        name: Say hi\n", ""),
                "bare dash": mutated("      - name: Check out\n", "      -\n"),
                "steps inline": mutated("    steps: # x\n", "    steps: []\n"),
                "steps flush with steps": mutated(
                    "      - name: Check", "    - name: Check"
                ),
                "name only under with": mutated(
                    "      - name: Check out\n        uses: actions/checkout@v4\n",
                    "      - uses: actions/checkout@v4\n"
                    "        with:\n"
                    "          name: x\n",
                ),
            }
        )


if __name__ == "__main__":
    unittest.main()
