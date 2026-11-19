"""Checks the installed distribution's license metadata (PEP 639)."""

import importlib.metadata
import pathlib
import unittest
from typing import cast

LICENSE = pathlib.Path(__file__).resolve().parents[3] / "LICENSE"


class License(unittest.TestCase):
    """The installed distribution's license metadata."""

    def test_metadata(self) -> None:
        """PEP 639, "Add `License-Expression` field" and "Add `License-File` field"."""
        metadata = importlib.metadata.metadata("starlightpdf")
        self.assertEqual(metadata["Metadata-Version"], "2.4")
        for name, expected in (
            ("License-Expression", ["Apache-2.0"]),
            ("License-File", ["LICENSE"]),
        ):
            self.assertEqual(cast("list[str] | None", metadata.get_all(name)), expected)

    def test_license_file(self) -> None:
        """PEP 639, "License files in project formats"."""
        dist = importlib.metadata.distribution("starlightpdf")
        path = f"starlightpdf-{dist.version}.dist-info/licenses/LICENSE"
        recorded = [file for file in dist.files or [] if file.as_posix() == path]
        self.assertEqual(len(recorded), 1, f"RECORD lacks {path}")
        self.assertEqual(recorded[0].read_binary(), LICENSE.read_bytes())
