"""Checks the version the library reports."""

import importlib.metadata
import unittest

import starlightpdf


class Version(unittest.TestCase):
    """The library's version."""

    def test_library_version(self) -> None:
        """The installed distribution's version."""
        self.assertEqual(
            starlightpdf.library_version(), importlib.metadata.version("starlightpdf")
        )
