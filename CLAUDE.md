# Rules

1. **Dependencies only for a strong reason.** Any crate, the core included, and build, test and CI tooling may use a dependency when writing it here would be large or risky to get wrong, or it is the standard tool for the job. It must have a permissive license and pass the checks: license allow list in `deny.toml`, a reason comment in the manifest, a source audit in `supply-chain/`, no advisories. Otherwise write it or ask.
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
- When a decision changes, rewrite its file; when it no longer applies, delete it. Never add a superseding one.
- Record only decisions that are hard to reverse, surprising without context and a real trade-off.

# Testing

- **Test what matters.** Add a test only where it guards behaviour that could break unnoticed; one good test beats many. Never add a test that repeats another or checks wording. A bug fix in code starts with a test that fails without the fix.
- **Public API first.** Test behaviour through the public API; test internals directly only where a standard publishes test vectors.
- **Cite the spec.** Where a test verifies a standard, name the standard and clause.
- **Round-trip.** Whatever the library writes, it must read back with the same content.
- **Corpus.** Valid and malformed test files live in `tests/corpus/`. Every file is made by this project; never add a file created by someone else.
- **Independent expectations.** Expected values come from a standard or from reasoning that shares no code with the library; never copy them from its output.
- **Fuzz every parser.** Untrusted input must never crash, hang or exhaust memory. Every finding becomes a corpus file.
- **Deterministic.** No network, clock, randomness or dependence on test order.
- **One conformance suite.** Shared cases are data files that every language binding runs.
- **Never weaken a test.** Don't skip, loosen or delete a test to make it pass; fix the code.
- **Changed code only.** CI runs a test or check only when the pull request changes code it covers or depends on.
- **Rules 1 and 2 apply.** Test code follows the same dependency rule and passes the same checks.

# Releases

- **Trunk-based.** `main` is the only long-lived branch.
- **Nightly.** Automation tags `main` as `nightly-YYYY-MM-DD` at 22:00 UTC.
- **Release.** Only a human promotes a nightly to `vX.Y.Z`.
