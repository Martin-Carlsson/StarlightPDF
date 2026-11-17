# Object layer and document layer

The README promises access to a PDF's structure, and the document layer will always lag behind what some user needs. The object layer reads and replaces raw objects and streams (get, change, put back); the document layer (pages, rotate, merge) uses only the public object layer. Users can do what the document layer cannot yet, building on it proves it is enough, and get-change-put binds cheaply where live nested objects would not.

Saving writes what change tracking recorded. The object layer exposes deliberate handles and readers, never internal caches or storage; its edits join change tracking and invalidate derived data. An object-layer edit that bypasses the overlay would break saving.
