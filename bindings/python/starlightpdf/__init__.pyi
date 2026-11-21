import os
from collections.abc import Callable
from typing import Self, final

__all__ = [
    "Deviation",
    "Deviations",
    "Document",
    "Documents",
    "Encryption",
    "InfoDictionary",
    "Layer",
    "ObjectId",
    "Page",
    "PdfError",
    "Rect",
    "ResourcePolicy",
    "SaveReport",
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
    def from_append(append: Callable[[bytes], object]) -> Sink: ...
    @staticmethod
    def from_file(file: str | os.PathLike[str]) -> Sink: ...
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
class Layer: ...

@final
class Page:
    def boundary(self, *, kind: str = "Crop") -> Rect: ...
    def draw_image(
        self,
        image: Source,
        mask: Source | None = None,
        layer: Layer | None = None,
        *,
        x: float = 0.0,
        y: float = 0.0,
        width: float | None = None,
        height: float | None = None,
        ignore_orientation: bool = False,
        format: str | None = None,  # noqa: A002 runtime keyword name
        opacity: float | None = None,
    ) -> None: ...
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
class InfoDictionary:
    @property
    def author(self) -> str: ...
    @property
    def created(self) -> str: ...
    @property
    def creator(self) -> str: ...
    @property
    def keywords(self) -> str: ...
    @property
    def modified(self) -> str: ...
    @property
    def producer(self) -> str: ...
    @property
    def subject(self) -> str: ...
    @property
    def title(self) -> str: ...

@final
class SaveReport:
    @property
    def broken_signatures(self) -> list[str]: ...
    @property
    def length(self) -> int: ...

@final
class Documents:
    def __iter__(self) -> Self: ...
    def __next__(self) -> Document: ...

@final
class Document:
    @staticmethod
    def create(
        policy: ResourcePolicy | None = None, *, version: tuple[int, int] | None = None
    ) -> Document: ...
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
    @property
    def info_dictionary(self) -> InfoDictionary: ...
    def set_info_dictionary(
        self,
        *,
        author: str | None = None,
        created: str | None = None,
        creator: str | None = None,
        keywords: str | None = None,
        modified: str | None = None,
        producer: str | None = None,
        subject: str | None = None,
        title: str | None = None,
    ) -> None: ...
    def add_layer(self, *, name: str = "", visible: bool = True) -> Layer: ...
    def insert_blank_page(
        self,
        *,
        width: float | None = None,
        height: float | None = None,
        index: int | None = None,
    ) -> Page: ...
    def page(self, *, index: int) -> Page: ...
    def save(
        self,
        sink: Sink,
        progress: Callable[[int, int], bool] | None = None,
        *,
        allow_loss: bool = False,
        break_signatures: bool = False,
        cipher: str | None = None,
        color_space: str | None = None,
        image_quality: int | None = None,
        image_resolution: int | None = None,
        mode: str = "Rewrite",
        optimizations: list[str] = ...,
        owner_password: str = "",
        recipients: bytes = b"",
        restrictions: list[str] = ...,
        standards: list[str] = ...,
        user_password: str = "",
        version: tuple[int, int] | None = None,
    ) -> SaveReport: ...
    def split(
        self,
        *,
        by: str = "Pages",
        count: int = 0,
        starts: list[int] = ...,
        allow_loss: bool = False,
    ) -> Documents: ...
    def stream_data(
        self, sink: Sink, object_id: ObjectId, /, *, decode: bool = False
    ) -> None: ...

def header_version(data: bytes) -> tuple[int, int]: ...
def library_version() -> str: ...
