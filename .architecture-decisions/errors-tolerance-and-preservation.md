# Errors, tolerance and preservation

C returns numbers, so error codes must exist anyway, and Rust callers expect errors to implement `core::error::Error`. One `PdfError`: a stable code with a fixed number, a message and, where known, the operation, byte offset, object number and underlying I/O cause; codes are non-exhaustive, so bindings accept unknown ones; a binding may map codes to exception subclasses; it implements `core::error::Error` and so is not named `Error`. One error maps to one exception type per language, adding a code breaks nobody, `?` carries it into `Box<dyn Error>`, and clippy's denied `error_impl_error` rejects a type named `Error` that implements the trait.

Many real PDFs break the standard, and viewers open them anyway. Tolerant by default: `open` accepts what viewers accept, rebuilds a broken cross-reference table, and records each deviation in `doc.warnings` up to the count the resource policy sets, then only counts the rest; the list counts against the policy's budget; the `strict` option rejects the first deviation; `validate` is a separate operation that reads everything and returns every deviation as a cursor, and a successful open is not a PDF/A or PDF/UA result. Users judge a PDF library by whether it opens what Adobe Reader opens, strict mode serves validation, and a file with a deviation per object must not grow a list with its size.

Reading is lazy, so `open` sees little of the file. Errors may surface late: any call after `open` can find damage. It is the price of the rule that library memory never grows with the file's bytes, and permanent.

An edit to one part of a PDF can break another that refers to it. An edit preserves unrelated content, forms, annotations and metadata; an operation that cannot preserve a structure returns a code or applies a loss policy the caller chose, never a silent drop. Rotating a page must never cost a user their form fields.

PDFs carry actions and JavaScript. Actions and scripts can be read, never executed. Untrusted input stays data.
