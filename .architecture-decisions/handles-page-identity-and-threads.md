# Handles, page identity and threads

`Document`, `Page` and the rest are reference-counted handles, in Rust too; a handle holds a reference, never decoded data, which lives in the cache. Python, JS and C cannot express Rust borrows, so with borrows the bindings would be a second API the conformance suite does not describe; handles also make use-after-close impossible.

A `Page` follows the page, not its position; after the page is deleted the handle fails with a stale-handle code. Inserting a page before it must not make it point at a different page.

`close()` releases the source at once, except backing an import still holds, and invalidates dependent handles; later calls return a code; garbage collection is only a fallback. Garbage-collected languages give no other deterministic release.

Handles are thread-safe: many readers or one writer per document, independent documents in parallel within the shared budget; a progress callback must not re-enter and change the active document. Java, C# and Go share objects between threads and free them from another, so handles must survive that anyway, and parallel rendering and extraction then come free.
