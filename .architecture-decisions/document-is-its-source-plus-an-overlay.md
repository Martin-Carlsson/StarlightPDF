# A document is its source plus an overlay

An object graph costs a multiple of the file in RAM, and a reader copied into a writer copies the whole document on every edit. `Document` and `Page` are objects; a document is its source plus an overlay of changed and new objects; edits go to the overlay, `save` writes them, `revert` drops them, and `close` discards unsaved changes and never writes. It is the only model that meets the memory rule, that library memory must not grow with file size, and the overlay is exactly what an incremental save appends.

An edit succeeds or leaves the document unchanged; there is no implicit undo history; grouped transactions come later, backed by the overlay, not a snapshot. A document-sized snapshot per edit breaks the memory rule.

Large data (an image, font or attachment) is a reference to where its bytes are: a range of the source, a file the caller supplied, or another open document. An import captures the source as it is at that moment and keeps its backing alive, so closing the source document does not break the import and later edits to the source do not change it. Merging two 1 GB files then copies nothing, and the result cannot change under the user's feet.

Free functions stay for inspection that needs no parse, `header_version(bytes)` among them; `doc.version` is the effective version: the header (ISO 32000-2:2020, 7.5.2) unless the catalog's `Version` entry is later (7.7.2). The header version and the effective version are different concepts, and folding one into the other hides that.
