---
name: build
description: Build a GitHub issue until its definition of done is met, then open a pull request.
argument-hint: <issue number>
disable-model-invocation: true
hooks:
  Stop:
    - hooks:
        - type: command
          command: sh "$CLAUDE_PROJECT_DIR/.claude/skills/build/hook.sh" stop
  PreToolUse:
    - hooks:
        - type: command
          command: sh "$CLAUDE_PROJECT_DIR/.claude/skills/build/hook.sh" tool
---

Build issue $ARGUMENTS. Do not stop until its Definition of done is met and the pull request is open.

A Stop hook sends you back unless every check has passed on the files as they are now. After 24 hours without the user, tool calls are refused: stop and ask whether to continue.

1. **Read** the issue and `CLAUDE.md`. If the Goal or Definition of done is missing or not checkable, ask the user and update the issue. Then make sure it is labelled `build-ready` and `bug` or `feature`.
2. **Define checks.** For each Definition of done item, write a shell command that exits 0 only when the item holds: a test only where the item needs one and `CLAUDE.md` calls for it, otherwise a direct command. Run each: it should fail now. Write `.claude/build/state`:

   ```
   status: working
   issue: <number> <title>
   phase: explore
   check: <command, run from the repository root>
   ```

   Keep `phase:` current: `explore`, `test`, `implement`, `review`, `pr`.

   Then create branch `<number>-<slug>`, at most 60 characters, from `origin/main`. If it already exists, check its pull request and commits; ask only if someone else may be working on it.

   Show the checks. Do not wait for approval.
3. **Orchestrate.** You plan, delegate and verify; subagents do the work. Brief each with the issue, its items, the files and the checks it must pass.
   - **Explore:** subagents map the relevant code and spec clauses, in parallel.
   - **Test:** where an item needs a test and `CLAUDE.md` calls for it, a subagent writes it, failing, from the issue and the spec, never from the implementation.
   - **Implement:** a different subagent per item makes its check pass by the simplest means that fully meets it: instructions or configuration before code, existing code before new. Add nothing that no item needs: no extra option, abstraction, fallback, check or test. Bug: reproduce first, then fix. Run items in parallel when their files don't overlap.
   - **Review:** a fresh subagent reviews the diff against the issue and `CLAUDE.md`, and classifies the API change itself. It traces every file, function and test the diff adds to an item, and asks for the rest to be removed or simplified, and for every documentation file and code comment the diff adds or changes to be merged with an existing one on the same topic, deleted if unneeded, or shortened, where it can be; such a fix leaves no file that repeats another. Delegate the fixes and review again until clean.

   After every hand-back and after any change, run `sh .claude/skills/build/hook.sh check` from the repository root yourself; never trust a report.
4. **Open the pull request** once every check passes and the review is clean: commit, push, fill in `.github/pull_request_template.md`, ready for review. Label it `api-additive` or `api-breaking`; no label means none. Then add `pr: <url>` to the state file.

## Rules

- **Decisions in the issue are constraints.** Record one in `.architecture-decisions/` only if it is hard to reverse, surprising without context and a real trade-off; follow the rest without recording them. If one does not work, ask; never deviate.
- **Questions.** Ask only for a fact you cannot find in the repository, its issues or its pull requests, or a choice that is hard to reverse, surprising without context and a real trade-off. Make every other choice yourself. Recommend an answer to every question, multi-select included. Before you ask, look up again every fact the question depends on.
- **API change:** `none`, `additive` or `breaking`. Breaking means existing callers must change or get a different result, including different bytes written. Use it before 1.0 too. If the change is breaking and the issue does not say so, ask.
- **Never weaken a check.** Change one only if it was wrong, and say why in the pull request.
- **Ask, don't loop.** If you must ask, or a check has failed three times and you have no new idea: set `status: blocked`, ask one precise question and stop. When they answer, set `status: working`.
- **Cancelled.** If the user cancels the build, delete `.claude/build/state`.
