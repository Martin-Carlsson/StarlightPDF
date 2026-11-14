# Rules

1. **No dependencies, no exceptions.** Use only the language's standard library. Never add a third-party package, including for cryptography, image formats or OCR. If something is missing, write it or ask.
2. **Strictest settings, always.** Enable every compiler and linter check at its strictest level. Warnings are errors. Never disable, suppress or weaken a check to make code pass.
3. **Shortest text, always.** Answer the user with as little text as possible. The same goes for code comments and documentation.

# Documentation

- `README.md`: vision and roadmap only.
- `docs/`: everything else, plain Markdown, one topic per file, lowercase `kebab-case.md` names.
- `docs/README.md`: index linking every page. Update it when adding, renaming or removing a page.
- Change the docs in the same PR as the behavior they describe.
