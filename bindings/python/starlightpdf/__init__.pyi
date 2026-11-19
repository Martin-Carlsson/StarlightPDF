import os
from typing import final

__all__ = ["Document", "PdfError", "Source", "header_version", "library_version"]

class PdfError(Exception):
    code: str
    number: int
    message: str
    operation: str
    offset: int | None
    object_number: int | None
    io_cause: OSError | None

@final
class Source:
    @staticmethod
    def from_bytes(bytes: bytes) -> Source: ...  # noqa: A002 runtime keyword name
    @staticmethod
    def from_file(file: str | os.PathLike[str]) -> Source: ...

@final
class Document:
    @staticmethod
    def open(source: Source) -> Document: ...
    @property
    def page_count(self) -> int: ...

def header_version(data: bytes) -> tuple[int, int]: ...
def library_version() -> str: ...
