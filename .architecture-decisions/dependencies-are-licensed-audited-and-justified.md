# Dependencies are licensed, audited and justified

Only permissive licenses are allowed, so StarlightPDF stays usable under Apache-2.0 in closed-source products. The one exception is shellcheck (GPL-3.0), which CI only runs: nothing links, vendors or ships it. cargo-deny checks licenses and advisories. cargo-vet requires a source audit of every package version, recorded here or imported from Mozilla, Google, Bytecode Alliance or ISRG, and no exemptions. Each direct dependency's reason is a `#` comment on its manifest line.

## Cryptography is written in the core

Decrypting PDFs needs AES, SHA-2, MD5 and RC4. They are written in the core crate, not taken from RustCrypto. They are small and fully specified with published test vectors, and constant time matters little for decrypting.

## Rendering takes three decoders and writes the rest

Rendering needs decoders, font parsers, a rasterizer and colour conversion. Only the JPEG 2000, JBIG2 and CCITT fax decoders are dependencies (hayro): they are large and risky to write, and forbid `unsafe`. They are unbounded on hostile input, so the core passes them only what its guards have bounded, and what the guards cannot bound fails `Unsupported`. The two CC0-1.0 ICC profiles inside hayro-jpeg2000 are accepted, as CC0 asks nothing of users.

The rest is written in the core. Type 1 must be written anyway and embedding needs the same font parsers, so not ttf-parser or skrifa; pages are rendered in bands, so not tiny-skia; ICC and JPEG are fully specified, and moxcms and zune-jpeg bring `unsafe` code.
