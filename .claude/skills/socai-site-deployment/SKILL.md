---
name: socai-site-deployment
description: Deploy, configure, troubleshoot, and verify the socai marketing/download website on Vercel. Use for socai.io deployments, Vercel project settings, domain/redirect checks, Git preview setup, /download and /github redirect validation, and deployment runbooks for the site/ Astro app.
---

# socai site deployment

Use this skill whenever the task involves the socai website deployment, Vercel configuration, `socai.io`, `www.socai.io`, `/download`, `/github`, or PR preview deployments.

This skill is the source-of-truth deployment runbook. [`../../../docs/website-deployment.md`](../../../docs/website-deployment.md) intentionally stays brief and points back here to avoid duplicate instructions drifting.

## Project context

- Website source: `site/`
- Framework: Astro static site
- Vercel team/scope: `socai-d83824c8` (`socai`)
- Vercel project: `socai-site`
- Production domain: `https://socai.io`
- Canonical host: `socai.io`
- `www` behavior: `https://www.socai.io/*` should 308/redirect to `https://socai.io/*`
- Download route: `https://socai.io/download` redirects to GitHub's latest universal macOS DMG
- GitHub route: `https://socai.io/github` redirects to `https://github.com/tonyc-ship/socai`

Expected Vercel project settings:

| Setting | Value |
| --- | --- |
| Framework preset | Astro |
| Root directory | `site` |
| Install command | `pnpm install` |
| Build command | `pnpm build` |
| Output directory | `dist` |
| Node.js version | 24.x |

## Safety rules

- Use the `socai-d83824c8` scope for all Vercel project/domain commands.
- Do not deploy from the wrong Vercel team or a personal project.
- Do not commit `site/.vercel/`, `site/dist/`, `site/.astro/`, or `site/node_modules/`.
- Do not enable analytics unless the user explicitly approves it.
- Do not edit app release assets while doing website deployment work.
- Keep issue work on a dedicated branch and open/update a PR for repo changes.
- If you temporarily change Vercel project settings for a manual deploy, restore them before finishing.

## Local validation

Before deploying or opening a PR for deployment-related code/config:

```bash
cd site
pnpm install
pnpm build
```

Validate JSON/XML config when touched:

```bash
python3 -m json.tool site/vercel.json >/dev/null
python3 -m json.tool site/public/site.webmanifest >/dev/null
python3 - <<'PY'
import xml.etree.ElementTree as ET
ET.parse('site/public/sitemap.xml')
PY
```

## Inspect Vercel state

```bash
vercel whoami
vercel teams ls
vercel projects ls --scope socai-d83824c8
vercel project inspect socai-site --scope socai-d83824c8 --yes
vercel domains inspect socai.io --scope socai-d83824c8
vercel domains inspect www.socai.io --scope socai-d83824c8
```

Confirm project settings show root directory `site`, framework `Astro`, build command `pnpm build`, output `dist`, install `pnpm install`.

## Preferred production deployment

Preferred path after Git integration is enabled:

1. Merge website changes to `main`.
2. Let Vercel build the `socai-site` project from root directory `site`.
3. Verify production URLs with the commands below.

PR previews also require Git integration to be connected in the Vercel dashboard.

## Emergency/manual CLI production deployment

Because the Vercel project is configured with root directory `site`, a direct local deploy from `site/` can fail with `site/site` root-directory errors. If Git integration is not available and a CLI deploy is required, use this temporary-root workaround and restore root directory afterward.

```bash
set -euo pipefail

cat > /tmp/vercel-root-null.json <<'EOF'
{
  "rootDirectory": null
}
EOF
cat > /tmp/vercel-root-site.json <<'EOF'
{
  "rootDirectory": "site"
}
EOF

restore_root() {
  vercel api /v10/projects/socai-site \
    --scope socai-d83824c8 \
    -X PATCH \
    --input /tmp/vercel-root-site.json \
    --silent || true
}
trap restore_root EXIT

vercel api /v10/projects/socai-site \
  --scope socai-d83824c8 \
  -X PATCH \
  --input /tmp/vercel-root-null.json \
  --silent

cd site
vercel deploy --prod --scope socai-d83824c8 --project socai-site --yes
```

After the deploy, confirm the root directory was restored:

```bash
vercel project inspect socai-site --scope socai-d83824c8 --yes
```

Do not leave the project with `Root Directory .` or `null`; Git previews depend on `site`.

## URL verification

Use HEAD checks first:

```bash
curl -I https://socai.io/
curl -I https://www.socai.io/
curl -I https://socai.io/download
curl -I https://socai.io/github
```

Expected first-hop behavior:

- `https://socai.io/` returns `200`.
- `https://www.socai.io/` returns `308` with `location: https://socai.io/`.
- `https://socai.io/download` returns a Vercel redirect (`307`) to `https://github.com/tonyc-ship/socai/releases/latest/download/socai-macos-universal.dmg`.
- `https://socai.io/github` returns a Vercel redirect (`307`) to `https://github.com/tonyc-ship/socai`.

Use `-L` only when you want to follow the chain:

```bash
curl -I -L --max-time 30 -o /dev/null -w 'code=%{http_code}\nfinal=%{url_effective}\n' https://socai.io/download
```

## Git preview setup

Try:

```bash
cd site
vercel link --yes --scope socai-d83824c8 --project socai-site
vercel git connect https://github.com/tonyc-ship/socai --scope socai-d83824c8
```

If Vercel returns:

```text
Failed to link tonyc-ship/socai. You need to add a Login Connection to your GitHub account first.
```

then a Vercel/GitHub account owner must connect the GitHub login/integration in the Vercel dashboard, then connect `socai-site` to `tonyc-ship/socai` with root directory `site`.

## Troubleshooting

### `Root Directory "site" does not exist`

This usually happens when running local CLI deploy from `site/` while the remote project also has root directory `site`. Use the emergency/manual CLI deployment workaround above, or prefer Git deployments.

### Too many files uploaded

Deploying from the repo root can include Rust/Python build artifacts and exceed Vercel's file count. Do not deploy the repo root directly unless you have a tested ignore/archive strategy. Prefer Git deployments or the emergency/manual deploy from `site/` with temporary root reset.

### `www.socai.io` serves 200 instead of redirecting

Ensure the latest production deployment includes the host redirect in `site/vercel.json`, then redeploy. Verify with:

```bash
curl -I https://www.socai.io/
```

### GitHub latest release metadata is unavailable

The website should still show generic non-stale copy (`latest release`) and `/download` should still redirect to GitHub's latest release asset.

## Reporting back

When reporting a deployment task, include:

- Branch/PR if repo files changed
- Vercel project settings confirmed
- Deployment URL and production domain
- Commands run
- Verification output summary for `/`, `www`, `/download`, and `/github`
- Any remaining manual blockers, especially Git integration / PR previews
