# Long operations

PDF work is computing, not waiting. The core is synchronous; a callback reports progress and cancels cooperatively; bindings add async in their language's model. Async in the core would need a runtime dependency.

Lazy reading needs synchronous reads, which browsers allow only in a worker. In the browser the core runs in a worker, and the main-thread API returns promises. The page stays responsive and the core stays the same code as everywhere else.

On a page's main thread the core runs in a worker yet calls a sink's append or an operation's progress synchronously: the worker blocks with `Atomics.wait` on a `SharedArrayBuffer` while the main thread answers, which needs cross-origin isolation (COOP and COEP); without it such a callback fails `Unsupported`. A function cannot move to a worker, and an asynchronous callback would need an asynchronous core.

Remote and hardware signers answer over a network or a device: waiting, not computing. Signing runs in steps, prepare → the caller supplies the signature bytes → finalize, so those signers run in the binding's async model. The core never awaits.
