import os
from typing import Self, final

__all__ = [
    "Deviation",
    "Deviations",
    "Document",
    "Encryption",
    "ObjectId",
    "Page",
    "PdfError",
    "Rect",
    "ResourcePolicy",
    "Sink",
    "Source",
    "header_version",
    "library_version",
]

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
class Sink:
    @staticmethod
    def in_memory() -> Sink: ...
    def take_bytes(self) -> bytes: ...

@final
class ResourcePolicy:
    def __new__(
        cls,
        *,
        decoded_limit: int | None = None,
        object_limit: int | None = None,
        warning_limit: int | None = None,
    ) -> Self: ...

@final
class ObjectId:
    def __new__(cls, number: int, generation: int) -> Self: ...
    @property
    def number(self) -> int: ...
    @property
    def generation(self) -> int: ...

@final
class Rect:
    @property
    def x(self) -> float: ...
    @property
    def y(self) -> float: ...
    @property
    def width(self) -> float: ...
    @property
    def height(self) -> float: ...

@final
class Page:
    def boundary(self, *, kind: str = "Crop") -> Rect: ...
    @property
    def rotation(self) -> int: ...

@final
class Encryption:
    @property
    def cipher(self) -> str: ...
    @property
    def certificate(self) -> bool: ...
    @property
    def restrictions(self) -> list[str]: ...

@final
class Deviation:
    @property
    def clause(self) -> str: ...
    @property
    def message(self) -> str: ...
    @property
    def object_number(self) -> int | None: ...
    @property
    def offset(self) -> int | None: ...
    @property
    def page(self) -> int | None: ...

@final
class Deviations:
    def __iter__(self) -> Self: ...
    def __next__(self) -> Deviation: ...

@final
class Document:
    @staticmethod
    def open(
        source: Source,
        policy: ResourcePolicy | None = None,
        *,
        strict: bool = False,
        password: str = "",
    ) -> Document: ...
    @property
    def page_count(self) -> int: ...
    @property
    def catalog(self) -> ObjectId: ...
    @property
    def encryption(self) -> Encryption: ...
    @property
    def warnings(self) -> Deviations: ...
    @property
    def dropped_warnings(self) -> int: ...
    def object(self, object_id: ObjectId, /) -> str: ...
    def page(self, *, index: int) -> Page: ...
    def stream_data(
        self, sink: Sink, object_id: ObjectId, /, *, decode: bool = False
    ) -> None: ...

def header_version(data: bytes) -> tuple[int, int]: ...
def library_version() -> str: ...
