# Handles, page identity and threads

Python, JS and C cannot express Rust borrows. `Document`, `Page` and the rest are reference-counted handles, in Rust too; a handle holds a reference, never decoded data, which lives in the cache. With borrows the bindings would be a second API the conformance suite does not describe; handles also make use-after-close impossible.

Pages move when others are inserted or deleted. A `Page` follows the page, not its position; after the page is deleted the handle fails with a stale-handle code. Inserting a page before it must not make it point at a different page.

Garbage-collected languages free objects at no predictable time. `close()` releases the source at once, except backing an import still holds, and invalidates dependent handles; later calls return a code; garbage collection is only a fallback. Those languages give no other deterministic release.

Java, C# and Go share objects between threads and free them from another. Handles are thread-safe: many readers or one writer per document, and documents in parallel, sharing a budget only where they were opened with the same resource policy; a progress callback must not re-enter and change the active document. Handles must survive that anyway, and parallel rendering and extraction then come free.
