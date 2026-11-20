# Dependencies are licensed, audited and justified

Only permissive licenses are allowed, so StarlightPDF stays usable under Apache-2.0 in closed-source products. The one exception is shellcheck (GPL-3.0), which CI only runs: nothing links, vendors or ships it. cargo-deny checks licenses and advisories. cargo-vet requires a source audit of every package version, recorded here or imported from Mozilla, Google, Bytecode Alliance or ISRG, and no exemptions. Each direct dependency's reason is a `#` comment on its manifest line.

## Cryptography is written in the core

Decrypting PDFs needs AES, SHA-2, MD5 and RC4. They are written in the core crate, not taken from RustCrypto. They are small and fully specified with published test vectors, and constant time matters little for decrypting.
