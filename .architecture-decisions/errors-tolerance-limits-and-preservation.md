# Errors, tolerance, limits and preservation

C returns numbers, so error codes must exist anyway. One `Error`: a stable code, a message and, where known, the operation, byte offset, object number and underlying I/O cause; codes are non-exhaustive, so bindings accept unknown ones; a binding may map codes to exception subclasses. One error maps to one exception type per language, and adding a code breaks nobody.

Many real PDFs break the standard, and viewers open them anyway. Tolerant by default: `open` accepts what viewers accept, rebuilds a broken cross-reference table, and records each deviation in `doc.warnings`; the `strict` option rejects the first deviation; `validate` is a separate operation that reads everything, and a successful open is not a PDF/A or PDF/UA result. Users judge a PDF library by whether it opens what Adobe Reader opens, and strict mode serves validation.

Reading is lazy, so `open` sees little of the file. Errors may surface late: any call after `open` can find damage. It is the price of the rule that library memory must not grow with file size, and permanent.

An edit to one part of a PDF can break another that refers to it. An edit preserves unrelated content, forms, annotations and metadata; an operation that cannot preserve a structure returns a code or applies a loss policy the caller chose, never a silent drop. Rotating a page must never cost a user their form fields.

PDFs carry actions and JavaScript. Actions and scripts can be read, never executed. Untrusted input stays data.
