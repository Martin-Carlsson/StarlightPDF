"""Checks the installed distribution's license metadata (PEP 639)."""

import importlib.metadata
import pathlib
import unittest

LICENSE = pathlib.Path(__file__).resolve().parents[3] / "LICENSE"


class License(unittest.TestCase):
    def setUp(self):
        self.dist = importlib.metadata.distribution("starlightpdf")

    def test_metadata(self):
        """PEP 639, "Add `License-Expression` field" and "Add `License-File` field"."""
        metadata = self.dist.metadata
        self.assertEqual(metadata["Metadata-Version"], "2.4")
        self.assertEqual(metadata.get_all("License-Expression"), ["Apache-2.0"])
        self.assertEqual(metadata.get_all("License-File"), ["LICENSE"])

    def test_license_file(self):
        """PEP 639, "License files in project formats"."""
        path = f"starlightpdf-{self.dist.version}.dist-info/licenses/LICENSE"
        recorded = [file for file in self.dist.files or [] if file.as_posix() == path]
        self.assertEqual(len(recorded), 1, f"RECORD lacks {path}")
        self.assertEqual(recorded[0].locate().read_bytes(), LICENSE.read_bytes())
