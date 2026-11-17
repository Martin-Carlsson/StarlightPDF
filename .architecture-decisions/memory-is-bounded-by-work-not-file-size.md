# Memory is bounded by work, not file size

A 1 GB file can cost another 1 GB in three places: reading it in, building its objects in memory, and building the output before writing it. Library memory grows with what is touched and changed, never with the file size: an index of object offsets, a cache with a limit the caller sets, the overlay of changes, and the buffers of the active operation. This rule cannot be added later: an API designed without it returns whole things, and callers come to depend on that.

A resource policy passed at open sets the cache limit, decoded-data limit, nesting depth, object count, concurrency and temporary storage (off, or a directory with a quota); it counts library allocations only, not caller buffers, runtimes or OS caches. A 1 KB stream can decompress to gigabytes, and limits with safe defaults are the only way untrusted input never exhausts memory.

Exceeding a limit fails with a stable error code unless temporary storage is allowed, in which case work spills there and is removed on close or cancel; temporary storage is off by default. Configurable spill trades disk and speed for predictable RAM, and off by default means the library never writes files the caller did not ask for, which the browser could not do anyway.

The memory rule is tested by editing and saving a virtual 1 GB file, generated on the fly, through a source that counts bytes read and allocations; the bound is fixed after the first measurement and no public "1 GB costs X MB" figure is promised. A number invented before measuring is wrong, and the test, not the number, is what keeps the promise.

There is no 1.0 before open → inspect → rotate → save → reopen, then a merge of two file-backed sources, passes that test in Rust, Python, C and the browser. A contract untested in the C and browser bindings is a contract on paper.
