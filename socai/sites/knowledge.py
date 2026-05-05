"""Markdown site knowledge loader."""

from __future__ import annotations

from pathlib import Path


_SITE_DIR = Path(__file__).parent
_KNOWLEDGE_FILES = {
    "xiaohongshu": _SITE_DIR / "xhs" / "knowledge.md",
}


def load_site_knowledge(site: str) -> str:
    path = _KNOWLEDGE_FILES.get(str(site or "").strip().lower())
    if not path or not path.exists():
        return ""
    return path.read_text(encoding="utf-8").strip()


def format_site_knowledge(site: str) -> str:
    body = load_site_knowledge(site)
    if not body:
        return ""
    return f"Site knowledge for `{site}`:\n\n{body}"
