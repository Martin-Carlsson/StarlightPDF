"""Checks what the library reports of itself: its version and capabilities."""

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

    def test_capabilities_list_rendering(self) -> None:
        """The wheel has rendering built in."""
        self.assertIn("Rendering", starlightpdf.capabilities())
