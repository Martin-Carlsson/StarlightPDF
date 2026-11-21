# Long operations

PDF work is computing, not waiting. The core is synchronous; a callback reports progress and cancels cooperatively; bindings add async in their language's model. Async in the core would need a runtime dependency.

Lazy reading needs synchronous reads, which browsers allow only in a worker. In the browser the core runs in a worker, and the main-thread API returns promises. The page stays responsive and the core stays the same code as everywhere else.

The core calls a caller's callback, a sink's append or a save's progress, synchronously mid-operation, while on a page's main thread it runs in a worker. Callbacks run on the caller's thread: the worker blocks with `Atomics.wait` on a `SharedArrayBuffer` while the main thread answers each call, which needs cross-origin isolation (COOP and COEP headers); without it, passing a callback on a main thread fails with `Unsupported`. A function cannot move to a worker, a callback then sees the caller's own state, the DOM included, and an asynchronous callback would need an asynchronous core.

Remote and hardware signers answer over a network or a device: waiting, not computing. Signing runs in steps, prepare → the caller supplies the signature bytes → finalize, so those signers run in the binding's async model. The core never awaits.
