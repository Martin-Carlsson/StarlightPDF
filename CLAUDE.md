# Rules

1. **No dependencies, no exceptions.** Use only the language's standard library. Never add a third-party package, including for cryptography, image formats or OCR. If something is missing, write it or ask.
2. **Strictest settings, always.** Enable every compiler and linter check at its strictest level. Warnings are errors. Never disable, suppress or weaken a check to make code pass.
3. **Shortest text, always.** Answer the user with as little text as possible. The same goes for code comments and documentation.

# Documentation

## GLOSSARY.md

```md
**Term**:
{One or two sentences on what it is.}
_Avoid_: {rejected synonyms}
```

- One term per concept.
- Project-specific terms only.
- Group related terms under subheadings.

## .architecture-decisions/

- One decision per file: a title, then 1–3 sentences on context, decision and why.
- Name files `descriptive-slug.md`, unnumbered. An agent must be able to find the decision from the name alone.
- Each file stands alone; never build on another decision.
- When a decision changes, rewrite its file. Never add a superseding one.
- Record only decisions that are hard to reverse, surprising without context and a real trade-off.
