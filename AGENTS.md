# socai agent notes

The repo has two halves: a Python agent package (`socai/`) and a Tauri 2
desktop app (`app/`). The desktop app surfaces the agent to users.

## Python package — `socai/`

- `socai/agent/`: generic agent loop, LLM backends, run state, tool interface.
- `socai/browser/cdp/`: CDP endpoint discovery, long-lived browser session,
  page primitives, task tabs.
- `socai/browser/tools/`: generic browser tools built on CDP page sessions.
- `socai/sites/xhs/`: Xiaohongshu entities, JS extractors, runtime, site tools.
- `socai/cli/`: entry point `uv run socai`. Submodules:
  - `commands`: argparse dispatcher (`socai search_notes` / `topic_scan` /
    `extract_note` / `stop`). No-args path falls through to the REPL.
  - `repl`: interactive prompt-toolkit UI.
  - `runner`: headless task runner (reused by REPL and the future Tauri host).
  - `daemon` + `daemon_client`: long-lived process owning one browser "tool
    tab" reused across CLI calls, exposed over `~/.socai/daemon.sock`.
    Auto-spawns on first tool call, auto-shuts after 3h idle.

Rules:

- Keep one `BrowserTaskSessionManager` alive per app/CLI process; create a new
  task tab per user task.
- Keep JS extractors in a small JSON-returning contract; Python injects, calls,
  and validates results.
- Tool subcommands wrap existing `XhsRuntime` / site tools — don't duplicate
  XHS logic in the daemon. Any cleanups to the public data shape go in
  `sites/xhs/entities.py` and `sites/xhs/tools.py`, not in the daemon layer.

## Desktop app — `app/`

Stack: Tauri 2.11 (Rust shell) + Vite 6 + vanilla TypeScript (no UI framework).
Bundle identifier `com.socai.app`. Product name lowercase `socai`.

Layout:

- `app/src/`: frontend — `main.ts`, `styles.css`, `assets/`.
- `app/src-tauri/`: Rust shell — `lib.rs`, `tauri.conf.json`, `capabilities/`, `icons/`.
- `app/branding/`: icon source-of-truth — `app-icon.svg` + rasterized `app-icon.png`.

Dev and build (run from `app/`):

```bash
pnpm install                          # one-time
pnpm exec tauri dev                   # daily dev loop (Vite HMR + Rust hot recompile)
pnpm exec tauri build --bundles app   # → target/release/bundle/macos/socai.app
```

Rules:

- **Brand is always lowercase `socai`** — productName, window title, hero text,
  error strings, comments. No Title Case anywhere.
- **Design system is monochrome.** Use tokens from `app/src/styles.css`
  (`--ink-0..9`, `--canvas`, `--fg`, `--line`, etc.). **No accent colors.**
  Status is filled vs hollow, never hue.
- **Hairlines, not shadows.** `--line` (#e5e5e5) carries all structural
  separation. `--shadow-pop` is reserved for popovers only.
- **Use the type-scale classes** — `.t-display`, `.t-h1`, `.t-h2`, `.t-h3`,
  `.t-lede`, `.t-body`, `.t-small`, `.t-eyebrow` (mono uppercase), `.t-mono`.
  Don't reinvent.
- **`tauri` (Rust) and `@tauri-apps/api` (npm) must share major/minor.**
  Bumping one requires bumping the other in the same commit; Tauri CLI hard-
  fails on minor drift.

Regenerating the app icon — edit `branding/app-icon.svg`, then from `app/`:

```bash
rsvg-convert branding/app-icon.svg -w 1024 -h 1024 -o branding/app-icon.png
pnpm exec tauri icon branding/app-icon.png
```

Both steps are deterministic. Commit the changed files in `src-tauri/icons/`.
The mobile / Microsoft Store fan-out emitted by `tauri icon` is gitignored —
regenerate on demand if a mobile target is ever added.

Gotchas:

- macOS LaunchServices caches icons aggressively. After a rebuild that changes
  the icon, run `killall Dock; killall Finder` to flush. If the bundle
  identifier or productName changed, also run
  `/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister -kill -r -domain local -domain system -domain user`.
- Fonts (General Sans, Geist Mono) load via Fontshare / Google Fonts CDNs at
  runtime. For offline-capable production builds, bundle `.woff2` and replace
  the two `@import` rules at the top of `app/src/styles.css`.
- The Vite dev server ignores `src-tauri/**` (see `vite.config.ts`) so Rust
  file changes don't cause spurious frontend reloads. Rust edits trigger a
  full Tauri shell restart instead.
