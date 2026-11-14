# Rules

1. **No dependencies, no exceptions.** Use only the language's standard library. Never add a third-party package, including for cryptography, image formats or OCR. If something is missing, write it or ask.
2. **Strictest settings, always.** Enable every compiler and linter check at its strictest level. Warnings are errors. Never disable, suppress or weaken a check to make code pass.
3. **Shortest text, always.** Answer the user with as little text as possible. The same goes for code comments and documentation.

# Documentation

## GLOSSARY.md

One root `GLOSSARY.md`.

```md
# StarlightPDF

{One or two sentences on what this project is.}

## Language

**Term**:
{One or two sentences on what it is.}
_Avoid_: {rejected synonyms}
```

- One preferred term per concept; alternatives go under `_Avoid_`.
- Project-specific terms only, no general programming concepts.
- Group related terms under subheadings.
