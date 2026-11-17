# Saving a document

`save(target)` is a streamed full rewrite. The `incremental` option appends the changes (ISO 32000-2:2020, 7.5.6); when the target is the opened file, unchanged since open, and the caller has explicitly allowed appending in place, only the changes are written. A requested incremental save never falls back to a rewrite; it fails with the reason. A rewrite is the right default because an incremental save keeps the old content in the file, which is wrong for removing anything; incremental stays available because signatures need it and it is the cheap path for large files.

Saving over the opened file writes beside it, then renames; appending in place has no atomic guarantee and the API says so. A failed save to a path leaves the previous file intact; to an arbitrary sink it may leave partial output. Users expect a saved path to hold either the old file or the new one, never half of each.

Whole-file work (compress, linearize, encrypt, remove unused objects) is a save option, not an edit; linearizing and signing need a seekable target or temporary storage and fail with a code on a bare append-only sink. As an edit it would put the whole file in the overlay, and linearizing and signing both patch bytes already written.

Redaction and sanitizing refuse an incremental save. Rewriting signed input needs an explicit option allowing signatures to break, and what a save does to each signature is reported separately (ISO 32000-2:2020, 12.8). Removed content must leave the file, and a user asking to preserve signatures must get a refusal, not a broken signature.

Saving never changes the logical document, so held pages stay valid; after a successful save to the opened file the session's source becomes the saved revision and the overlay is released; saving elsewhere leaves the session on its source. Hundreds of edit-and-save rounds must not retain history, and saving a copy must not silently switch the session to another file.
