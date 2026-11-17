# One API model in every language

The same types, operations, option names, defaults, units, error codes and results everywhere, defined by the conformance suite; each language keeps its own mechanics: casing, properties, iteration, `with`/`using`, exceptions, stream types, async. Docs, examples and bug reports carry over, yet Python still gets `with doc:` and `for page in doc.pages`.

The public API uses only what every language can bind: handles, plain values, flat option records, sources, sinks and callbacks; one option record per operation (keyword arguments in Python, a versioned struct in C). A generic or borrowed type needs a workaround in every binding, and adding an option breaks no caller.

Pages index like the language's own lists: 0-based in most, 1-based in R, Julia, Lua and the CLI; ranges are half-open in code and inclusive in the CLI; the conformance suite is 0-based and a 1-based adapter shifts; page labels are separate from positions. Pages should behave like the language's lists, and people type the numbers a viewer shows.

Relative operations end in `_by` (`rotate_by`), absolute ones start with `set_` (`set_rotation`); an operation returns a light result or report, never a serialized document; a large result comes as an iterator or in chunks, with a collect convenience. Reading all text or all rendered pages must never be an unavoidable step.

The C API has explicit release functions, buffer lengths, allocator ownership, versioned option structs and error retrieval; no binding adds a whole-buffer copy behind a convenient call. Most listed languages reach the core through C, so its contract is the one most users depend on.
