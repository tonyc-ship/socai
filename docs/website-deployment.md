# Website deployment

The socai marketing/download site lives in [`site/`](../site/) and is deployed to Vercel at [`https://socai.io`](https://socai.io).

The detailed deployment runbook is the shared project skill:

- [`.claude/skills/socai-site-deployment/SKILL.md`](../.claude/skills/socai-site-deployment/SKILL.md)

Claude Code can read that skill directly. Pi loads the same skill directory via [`.pi/settings.json`](../.pi/settings.json).

Use the skill for:

- Vercel project settings
- production deployment steps
- `www.socai.io` canonical redirect behavior
- `/download` and `/github` redirect verification
- Git preview setup
- troubleshooting deployment failures

Keeping the runbook in one place avoids drift between docs and agent instructions.
