# Object layer and document layer

The object layer reads and replaces raw objects and streams (get, change, put back); the document layer (pages, rotate, merge) uses only the public object layer. The README promises structure access, users can do what the document layer cannot yet, building on it proves it is enough, and get-change-put binds cheaply where live nested objects would not.

The object layer exposes deliberate handles and readers, never internal caches or storage; its edits join change tracking and invalidate derived data. An object-layer edit that bypasses the overlay would break saving.

Canvas (draw this text at x, y) and layout (place these paragraphs and break pages) are separate concepts; layout builds on canvas. The README lists both, and a layout engine has to draw through something.
