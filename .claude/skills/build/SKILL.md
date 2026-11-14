---
name: build
description: Build a GitHub issue until its definition of done is met, then open a pull request.
argument-hint: <issue number>
disable-model-invocation: true
hooks:
  Stop:
    - hooks:
        - type: command
          command: sh "$CLAUDE_PROJECT_DIR/.claude/skills/build/stop-check.sh"
---

Build issue $ARGUMENTS. Do not stop until its Definition of done is met and the pull request is open.

A Stop hook reruns your checks whenever you try to stop and sends you back if one fails.

1. **Read** the issue and `CLAUDE.md`. If the Goal or Definition of done is missing or not checkable, ask the user, update the issue, then continue.
2. **Define checks.** For each Definition of done item, write at least one shell command that exits 0 only when the item holds; normally a test, written first. Run each: it should fail now. Write `.claude/build/state`:

   ```
   status: working
   check: <command, run from the repository root>
   ```

   Show the checks. Do not wait for approval.
3. **Work.** Bug: reproduce it with a failing test, then fix. Feature: tests first.
4. **Open the pull request** once every check passes: commit, push, fill in `.github/pull_request_template.md`, ready for review. Then add `pr: <url>` to the state file.

## Rules

- **Never weaken a check.** Change one only if it was wrong, and say why in the pull request.
- **Ask, don't loop.** If you need something only the user has (a decision, a credential, access), or a check has failed three times and you have no new idea: set `status: blocked`, ask one precise question and stop. When they answer, set `status: working`.
