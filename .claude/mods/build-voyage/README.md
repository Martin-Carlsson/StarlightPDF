# build-voyage

A Claude Code mod: a mission-control pane for a `/build` run. It reads `.claude/build/` and shows the issue, the flight stage, each check as a star, the time left before the 24-hour check-in and the refused stops.

Install, at the prompt of a terminal session:

```
/plugin install build-voyage --marketplace Martin-Carlsson/StarlightPDF
```

Open the pane with `/voyage`. It opens by itself when a build starts.

Test: `claude plugin test .claude/mods/build-voyage`
