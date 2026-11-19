---
name: issue
description: Write a GitHub issue with a goal and a definition of done.
argument-hint: <what is wrong or wanted>
disable-model-invocation: true
---

Write a GitHub issue for: $ARGUMENTS

1. **Research.** Read `README.md`, `GLOSSARY.md`, `.architecture-decisions/` and the relevant code, and note the files each task will change. Search existing issues; if one covers this, say so and stop. Find open issues that must merge before this one (`Blocked by`) or after it (`Blocking`). Research the dependencies the task needs: reason, license, repository, registry page, homepage.
2. **Classify.** Bug: existing behaviour is wrong. Feature: new behaviour. A merged issue is a bug only if every task is one. Decide yourself.
3. **Ask.** Ask only for a fact you cannot find in the repository, its issues or its pull requests, or a choice that is hard to reverse, surprising without context and a real trade-off. Make every other choice yourself. Recommend an answer to every question, multi-select included. A few questions at a time. The issue must establish, by finding or deciding:
   - Bug: steps to reproduce, input, expected, actual.
   - Feature: who needs it, scope, which standard and clause applies.
   - Decisions: the choices you asked about. A breaking API change and every new dependency are always one; ask about each, recommending use or writing it here.
4. **Draft.** For each issue, fill in `.github/ISSUE_TEMPLATE/bug.md` or `feature.md`, without its front matter, and show it with the choices you made yourself, which tasks were merged and which were split with the files that decided each, and each open `Blocked by` and `Blocking` issue's number, title and one-sentence reason, so the user can correct them. Omit an empty list.
5. **Post.** Only after the user approves. Label `bug` or `feature`, plus `build-ready`. Pass approved numbers whose `gh issue view <n> --json state` is `OPEN` as `gh issue create --blocked-by <n,n> --blocking <n,n>`, omitting an empty flag. Reply with the link.

## Writing rules

- **Title:** the outcome, imperative, under 70 characters.
- **Goal:** the outcome and who benefits. Never the implementation.
- **Definition of done:** an item only for a result the goal needs, never for how to reach it. Cover the edge cases that can occur in this task, and no others. Each item is one observable result that a test or command can check, citing the standard and clause it verifies, if any. "Opening an empty file returns error X", not "the parser is robust". No test files, modules, function names or algorithms.
- **Decisions:** only choices the user made that are hard to reverse, surprising without context and a real trade-off, each with its reason in one sentence. A dependency: `**name version** (license): reason. [Repository](…), [crates.io](…), [homepage](…).`, homepage if any. Omit the section if there are none.
- **Group by files.** Tasks mostly in the same files are one issue, given apart or as one, with one title and one goal covering all; split off only a task with another goal and changes mostly other files. A file most changes touch (`README.md`, `Cargo.lock`) is no reason to merge.
- Use `GLOSSARY.md` terms.
