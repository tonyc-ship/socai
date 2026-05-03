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

For the app/backend path, keep one `BrowserTaskSessionManager` alive for the process lifetime and create one task per user request. That reuses the same CDP socket and opens a fresh tab for each task instead of reconnecting to Chrome each time.
