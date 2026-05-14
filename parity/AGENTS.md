# parity/ — agent workflow rules

These are durable rules for any future agent (Claude, Codex, etc.) running
the parity tests in this folder. The user does not want to copy-paste shell
output back and forth — the agent runs the commands and reports the diff.

## Run the commands yourself

When the user asks you to validate parity for a phase or a tool:

1. **Run the Rust + Python pair via Bash directly** — do not paste commands
   for the user to run.
2. **Save outputs under `parity/<scenario>/`** — never under `/tmp/`. The
   filename convention is `rust.json`, `python.json`, `diff.txt`. Stderr
   goes to `rust.stderr` / `python.stderr` so a failure is inspectable
   without re-running.
3. **Diff with `jq -S` canonicalization** before reporting. The diff itself
   goes to `<scenario>/diff.txt` and gets shown inline in the chat.
4. **Interpret the diff** before reporting. Volatile fields (`card_count`,
   feed listings, comment counts) are expected to drift between runs and
   are not bugs. Shape / type / key-name differences are bugs.

Example for the Phase-1 smoke test:

```bash
mkdir -p parity/phase1_smoke
cargo run --example eval_xhs_extractor -p socai-browser -- \
    <url> pageState > parity/phase1_smoke/rust.json 2>parity/phase1_smoke/rust.stderr
uv run python scripts/run_xhs_extractor.py \
    <url> pageState > parity/phase1_smoke/python.json 2>parity/phase1_smoke/python.stderr
diff <(jq -S . parity/phase1_smoke/rust.json) \
     <(jq -S . parity/phase1_smoke/python.json) \
    > parity/phase1_smoke/diff.txt 2>&1 || true
```

## Handle the Chrome remote-debugging approval popup

The first time anything connects to Chrome's CDP after a relaunch, Chrome
shows a native macOS dialog:

> "Google Chrome" wants to allow remote debugging.
> [Don't Allow]  [Allow]

This popup **blocks the underlying Bash command silently** — the agent
sees no failure signal, no hang indication; the Bash call just takes
however long the user takes to click. **Therefore: preemptively screenshot
before any CDP-using command**, do not wait for failure.

The popup is a native macOS dialog, not a webpage, so the `claude-in-chrome`
MCP screenshot tool can't see it (it's browser-scoped). Use macOS
system-level `screencapture` + the `Read` tool, and `osascript` to click.

Workflow:

1. **Before** running the first parity / CDP command of a session:

   ```bash
   screencapture -x parity/_screenshots/<scenario>.png
   ```

   Then `Read` the PNG with the standard Read tool. Look for the Chrome
   approval dialog ("Google Chrome wants to allow remote debugging" or the
   chrome://inspect "Allow" button if that page is in focus).

2. If a popup is visible, click "Allow" via AppleScript:

   ```bash
   osascript -e 'tell application "System Events" to tell process "Google Chrome" \
       to click button "Allow" of window 1'
   ```

   If clicking by name fails, fall back to clicking by coordinate (read the
   coords off the screenshot):

   ```bash
   osascript -e 'tell application "System Events" to click at {<x>, <y>}'
   ```

3. If Chrome isn't visible in the screenshot, it may be in a different Space
   or hidden:

   ```bash
   osascript -e 'tell application "Google Chrome" to activate'
   ```

   Then re-screenshot.

4. After clicking, proceed with the parity command via Bash. If a later
   command fails or behaves oddly, repeat the screenshot — Chrome re-prompts
   after profile changes or long idle periods.

If the popup does not appear and discovery still fails, the problem is
upstream (Chrome not launched with `--remote-debugging-port`, profile
locked, accessibility permission missing for the terminal, etc.) — at that
point ask the user.

> Accessibility permission: `osascript` system-level clicks require the
> terminal application running Claude Code (iTerm2 / Terminal.app) to have
> Accessibility permission in System Settings → Privacy & Security →
> Accessibility. If `osascript click at` produces no effect, that's the
> usual cause.

## What to commit, what to ignore

- **Commit** the scenario folder under `parity/` only when you intentionally
  want the captured JSON as a tracked fixture (e.g., a stable Phase-2
  per-tool expected output).
- **Do not commit** the runs from this Phase-1 smoke test — the outputs are
  homepage state with volatile counters. Add `parity/phase1_smoke/` to
  `.gitignore` (already done) or always run from a clean folder.
