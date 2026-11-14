---
name: issue
description: Write a GitHub issue with a goal and a definition of done.
argument-hint: <what is wrong or wanted>
disable-model-invocation: true
---

Write a GitHub issue for: $ARGUMENTS

1. **Research.** Read `README.md`, `GLOSSARY.md`, `.architecture-decisions/` and the relevant code. Search existing issues; if one covers this, say so and stop.
2. **Classify.** Bug: existing behaviour is wrong. Feature: new behaviour. Decide yourself.
3. **Ask.** Ask only for a fact you cannot find in the repository, its issues or its pull requests, or a choice that is hard to reverse, surprising without context and a real trade-off. Make every other choice yourself. Recommend an answer to every question, multi-select included. A few questions at a time. The issue must establish, by finding or deciding:
   - Bug: steps to reproduce, input, expected, actual.
   - Feature: who needs it, scope, which standard and clause applies.
   - Decisions: the choices you asked about. A breaking API change is always one.
4. **Draft.** Fill in `.github/ISSUE_TEMPLATE/bug.md` or `feature.md`, without its front matter, and show it with the choices you made yourself, so the user can correct them.
5. **Post.** Only after the user approves. Label `bug` or `feature`, plus `build-ready`. Reply with the link.

## Writing rules

- **Title:** the outcome, imperative, under 70 characters.
- **Goal:** the outcome and who benefits. Never the implementation.
- **Definition of done:** an item only for a result the goal needs, never for how to reach it. Cover the edge cases that can occur in this task, and no others. Each item is one observable result that a test or command can check, citing the standard and clause it verifies, if any. "Opening an empty file returns error X", not "the parser is robust". No test files, modules, function names or algorithms.
- **Decisions:** only choices the user made that are hard to reverse, surprising without context and a real trade-off, each with its reason in one sentence. Omit the section if there are none.
- **One issue, one goal.** Split anything larger.
- Use `GLOSSARY.md` terms.
