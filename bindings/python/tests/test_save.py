"""Saving."""

import pathlib
import tempfile
import unittest

import starlightpdf

NESTED = (
    pathlib.Path(__file__).resolve().parents[3]
    / "tests"
    / "corpus"
    / "page-count"
    / "nested-tree.pdf"
)


class Save(unittest.TestCase):
    """Saving."""

    def test_sinks_receive_the_same_bytes(self) -> None:
        """Memory, append and file sinks get the same bytes of one document."""
        document = starlightpdf.Document.open(
            starlightpdf.Source.from_bytes(bytes=NESTED.read_bytes())
        )
        memory = starlightpdf.Sink.in_memory()
        document.save(memory)
        expected = memory.take_bytes()
        chunks: list[bytes] = []
        document.save(starlightpdf.Sink.from_append(chunks.append))
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "saved.pdf"
            document.save(starlightpdf.Sink.from_file(path))
            written = path.read_bytes()
        self.assertTrue(expected.startswith(b"%PDF-"))
        self.assertEqual((b"".join(chunks), written), (expected, expected))
