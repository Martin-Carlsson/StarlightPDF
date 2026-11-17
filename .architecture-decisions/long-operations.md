# Long operations

The core is synchronous; a callback reports progress and cancels cooperatively; bindings add async in their language's model. PDF work is computing, not waiting, and async in the core would need a runtime dependency.

In the browser the core runs in a worker, where synchronous file reads exist, and the main-thread API returns promises. Lazy reading needs synchronous reads, which browsers allow only in a worker.

Signing runs in steps, prepare → the caller supplies the signature bytes → finalize, so remote and hardware signers run in the binding's async model. Remote signing is waiting, not computing, and the core never awaits.
