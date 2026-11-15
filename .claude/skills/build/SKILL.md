---
name: build
description: Build a GitHub issue until its definition of done is met, then open a pull request and merge it if safe.
argument-hint: [issue number]
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

Build issue $ARGUMENTS. With a number, first run `sh .claude/skills/build/hook.sh start <number>`: on exit 3, ask whether to build it anyway, since another build may be working on it (recommend no), and stop on no. With no number, run `sh .claude/skills/build/hook.sh next` and build the issue it prints. If either exits otherwise non-zero, say so and stop. Do not stop until its Definition of done is met and the pull request is open, its CI is green, the retro comment is posted and the merge command has run.

A Stop hook sends you back unless every check passes on the current files and the open pull request's CI is green, its retro comment is recorded and the merge command has run on its head commit; no CI checks counts as green. After 24 hours without the user, tool calls are refused: stop and ask whether to continue.

1. **Read** the issue and `CLAUDE.md`. If the Goal or Definition of done is missing or not checkable, ask the user and update the issue. Then make sure it is labelled `build-ready` and `bug` or `feature`. Run `gh issue edit <number> --add-label building`.
2. **Define checks.** For each Definition of done item, write a shell command that exits 0 only when the item holds, and can do so while the build runs (the issue is labelled `building`): a test only where the item needs one and `CLAUDE.md` calls for it, otherwise a direct command. Write `.claude/build/state`:

   ```
   status: working
   issue: <number> <title>
   phase: explore
   check: <command, run from the repository root>
   ```

   Checks that read results of one test run share one command before the first ` | `, a plain one (words, paths and `2>&1` only), e.g. `<suite> 2>&1 | grep -qxF '<test>'`: `hook.sh check` has it run once and feeds its output to each check's filter. An item whose behaviour can only be observed after merge gets a static check; the pull request says it is verifiable only after merge, with the command to run then. Check a crates.io link with a user agent against `https://crates.io/api/v1/crates/<name>`: the page answers curl 403 or 404, the API 403 without one.

   Run `sh .claude/skills/build/hook.sh check`: every check should fail now.

   Keep `phase:` current: `explore`, `test`, `implement`, `review`, `pr`, `ci`, `retro`.

   Then create branch `<number>-<slug>`, at most 60 characters, from `origin/main`. If it already exists, check its pull request and commits; ask only if someone else may be working on it.

   Show the checks. Do not wait for approval.
3. **Orchestrate.** You plan, delegate and verify; subagents do the work. You read no source file, test file, diff or log yourself, in any step: a subagent reads them and reports what you need to plan, write the checks and verify. Brief each with the issue, its items, the files and the checks it must pass. Each report holds its result, what you must decide and the ideas it came across outside the issue (one line each, with what prompted it), never file contents or a diff. Note retro material as you go: what took long, failed checks, review misses, expensive calls, missing facts.
   - **Explore:** subagents map the relevant code and spec clauses, in parallel.
   - **Test:** where an item needs a test and `CLAUDE.md` calls for it, a subagent writes it, failing, from the issue and the spec, never from the implementation, and none for behaviour the issue puts out of scope.
   - **Implement:** a different subagent per item makes its check pass by the simplest means that fully meets it: instructions or configuration before code, existing code before new. Add nothing that no item needs: no extra option, abstraction, fallback, check or test. Bug: reproduce first, then fix. Run items in parallel when their files don't overlap.
   - **Review:** a fresh subagent reviews the diff (`git diff $(git merge-base HEAD origin/main)`, plus untracked files) against the issue and `CLAUDE.md`, and classifies the API change itself. It traces every file, function and test the diff adds to an item, and asks for the rest to be removed or simplified, for tests that repeat another to be removed, for each limit an item sets to be tried with the input that fits it worst (a width with one unbreakable word), and for every documentation file and code comment the diff adds or changes to be merged with an existing one on the same topic, deleted if unneeded, or shortened, where it can be; such a fix leaves no file that repeats another. Delegate the fixes and review again until clean.

   While a subagent runs, end your turn and never poll: the Stop hook lets a stop through.

   After every hand-back and after any change, run `sh .claude/skills/build/hook.sh check` from the repository root yourself; never trust a report. It names each failing check; a subagent reruns it and fixes the cause.
