# CLAUDE.md

Instructions for Claude Code when working in this repository.

StarlightPDF is an open-source PDF library with zero dependencies. The project is at the vision stage; [README.md](README.md) holds the roadmap and the status of each feature.

## Rules

These rules have no exceptions. If one of them seems to block the task, stop and ask instead of working around it.

### 1. No dependencies

StarlightPDF uses only the language's standard library.

- Never add a third-party package, whatever it is for. Cryptography, image formats and OCR are not exceptions.
- If the standard library lacks something, write it in this repository or ask.

### 2. Strictest settings

Every compiler and linter check is enabled at its strictest level, and warnings are errors.

- Never disable, suppress or weaken a check to make code pass. Fix the code instead.
- When adding a new tool or language target, configure it at its strictest level from the start.
