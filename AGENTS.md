# Socai Agent Notes

## Structure

- `core/agent/`: generic agent loop, LLM backends, run state, and tool interface.
- `core/browser/cdp/`: CDP endpoint discovery, long-lived browser session, page primitives, and task tabs.
- `core/browser/tools/`: generic browser tools built on CDP page sessions.
- `core/sites/xhs/`: Xiaohongshu entities, JS extractors, runtime, and site tools.
- `core/cli.py`: minimal interactive CLI that runs the agent loop with browser/site tools.

## Rules

- Keep one `BrowserTaskSessionManager` alive per app/CLI process and create a new task tab per user task.
- Keep JS extractors in a small JSON-returning contract; Python injects, calls, and validates results.
