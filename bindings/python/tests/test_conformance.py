"""Runs the conformance suite in tests/conformance/*.tsv through the binding."""

import pathlib
import unittest

import starlightpdf

SUITE_DIR = pathlib.Path(__file__).resolve().parents[3] / "tests" / "conformance"
OPERATIONS = {"greet": starlightpdf.greet}


class SuiteError(Exception):
    """A case failed."""


def run_case(line):
    *head, expected = line.split("\t")
    if not head:
        raise SuiteError(f"malformed line {line!r}")
    operation, *args = head
    if operation not in OPERATIONS:
        raise SuiteError(f"unknown operation {operation!r}")
    actual = OPERATIONS[operation](*args)
    if actual != expected:
        raise SuiteError(f"{operation}: expected {expected!r}, got {actual!r}")


def run(suite):
    cases = [line for line in suite.removesuffix("\n").split("\n") if not line.startswith("#")]
    if not cases:
        raise SuiteError("no cases")
    for case in cases:
        run_case(case)


class Conformance(unittest.TestCase):
    def test_suite_passes(self):
        texts = [path.read_text(encoding="utf-8") for path in sorted(SUITE_DIR.glob("*.tsv"))]
        run("".join(text if text.endswith("\n") else text + "\n" for text in texts))

    def test_unknown_operation_fails(self):
        with self.assertRaises(SuiteError):
            run("nope\tx\ty\n")

    def test_suite_without_cases_fails(self):
        with self.assertRaises(SuiteError):
            run("# operation\targuments\texpected\n")
