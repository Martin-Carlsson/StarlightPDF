# StarlightPDF

An open-source PDF library with zero dependencies that aims to cover everything you can do with a PDF: read, extract, create, edit, sign, render and optimize.

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
| 💤 | Rust | crates.io |
| 💤 | C and C++ | C API, vcpkg, Conan |
| 💤 | Python | PyPI |
| 💤 | .NET (C#, F#, VB.NET) | NuGet |
| 💤 | Go | Go modules |
| 💤 | Java, Kotlin, Scala | Maven Central |
| 💤 | JavaScript and TypeScript (Node.js, Deno, Bun, browser) | npm, WebAssembly |
| 💤 | Swift and Objective-C | Swift Package Manager, CocoaPods |
| 💤 | Dart and Flutter | pub.dev |
| 💤 | PHP | Packagist |
| 💤 | Ruby | RubyGems |
| 💤 | R | CRAN |
| 💤 | Julia | General registry |
| 💤 | Elixir and Erlang | Hex |
| 💤 | Perl | CPAN |
| 💤 | Lua | LuaRocks |
| 💤 | Haskell | Hackage |
| 💤 | Zig | Zig package manager |
| 💤 | Any other language | Command-line tool |

## Features

### Reading and inspecting

- 💤 **Open and parse:** from file, stream or memory; password-protected files; lazy loading of large files
- 💤 **Document info:** page count, PDF version, page sizes, fonts used, embedded files, encryption status
- 💤 **Metadata:** read title, author, subject, keywords, dates; XMP metadata
- 💤 **Structure access:** raw objects, streams, cross-reference table, document catalog
- 💤 **Validation:** check a file against the PDF specification and report errors
- 💤 **Repair:** recover damaged or malformed files, rebuild the cross-reference table

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

- 💤 **Combine and divide:** merge files, split by page, range, size or bookmark, extract pages
- 💤 **Arrange:** reorder, delete, duplicate, insert blank pages, insert pages from other files
- 💤 **Transform:** rotate, crop, resize, scale, change page boxes (media, crop, bleed, trim, art)
- 💤 **Imposition:** N-up (several pages per sheet), booklets, posters, tiling
- 💤 **Overlay:** place one page over or under another (letterheads, backgrounds)

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
- 💤 **Decryption:** remove protection with the password
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
- 💤 **Deployment:** zero dependencies, cross-platform, WebAssembly, command-line tool
- 💤 **Robustness:** tolerance of malformed files, safe handling of untrusted input

## Working with Claude Code

| Command | What it does |
| --- | --- |
| `/issue <description>` | Drafts a bug or feature issue. |
| `/build <issue number>` | Builds the issue and opens a pull request. |

`/build` shows a progress pane, enabled by `.claude/settings.json`. Install it if missing; update it if outdated:

```
claude plugin install build-voyage --marketplace Martin-Carlsson/StarlightPDF
claude plugin marketplace update starlightpdf && claude plugin update build-voyage@starlightpdf
```

## Contributing

StarlightPDF is just getting started, and contributions are welcome. Open an issue to discuss a feature from the list above before starting work on it, so effort isn't duplicated.

## License

StarlightPDF is released under the [Apache License 2.0](LICENSE). You can use it in open-source and closed-source projects, including commercial products.