4. **Open the pull request** once every check passes and the review is clean: a subagent commits, pushes and fills in `.github/pull_request_template.md`, its API and Dependencies lines included, which read exactly `- **API:** none` and `- **Dependencies:** none` when there is none, ready for review. Label it `api-additive` or `api-breaking`; no label means none. Label it `adds-dependency` if it adds one. Then add `pr: <url>` to the state file.
5. **Follow CI.** Set `phase: ci` and run `gh pr checks <url> --watch >/dev/null; gh pr checks <url>`. If a CI check fails, a subagent reads its log (`gh run view --log-failed`), fixes the cause and pushes; follow the new run.
6. **Retro.** Once CI is green, set `phase: retro`. This step changes no file but the state file and pushes no commit. Post one comment on the pull request with `gh pr comment`, then add `comment: <url>` to the state file. Its sections:
   - `Ideas for future issues`: what the build's agents came across outside the issue: a bug, a missing test, a simplification or a feature. Each is one line that states the outcome, ready to pass to `/issue`, and names what prompted it. Leave out an idea that an open issue already covers (check with `gh issue list`).
   - `Retro`: suggested changes to the agents' environment (skills, `CLAUDE.md`, checks, hooks, tools), never to the product, most severe first. Each names what happened in this build that prompted it, and is one of: navigation (something took long to find), automated check (a mistake a check could have caught; suggest it as a check, never as an instruction), review (a mistake the review missed), steering (an instruction that changed nothing, or belongs in a check), tool economy (an expensive tool call), information access (a fact the agents could not get).

   A section with nothing to report reads `Nothing.`
7. **Merge.** A subagent that took no part in the build reads the diff, the issue and the pull request body and answers `safe` or `unsafe` with one reason: `unsafe` when in doubt, and always when the body says a check was changed, an item is verifiable only after merge or `Not done` is not `Nothing.` Add `verdict: safe|unsafe <sha> <reason>` to the state file, `<sha>` from `git rev-parse HEAD`, and run `sh .claude/skills/build/hook.sh merge`. If `up to date` fails, whichever other conditions failed, merge `origin/main` into the branch, run `sh .claude/skills/build/hook.sh check`, push, follow CI and ask for the verdict again, at most three times. Never otherwise change the pull request to make a condition hold.

   Your last message to the user says whether the pull request was merged and repeats the command's lines: after a merge, what each condition found; otherwise, each failed condition.

## Rules

- **Decisions in the issue are constraints.** Record one in `.architecture-decisions/` only if it is hard to reverse, surprising without context and a real trade-off; follow the rest without recording them. If one does not work, ask; never deviate.
- **Questions.** Ask only for a fact you cannot find in the repository, its issues or its pull requests, or a choice that is hard to reverse, surprising without context and a real trade-off. Make every other choice yourself. Recommend an answer to every question, multi-select included. Before you ask, have a subagent look up again every fact the question depends on.
- **Dependencies:** one the issue does not name is added without asking when Rule 1 is met. Where no imported audit covers it, a subagent reviews its source and runs `cargo vet certify`; never an exemption.
- **API change:** `none`, `additive` or `breaking`. Breaking means existing callers must change or get a different result, including different bytes written. Use it before 1.0 too. If the change is breaking and the issue does not say so, ask.
- **Never weaken a check.** Change one only if it was wrong, and say why in the pull request.
- **Ask, don't loop.** If you must ask, or a check, or the same CI check, has failed three times and you have no new idea: set `status: blocked`, ask one precise question and stop. When they answer, set `status: working`.
- **Cancelled.** If the user cancels the build, run `gh issue edit <number> --remove-label building`, then delete `.claude/build/state`.
