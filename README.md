# Socai

The web-use agent core.

## Setup

```bash
uv sync
```

## Live XHS CDP Smoke

```bash
uv run python scripts/xhs_cdp_smoke.py "伊兰特N"
```

The smoke script reuses your existing logged-in Chrome profile. If Chrome opens `chrome://inspect/#remote-debugging`, approve remote debugging and rerun the command.
It always opens a new tab, searches, opens the first result, and prints extracted note details by default.
