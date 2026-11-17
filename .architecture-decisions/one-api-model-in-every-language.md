# One API model in every language

Many languages bind one core, and each binding could drift into an API of its own. The same types, operations, option names, defaults, units, error codes and results everywhere, defined by the conformance suite; each language keeps its own mechanics: casing, properties, iteration, `with`/`using`, exceptions, stream types, async. Docs, examples and bug reports carry over, yet Python still gets `with doc:` and `for page in doc.pages`.

Generics, borrows and long argument lists do not cross every language boundary. The public API uses only what every language can bind: handles, plain values, flat option records, sources, sinks and callbacks; one option record per operation (keyword arguments in Python, a versioned struct in C). A generic or borrowed type needs a workaround in every binding, and adding an option breaks no caller.

Languages disagree on where a list starts and how a range ends. Pages index and range like the language's own lists: 0-based in most, 1-based in R, Julia, Lua and the CLI; a range is the language's own, half-open where that is its convention, inclusive in the 1-based languages and the CLI; the conformance suite is 0-based and half-open, and an adapter converts; page labels are separate from positions. Pages should behave like the language's lists, a half-open range from 1 matches no convention, and people type the numbers a viewer shows.

A name must tell relative from absolute, and some results are as large as the document. Relative operations end in `_by` (`rotate_by`), absolute ones start with `set_` (`set_rotation`); an operation returns a light result or report, never a serialized document; a large result comes as an iterator or in chunks, with a collect convenience. Reading all text or all rendered pages must never be an unavoidable step.

Most listed languages reach the core through C, so its contract is the one most users depend on. The C API has explicit release functions, buffer lengths, allocator ownership, versioned option structs and error retrieval; no binding adds a whole-buffer copy behind a convenient call. Ownership left implicit in C leaks or copies in every language above it.
