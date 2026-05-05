"""Progressive disclosure for site-specific toolkits."""

from __future__ import annotations

import json

from socai.agent.tool import Tool, ToolContext

from .knowledge import format_site_knowledge


SITE_TOOLKITS = {
    "xiaohongshu": {
        "display_name": "Xiaohongshu / 小红书 / XHS",
        "domains": ["xiaohongshu.com", "xhslink.com"],
        "when_to_use": "Search/read Xiaohongshu notes, comments, creator profiles, image/video evidence.",
        "coarse_tools": [
            "xhs_search_notes",
            "xhs_topic_scan",
            "xhs_read_note",
            "xhs_extract_profile",
        ],
    },
}


def _json(payload: dict) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2)


def site_catalog_prompt() -> str:
    items = []
    for site, info in SITE_TOOLKITS.items():
        items.append(
            f"- `{site}` ({info['display_name']}): {info['when_to_use']} "
            f"Coarse tools after enable: {', '.join(info['coarse_tools'])}."
        )
    return (
        "Site toolkit discovery:\n"
        "- Initially, only generic browser tools and `site_toolbox` are exposed.\n"
        "- If the task clearly needs a listed site, call `site_toolbox(site=...)` first.\n"
        "- After that call, the next model turn receives that site's concrete tool schemas.\n"
        + "\n".join(items)
    )


class SiteToolboxTool(Tool):
    name = "site_toolbox"
    description = (
        "List or enable site-specific browser toolkits. Call this when a task needs a known site "
        "such as Xiaohongshu; it returns the site knowledge and exposes concrete tools next turn."
    )

    @property
    def always_available(self) -> bool:
        return True

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "site": {
                    "type": "string",
                    "enum": ["xiaohongshu"],
                    "description": "Site toolkit to enable. Omit only to list available toolkits.",
                }
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        site = str(params.get("site") or "").strip().lower()
        if not site:
            return _json({"ok": True, "available_site_toolkits": SITE_TOOLKITS})
        info = SITE_TOOLKITS.get(site)
        if not info:
            return _json({"ok": False, "error": f"unknown_site:{site}", "available_site_toolkits": SITE_TOOLKITS})
        ctx.enabled_sites.add(site)
        return _json(
            {
                "ok": True,
                "enabled_site": site,
                "toolkit": info,
                "knowledge": format_site_knowledge(site),
                "next_turn": "Concrete site tool schemas are now enabled. Prefer them over generic browser tools on this site.",
            }
        )
