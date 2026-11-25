"""Rendering's options as Python checks them."""

import pathlib
import unittest
from typing import Never, cast

import starlightpdf

BLANK = (
    pathlib.Path(__file__).resolve().parents[3]
    / "tests"
    / "corpus"
    / "render"
    / "output-blank.pdf"
)


class Render(unittest.TestCase):
    """Rendering's options."""

    def test_hidden_layers_not_a_sequence_of_layers(self) -> None:
        """A string is no list of layers, as in JavaScript: `TypeError`."""
        source = starlightpdf.Source.from_bytes(bytes=BLANK.read_bytes())
        page = starlightpdf.Document.open(source).page(index=0)
        with self.assertRaises(TypeError):
            # Typed as the stub allows, so that the wrong value reaches the binding.
            layers = cast("list[Never]", "x")
            page.render(starlightpdf.Sink.in_memory(), hidden_layers=layers)

    def test_hidden_layers_not_empty(self) -> None:
        """Any layer to hide fails `NotImplemented`, as no layer can be made yet."""
        source = starlightpdf.Source.from_bytes(bytes=BLANK.read_bytes())
        page = starlightpdf.Document.open(source).page(index=0)
        # Typed as the stub allows, so that a non-empty list reaches the binding.
        layers = cast("list[Never]", [None])
        with self.assertRaises(starlightpdf.PdfError) as caught:
            page.render(starlightpdf.Sink.in_memory(), hidden_layers=layers)
        self.assertEqual(caught.exception.code, "NotImplemented")
