# Socai Agent Notes

## Structure

- `socai/agent/`: generic agent loop, LLM backends, run state, and tool interface.
- `socai/browser/cdp/`: CDP endpoint discovery, long-lived browser session, page primitives, and task tabs.
- `socai/browser/tools/`: generic browser tools built on CDP page sessions.
- `socai/sites/xhs/`: Xiaohongshu entities, JS extractors, runtime, and site tools.
- `socai/cli.py`: minimal interactive CLI that runs the agent loop with browser/site tools.

## Rules

- Keep one `BrowserTaskSessionManager` alive per app/CLI process and create a new task tab per user task.
- Keep JS extractors in a small JSON-returning contract; Python injects, calls, and validates results.
