# StarlightPDF 🌟

An open-source PDF library that aims to cover everything you can do with a PDF: read, extract, create, edit, sign, render and optimize.

Website: [starlightpdf.com](https://starlightpdf.com)

> **Project status:** StarlightPDF is at the vision stage. The lists below describe where the project is going, and the status markers show how far each part has come.

## Legend

| Status | Meaning |
| --- | --- |
| ✅ | Done |
| 👨‍💻 | Working |
| 🔜 | Next up |
| 💤 | Backlog |

## Languages

StarlightPDF will be available as a dependency in:

| Status | Language | Package source |
| --- | --- | --- |
| 👨‍💻 | Rust | crates.io |
| 💤 | C and C++ | C API, vcpkg, Conan |
| 👨‍💻 | Python | PyPI |
| 💤 | .NET (C#, F#, VB.NET) | NuGet |
| 💤 | Go | Go modules |
| 💤 | Java, Kotlin, Scala | Maven Central |
| 👨‍💻 | JavaScript and TypeScript (Node.js, Deno, Bun, browser) | npm, WebAssembly |
| 💤 | Swift and Objective-C | Swift Package Manager, CocoaPods |
| 💤 | Dart and Flutter | pub.dev |
| 🔜 | PHP | Packagist |
| 💤 | Ruby | RubyGems |
| 💤 | R | CRAN |
| 💤 | Julia | General registry |
| 💤 | Elixir and Erlang | Hex |
| 💤 | Perl | CPAN |
| 💤 | Lua | LuaRocks |
| 💤 | Haskell | Hackage |
| 💤 | Zig | Zig package manager |
| 💤 | Any other language | Command-line tool |

## Architecture

All logic is written once, in a Rust core crate. Four interfaces expose it:

| Interface | Tool | For |
| --- | --- | --- |
| C API | `extern "C"`, cbindgen header | Most listed languages |
| Native bindings | PyO3, napi-rs, UniFFI | Python (PyO3), Node.js (napi-rs), Swift and Kotlin (UniFFI) |
| WebAssembly | wasm-bindgen | JavaScript in the browser |
| Command line | Standalone command-line tool | Everything else |

CI builds prebuilt binaries per platform and publishes them to each package registry.

Design decisions are recorded in [`.architecture-decisions/`](.architecture-decisions/).

## Features

### Reading and inspecting

- 👨‍💻 **Open and parse:** from file, stream or memory; password-protected files; lazy loading of large files
- 👨‍💻 **Document info:** page count, PDF version, page sizes, fonts used, embedded files, encryption status
- 💤 **Metadata:** read title, author, subject, keywords, dates; XMP metadata
- 👨‍💻 **Structure access:** raw objects, streams, cross-reference table, document catalog
- 💤 **Validation:** check a file against the PDF specification and report errors
- 👨‍💻 **Repair:** recover damaged or malformed files, rebuild the cross-reference table

### Content extraction

- 💤 **Text:** plain text, text with positions, fonts and colors, reading-order reconstruction
- 💤 **Layout analysis:** words, lines, paragraphs, columns, headings
- 💤 **Tables:** detect and extract to rows and columns
- 💤 **Images:** extract embedded images in original format, or convert on export to PNG, JPEG, TIFF, HEIC/HEIF
- 💤 **Other content:** vector graphics, attachments, links, bookmarks, form data, annotations, embedded fonts
- 💤 **Conversion out:** to Markdown, HTML, XML, JSON, Word, Excel, plain text
- 💤 **OCR:** recognize text in scanned pages, add a searchable text layer
- 💤 **Search:** find text, regular expressions, return hit positions

### Page operations

- 👨‍💻 **Combine and divide:** merge files, split by page, range, size or bookmark, extract pages
- 💤 **Arrange:** reorder, delete, duplicate, insert blank pages, insert pages from other files
- 💤 **Transform:** rotate, crop, resize, scale, change page boxes (media, crop, bleed, trim, art)
- 💤 **Imposition:** N-up (several pages per sheet), booklets, posters, tiling
- 💤 **Superimpose:** place one page over or under another (letterheads, backgrounds)

### Creating content

- 💤 **Text:** fonts, sizes, colors, alignment, wrapping, right-to-left and vertical scripts, CJK
- 💤 **Fonts:** standard fonts, TrueType and OpenType embedding, subsetting, fallback
- 💤 **Graphics:** lines, shapes, paths, curves, fills, gradients, patterns, transparency, blend modes, clipping
- 💤 **Images:** masks and transparency, and these formats:
  - 💤 Common raster: JPEG, PNG, GIF, BMP, TIFF
  - 💤 Modern raster: WebP, HEIC/HEIF, AVIF, JPEG XL, JPEG 2000
  - 💤 Vector: SVG, EPS, EMF, WMF
  - 💤 Scanning and fax: JBIG2, CCITT Group 3 and 4, multi-page TIFF
  - 💤 Camera RAW: DNG, CR2, CR3, NEF, ARW, ORF, RAF, RW2
  - 💤 Professional and print: PSD, OpenEXR, Radiance HDR, DPX
  - 💤 Medical: DICOM
  - 💤 Legacy and other: ICO, TGA, PCX, PNM (PBM, PGM, PPM), XBM, XPM, DDS, QOI
- 💤 **Image handling:** EXIF orientation, ICC color profiles, HDR and 10-bit to standard conversion, multi-image files (HEIF sequences, multi-page TIFF), Live Photo stills
- 💤 **Layout engine:** paragraphs, tables, lists, columns, automatic page breaks, headers, footers, page numbers
- 💤 **Generators:** table of contents, barcodes, QR codes, charts
- 💤 **Conversion in:** from HTML and CSS, Markdown, Office documents, templates, and images (JPEG, PNG, TIFF, HEIC/HEIF), including many images into one PDF

### Editing existing content

- 💤 **Add:** text, images and shapes on existing pages
- 💤 **Stamps:** watermarks, headers, footers, page numbers, Bates numbering
- 💤 **Change:** find and replace text, replace or remove images, edit vector objects
- 💤 **Redaction:** permanently remove content, not just cover it
- 💤 **Flatten:** merge annotations, form fields, layers or transparency into the page
- 💤 **Metadata:** write or strip document info and XMP

### Navigation and interactivity

- 💤 **Bookmarks:** create, edit, delete, nest
- 💤 **Links:** internal links, web links, named destinations
- 💤 **Viewer settings:** page labels, opening page, layout and zoom, viewer preferences
- 💤 **Actions:** JavaScript, page transitions, article threads
- 💤 **Rich content:** layers (optional content), embedded multimedia, 3D

### Annotations

- 💤 **Markup:** highlight, underline, strikeout, squiggly
- 💤 **Notes and shapes:** sticky notes, free text, lines, arrows, rectangles, ink
- 💤 **Other types:** stamps, file attachments, redaction marks, popups
- 💤 **Management:** read, add, edit, delete, flatten, import and export (FDF, XFDF), appearance generation

### Forms

- 💤 **Fields:** text, checkbox, radio, dropdown, list, button, signature
- 💤 **Use:** read values, fill, flatten, import and export data (FDF, XFDF, XML, JSON)
- 💤 **Build:** create and edit fields, formatting, validation, calculation scripts
- 💤 **Legacy:** XFA forms

### Security

- 💤 **Encryption:** RC4, AES-128, AES-256; user and owner passwords; certificate-based
- 💤 **Permissions:** restrict printing, copying, editing, form filling
- 👨‍💻 **Decryption:** remove protection with the password
- 💤 **Sanitizing:** remove hidden data, scripts, metadata, attachments

### Digital signatures

- 💤 **Sign:** certificate signatures, visible or invisible, multiple signatures, incremental saves
- 💤 **Advanced:** timestamps, long-term validation, PAdES levels, certification signatures
- 💤 **Hardware:** smart cards, HSMs, remote signing services
- 💤 **Verify:** validate signatures, certificate chains, revocation, detect changes after signing

### Standards and compliance

- 💤 **Archiving:** PDF/A creation, conversion, validation
- 💤 **Accessibility:** PDF/UA, tagged PDF, structure tree, alt text, reading order
- 💤 **Print:** PDF/X, output intents, ICC color profiles
- 💤 **Other:** PDF/E, PDF/VT, PDF 2.0, ZUGFeRD and Factur-X e-invoices

### Rendering and printing

- 💤 **Rasterize:** pages to PNG, JPEG, TIFF, HEIC/HEIF; thumbnails; chosen DPI and color space
- 💤 **Vector export:** to SVG, PostScript
- 💤 **Print:** send to a printer, with tray, duplex and scaling options
- 💤 **Display:** viewer components, annotation and form rendering, layer visibility

### Optimization

- 💤 **Compress:** streams, object streams, image downsampling and recompression
- 💤 **Clean up:** remove unused objects, duplicate fonts and images, subset fonts
- 💤 **Linearize:** fast web view
- 💤 **Convert:** color spaces (RGB, CMYK, grayscale), PDF version

### Attachments and packaging

- 💤 **Embedded files:** add, extract, remove
- 💤 **Portfolios:** PDF collections, associated files

### Comparison and analysis

- 💤 **Diff:** compare two files by text, visually or by structure
- 💤 **Preflight:** check fonts, images, colors and print readiness

### Engineering properties

- 💤 **Performance:** streaming, low memory use, incremental updates, thread safety, async, batch processing
- 💤 **Deployment:** minimal, audited dependencies, cross-platform, WebAssembly, command-line tool
- 💤 **Robustness:** tolerance of malformed files, safe handling of untrusted input

## Working with Claude Code

| Command | What it does |
| --- | --- |
| `/issue <description>` | Drafts a bug or feature issue. |
| `/build [issue number]` | Builds the issue, opens a pull request and merges it when the merge command's conditions hold; refuses one blocked by an open issue and asks first if it is `building`. Without a number, labels `building` and builds the lowest open `build-ready` issue you opened that is neither. |

`/build` shows a progress pane, enabled and kept up to date by `.claude/settings.json`. Install it if missing:

```
claude plugin install build-voyage --marketplace Martin-Carlsson/StarlightPDF
```

`main` accepts only signed commits. To sign locally, add your SSH key on GitHub as a *Signing Key*.

## Contributing

StarlightPDF is just getting started, and contributions are welcome. Open an issue to discuss a feature from the list above before starting work on it, so effort isn't duplicated.

Before pushing, run `sh ci/setup.sh` once, then `sh ci/check.sh <area>` for each area CI runs.

## Legal

### License

StarlightPDF is released under the [Apache License 2.0](LICENSE). You can use it in open-source and closed-source projects, including commercial products.

### Governing law and venue

Danish law. Copenhagen City Court.

### Dependencies

| Name | License | Links | What and why |
| --- | --- | --- | --- |
| pyo3 | [MIT](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/pyo3/pyo3), [crates.io](https://crates.io/crates/pyo3), [Homepage](https://pyo3.rs/) | Python binding layer; hand-written FFI would be large and unsafe. |
| pyo3-ffi | [MIT](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/pyo3/pyo3), [crates.io](https://crates.io/crates/pyo3-ffi), [Homepage](https://pyo3.rs/) | CPython C API declarations; needed by pyo3. |
| pyo3-macros | [MIT](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/pyo3/pyo3), [crates.io](https://crates.io/crates/pyo3-macros), [Homepage](https://pyo3.rs/) | pyo3's procedural macros; needed by pyo3. |
| pyo3-macros-backend | [MIT](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/pyo3/pyo3), [crates.io](https://crates.io/crates/pyo3-macros-backend), [Homepage](https://pyo3.rs/) | Code generation; needed by pyo3-macros. |
| pyo3-build-config | [MIT](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/pyo3/pyo3/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/pyo3/pyo3), [crates.io](https://crates.io/crates/pyo3-build-config), [Homepage](https://pyo3.rs/) | Finds the Python interpreter at build time; needed by pyo3 and pyo3-ffi. |
| libc | [MIT](https://github.com/rust-lang/libc/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-lang/libc/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-lang/libc), [crates.io](https://crates.io/crates/libc) | C library types and functions; needed by pyo3, pyo3-ffi, cc, jobserver and getrandom. |
| once_cell | [MIT](https://github.com/matklad/once_cell/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/matklad/once_cell/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/matklad/once_cell), [crates.io](https://crates.io/crates/once_cell) | One-time initialization; needed by pyo3. |
| portable-atomic | [Apache-2.0](https://github.com/taiki-e/portable-atomic/blob/HEAD/LICENSE-APACHE) OR [MIT](https://github.com/taiki-e/portable-atomic/blob/HEAD/LICENSE-MIT) | [Repository](https://github.com/taiki-e/portable-atomic), [crates.io](https://crates.io/crates/portable-atomic) | Atomic types on every target; needed by pyo3. |
| target-lexicon | [Apache-2.0](https://github.com/bytecodealliance/target-lexicon/blob/HEAD/LICENSE) WITH [LLVM-exception](https://github.com/bytecodealliance/target-lexicon/blob/HEAD/LICENSE) | [Repository](https://github.com/bytecodealliance/target-lexicon), [crates.io](https://crates.io/crates/target-lexicon) | Parses target triples; needed by pyo3-build-config. |
| proc-macro2 | [MIT](https://github.com/dtolnay/proc-macro2/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/dtolnay/proc-macro2/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/dtolnay/proc-macro2), [crates.io](https://crates.io/crates/proc-macro2) | Token streams for procedural macros; needed by pyo3-macros and wasm-bindgen-macro. |
| quote | [MIT](https://github.com/dtolnay/quote/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/dtolnay/quote/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/dtolnay/quote), [crates.io](https://crates.io/crates/quote) | Turns templates into Rust tokens; needed by pyo3-macros and wasm-bindgen-macro. |
| syn | [MIT](https://github.com/dtolnay/syn/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/dtolnay/syn/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/dtolnay/syn), [crates.io](https://crates.io/crates/syn) | Parses Rust source; needed by pyo3-macros and wasm-bindgen-macro. |
| heck | [MIT](https://github.com/withoutboats/heck/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/withoutboats/heck/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/withoutboats/heck), [crates.io](https://crates.io/crates/heck) | Case conversion; needed by pyo3-macros-backend. |
| unicode-ident | ([MIT](https://github.com/dtolnay/unicode-ident/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/dtolnay/unicode-ident/blob/HEAD/LICENSE-APACHE)) AND [Unicode-3.0](https://github.com/dtolnay/unicode-ident/blob/HEAD/LICENSE-UNICODE) | [Repository](https://github.com/dtolnay/unicode-ident), [crates.io](https://crates.io/crates/unicode-ident) | Identifier character tables; needed by proc-macro2, syn and wasm-bindgen-shared. |
| wasm-bindgen | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen), [crates.io](https://crates.io/crates/wasm-bindgen), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | WebAssembly glue; hand-written glue needs `unsafe`. |
| js-sys | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys), [crates.io](https://crates.io/crates/js-sys), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | JavaScript built-in types; needed with wasm-bindgen. |
| wasm-bindgen-macro | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen), [crates.io](https://crates.io/crates/wasm-bindgen-macro), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | wasm-bindgen's procedural macros; needed by wasm-bindgen. |
| wasm-bindgen-macro-support | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen), [crates.io](https://crates.io/crates/wasm-bindgen-macro-support), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | Code generation; needed by wasm-bindgen-macro. |
| wasm-bindgen-shared | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen), [crates.io](https://crates.io/crates/wasm-bindgen-shared), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | Data shared by the macros and wasm-bindgen-cli; needed by wasm-bindgen and wasm-bindgen-macro-support. |
| rustversion | [MIT](https://github.com/dtolnay/rustversion/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/dtolnay/rustversion/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/dtolnay/rustversion), [crates.io](https://crates.io/crates/rustversion) | Checks the compiler version at build time; needed by wasm-bindgen. |
| bumpalo | [MIT](https://github.com/fitzgen/bumpalo/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/fitzgen/bumpalo/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/fitzgen/bumpalo), [crates.io](https://crates.io/crates/bumpalo) | Arena allocation; needed by wasm-bindgen-macro-support. |
| libfuzzer-sys | ([MIT](https://github.com/rust-fuzz/libfuzzer/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-fuzz/libfuzzer/blob/HEAD/LICENSE-APACHE)) AND [NCSA](https://github.com/rust-fuzz/libfuzzer/blob/HEAD/README.md#license) | [Repository](https://github.com/rust-fuzz/libfuzzer), [crates.io](https://crates.io/crates/libfuzzer-sys) | libFuzzer runtime: the standard coverage-guided fuzzer for Rust; large to write here. |
| arbitrary | [MIT](https://github.com/rust-fuzz/arbitrary/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-fuzz/arbitrary/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-fuzz/arbitrary), [crates.io](https://crates.io/crates/arbitrary) | Structured fuzz inputs; needed by libfuzzer-sys. |
| cc | [MIT](https://github.com/rust-lang/cc-rs/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-lang/cc-rs/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-lang/cc-rs), [crates.io](https://crates.io/crates/cc) | Compiles libFuzzer at build time; needed by libfuzzer-sys. |
| find-msvc-tools | [MIT](https://github.com/rust-lang/cc-rs/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-lang/cc-rs/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-lang/cc-rs), [crates.io](https://crates.io/crates/find-msvc-tools) | Finds MSVC on Windows; needed by cc. |
| jobserver | [MIT](https://github.com/rust-lang/jobserver-rs/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-lang/jobserver-rs/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-lang/jobserver-rs), [crates.io](https://crates.io/crates/jobserver) | Limits parallel compiler jobs; needed by cc. |
| shlex | [MIT](https://github.com/comex/rust-shlex/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/comex/rust-shlex/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/comex/rust-shlex), [crates.io](https://crates.io/crates/shlex) | Splits compiler flags; needed by cc. |
| getrandom | [MIT](https://github.com/rust-random/getrandom/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-random/getrandom/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-random/getrandom), [crates.io](https://crates.io/crates/getrandom) | OS random numbers on Windows; needed by jobserver. |
| cfg-if | [MIT](https://github.com/rust-lang/cfg-if/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-lang/cfg-if/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-lang/cfg-if), [crates.io](https://crates.io/crates/cfg-if) | Platform-specific items; needed by getrandom, wasm-bindgen and js-sys. |
| r-efi | [MIT](https://github.com/r-efi/r-efi/blob/HEAD/AUTHORS) OR [Apache-2.0](https://github.com/r-efi/r-efi/blob/HEAD/AUTHORS) OR [LGPL-2.1-or-later](https://github.com/r-efi/r-efi/blob/HEAD/AUTHORS) | [Repository](https://github.com/r-efi/r-efi), [crates.io](https://crates.io/crates/r-efi) | UEFI definitions; needed by getrandom on UEFI. |
| cargo-deny | [MIT](https://github.com/EmbarkStudios/cargo-deny/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/EmbarkStudios/cargo-deny/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/EmbarkStudios/cargo-deny), [crates.io](https://crates.io/crates/cargo-deny/0.20.2) | CI tool; checks licenses and advisories. |
| cargo-vet | [Apache-2.0](https://github.com/mozilla/cargo-vet/blob/HEAD/LICENSE-APACHE) OR [MIT](https://github.com/mozilla/cargo-vet/blob/HEAD/LICENSE-MIT) | [Repository](https://github.com/mozilla/cargo-vet), [crates.io](https://crates.io/crates/cargo-vet/0.10.3), [Homepage](https://mozilla.github.io/cargo-vet/) | CI tool; records and imports source audits. |
| cargo-fuzz | [MIT](https://github.com/rust-fuzz/cargo-fuzz/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/rust-fuzz/cargo-fuzz/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/rust-fuzz/cargo-fuzz), [crates.io](https://crates.io/crates/cargo-fuzz/0.13.2) | CI tool; runs the fuzz targets. |
| wasm-bindgen-cli | [MIT](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-MIT) OR [Apache-2.0](https://github.com/wasm-bindgen/wasm-bindgen/blob/HEAD/LICENSE-APACHE) | [Repository](https://github.com/wasm-bindgen/wasm-bindgen), [crates.io](https://crates.io/crates/wasm-bindgen-cli/0.2.130), [Homepage](https://wasm-bindgen.github.io/wasm-bindgen/) | CI tool; writes the JavaScript glue. |
| actionlint | [MIT](https://github.com/rhysd/actionlint/blob/HEAD/LICENSE.txt) | [Repository](https://github.com/rhysd/actionlint), [pkg.go.dev](https://pkg.go.dev/github.com/rhysd/actionlint), [Homepage](https://rhysd.github.io/actionlint/) | CI tool; lints workflows. The standard tool, and large to write here. |
| shellcheck | [GPL-3.0](https://github.com/koalaman/shellcheck/blob/HEAD/LICENSE) | [Repository](https://github.com/koalaman/shellcheck), [Homepage](https://www.shellcheck.net) | CI tool; lints the shell scripts and workflow run: scripts. The standard tool. CI only runs it. |
| prettier | [MIT](https://github.com/prettier/prettier/blob/HEAD/LICENSE) | [Repository](https://github.com/prettier/prettier), [npm](https://www.npmjs.com/package/prettier), [Homepage](https://prettier.io) | CI tool; formats the mods and the JavaScript binding. The standard tool. |
| ruff | [MIT](https://github.com/astral-sh/ruff/blob/HEAD/LICENSE) | [Repository](https://github.com/astral-sh/ruff), [PyPI](https://pypi.org/project/ruff/), [Homepage](https://docs.astral.sh/ruff) | CI tool; lints and formats the Python. The standard tool, and large to write here. |
| mypy | [MIT](https://github.com/python/mypy/blob/HEAD/LICENSE) | [Repository](https://github.com/python/mypy), [PyPI](https://pypi.org/project/mypy/), [Homepage](https://www.mypy-lang.org) | CI tool; type-checks the Python binding's stub and tests. The standard tool, and large to write here. Needs typing_extensions (PSF-2.0) and pathspec (MPL-2.0), allowed because CI-only. |
| typescript | [Apache-2.0](https://github.com/microsoft/TypeScript/blob/HEAD/LICENSE.txt) | [Repository](https://github.com/microsoft/TypeScript), [npm](https://www.npmjs.com/package/typescript), [Homepage](https://www.typescriptlang.org/) | CI tool; compiles the JavaScript binding and type-checks it and its tests, and locally the mods. The standard tool. |
| Noto Emoji | [Apache-2.0](https://github.com/googlefonts/noto-emoji/blob/HEAD/LICENSE) | [Repository](https://github.com/googlefonts/noto-emoji) | Source of the 🌟 logo; Copyright 2013 Google, Inc. |
