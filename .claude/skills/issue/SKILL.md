---
name: issue
description: Write a GitHub issue with a goal and a definition of done.
argument-hint: <what is wrong or wanted>
disable-model-invocation: true
---

Write a GitHub issue for: $ARGUMENTS

1. **Research.** Read `README.md`, `GLOSSARY.md`, `.architecture-decisions/` and the relevant code. Search existing issues; if one covers this, say so and stop.
2. **Classify.** Bug: existing behaviour is wrong. Feature: new behaviour. Ask if unclear.
3. **Ask.** Ask what you could not find out, a few questions at a time, until you can write every section without guessing.
   - Bug: steps to reproduce, input, expected, actual.
   - Feature: who needs it, what is in and out of scope, which standard and clause applies.
   - Decisions: any choice the work forces that is hard to reverse, surprising without context and a real trade-off. A breaking API change is always one. Lay out the options and recommend one.
4. **Draft.** Fill in `.github/ISSUE_TEMPLATE/bug.md` or `feature.md`, without its front matter, and show it.
5. **Post.** Only after the user approves. Label `bug` or `feature`, plus `build-ready`. Reply with the link.

## Writing rules

- **Title:** the outcome, imperative, under 70 characters.
- **Goal:** the outcome and who benefits. Never the implementation.
- **Definition of done:** each item is one observable result that a test or command can check. "Opening an empty file returns error X", not "the parser is robust".
- **Cover what is easy to forget:** malformed input, edge cases, round-trip. Cite the standard and clause an item verifies.
- **Say what must be proven, not how.** No test files, modules, function names or algorithms.
- **Decisions:** only what the user decided, each with its reason in one sentence. Omit the section if there are none.
- **One issue, one goal.** Split anything larger.
- Use `GLOSSARY.md` terms.
