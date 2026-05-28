# Website deployment

The Socai marketing/download site lives in [`site/`](../site/) and is deployed to Vercel.

For agent workflows, Pi also has a project skill at [`.pi/skills/socai-site-deployment/SKILL.md`](../.pi/skills/socai-site-deployment/SKILL.md).

## Vercel project

- Team/scope: `socai-d83824c8` (`socai`)
- Project: `socai-site`
- Production domain: `https://socai.io`
- `www` behavior: `https://www.socai.io/*` redirects permanently to `https://socai.io/*`

Project settings:

| Setting | Value |
| --- | --- |
| Framework preset | Astro |
| Root directory | `site` |
| Install command | `pnpm install` |
| Build command | `pnpm build` |
| Output directory | `dist` |
| Node.js version | 24.x |

The project uses [`site/vercel.json`](../site/vercel.json) for download/GitHub redirects and the `www` canonical-host redirect.

## Domains

Both domains are attached to the `socai-site` Vercel project:

- `socai.io`
- `www.socai.io`

`socai.io` is registered at Vercel and uses Vercel nameservers, so no external DNS changes are required.

## Local validation

```bash
cd site
pnpm install
pnpm build
```

## Inspect Vercel state

```bash
vercel project inspect socai-site --scope socai-d83824c8 --yes
vercel domains inspect socai.io --scope socai-d83824c8
vercel domains inspect www.socai.io --scope socai-d83824c8
```

## Preferred production deploy

After Git integration is connected, prefer the normal Git flow:

1. Merge website changes to `main`.
2. Let Vercel build the `socai-site` project from root directory `site`.
3. Verify production URLs.

PR preview deployments also require the Vercel project to be connected to the GitHub repository.

## Emergency/manual production deploy

If Git integration is unavailable and a CLI deploy is required, temporarily clear the Vercel root directory, deploy from `site/`, and restore the root directory to `site` afterward.

This workaround is necessary because the Vercel project is configured with root directory `site`; running a local deploy from `site/` without temporarily clearing that setting can make Vercel look for `site/site`.

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

After deployment, confirm the root directory was restored:

```bash
vercel project inspect socai-site --scope socai-d83824c8 --yes
```

## Verify production

```bash
curl -I https://socai.io/
curl -I https://www.socai.io/
curl -I https://socai.io/download
curl -I https://socai.io/github
```

Expected first-hop behavior:

- `https://socai.io/` returns `200`.
- `https://www.socai.io/` redirects to `https://socai.io/`.
- `https://socai.io/download` redirects to the latest universal macOS DMG on GitHub Releases.
- `https://socai.io/github` redirects to `https://github.com/tonyc-ship/socai`.

## Git previews

Vercel preview deployments for pull requests require the `socai-site` project to be connected to the GitHub repository `tonyc-ship/socai` in Vercel.

Try:

```bash
cd site
vercel link --yes --scope socai-d83824c8 --project socai-site
vercel git connect https://github.com/tonyc-ship/socai --scope socai-d83824c8
```

If Vercel reports that a GitHub login connection is missing, open the Vercel dashboard and add/connect the GitHub account or installation first, then connect the project to the repository with root directory `site`.
