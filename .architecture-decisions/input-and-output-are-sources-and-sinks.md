# Input and output are sources and sinks

A stream with a read position serves one reader at a time, and PDF access jumps around the file. Input is a source with two calls, its size and "read these bytes at this offset", thread-safe from the first release; a path, a byte buffer, Rust's `Read + Seek` and each language's stream type are shortcuts that make one. Read-at has no shared position, so threads read in parallel; it fits files, memory, HTTP ranges and browser files; and requiring thread-safety later would break every custom source.

Some input can only be read forward: a pipe, a network response. A forward-only stream is spooled to temporary storage when the resource policy allows it; otherwise `open` fails with a code. "Accepts a stream" must never secretly mean "reads everything into RAM".

Memory mapping is the usual shortcut to lazy reading. The core does not memory-map. Memory mapping needs `unsafe` and a dependency, and crashes the process when another program truncates the file.

A PDF can be written front to back. Output is a sink that only appends; `to_bytes()` and other whole-buffer forms are shortcuts; everything large has a streaming form (`attachment.write_to(sink)`, `add_attachment(source)`). Output then goes to a socket or stdout, and no operation forces a whole result into memory.

Every binding hands bytes across a language boundary, where the easy path is a copy. A binding shares the caller's buffer where its runtime can keep it alive and unchanged, and documents where it copies instead; JavaScript cannot stop the caller changing a `Uint8Array`, so it shares one on the documented condition that it stays unchanged, rather than copy the whole file into memory; the browser binding reads a `File` or `Blob` through a source, never by copying it whole into WebAssembly. A copy at the boundary defeats the rule that library memory never grows with the file's bytes.

The README promises the browser, where files arrive as `File` and `Blob` and synchronous reads exist only in a worker. Large files in the browser are a first-class constraint: the source and sink model is tried there before the Rust traits are frozen. It is the environment most likely to break an I/O design.
