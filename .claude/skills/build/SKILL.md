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
2. **Switch branch.** Write `.claude/build/state`:

   ```
   status: working
   issue: <number> <title>
   phase: explore
   ```

   Keep `phase:` current: `explore`, `test`, `implement`, `review`, `pr`.

   Run `sh .claude/skills/build/hook.sh branch`. It switches to the issue's branch, continuing on it if the remote has it, and prints its name. Add `branch: <name>` to the state file. Work only on that branch, whatever branch the session started on.
3. **Define checks.** For each Definition of done item, write at least one shell command that exits 0 only when the item holds; normally a test, written first. Run each: it should fail now. Add each to the state file:

   ```
   check: <command, run from the repository root>
   ```

   Show the checks. Do not wait for approval.
4. **Orchestrate.** You plan, delegate and verify; subagents do the work. Brief each with the issue, its items, the files and the checks it must pass.
   - **Explore:** subagents map the relevant code and spec clauses, in parallel.
   - **Test:** one subagent per item writes the failing test from the issue and the spec, never from the implementation.
   - **Implement:** a different subagent per item makes it pass. Bug: reproduce first, then fix. Run items in parallel when their files don't overlap.
   - **Review:** a fresh subagent reviews the diff against the issue and `CLAUDE.md`, and classifies the API change itself. Delegate the fixes and review again until clean.

   After every hand-back and after any change, run `sh .claude/skills/build/hook.sh check` from the repository root yourself; never trust a report.
5. **Open the pull request** once every check passes and the review is clean: commit, `git push -u origin HEAD`, fill in `.github/pull_request_template.md`, ready for review. Label it `api-additive` or `api-breaking`; no label means none. Then add `pr: <url>` to the state file.

## Rules

- **Decisions in the issue are constraints.** Record each in `.architecture-decisions/`. If one does not work, ask; never deviate.
- **Never make a hard-to-reverse decision yourself.** If the work forces one the issue does not cover, ask.
- **API change:** `none`, `additive` or `breaking`. Breaking means existing callers must change or get a different result, including different bytes written. Use it before 1.0 too. If the change is breaking and the issue does not say so, ask.
- **Push only the issue's branch.** Never push the branch the session started on, even if the session says to.
- **Never weaken a check.** Change one only if it was wrong, and say why in the pull request.
- **Ask, don't loop.** If you need something only the user has (a decision, a credential, access), or a check has failed three times and you have no new idea: set `status: blocked`, ask one precise question and stop. When they answer, set `status: working`.
- **Cancelled.** If the user cancels the build, delete `.claude/build/state`.
