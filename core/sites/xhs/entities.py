"""Structured Xiaohongshu entities used by the minimal site runtime."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from urllib.parse import urlsplit, urlunsplit


def parse_count_text(raw: str) -> int:
    value = str(raw or "").strip().lower().replace(",", "").replace("+", "")
    if not value:
        return 0
    match = re.search(r"(\d+(?:\.\d+)?)(万|w|k)?", value)
    if not match:
        return 0
    number = float(match.group(1))
    unit = (match.group(2) or "").lower()
    if unit in {"万", "w"}:
        number *= 10_000
    elif unit == "k":
        number *= 1_000
    return int(round(number))


def normalize_url(url: str) -> str:
    value = str(url or "").strip()
    if not value:
        return ""
    parts = urlsplit(value)
    return urlunsplit((parts.scheme, parts.netloc, parts.path, parts.query, ""))


@dataclass
class XhsNoteCard:
    note_id: str = ""
    title: str = ""
    author: str = ""
    likes: str = ""
    link: str = ""
    cover_url: str = ""
    type: str = ""
    position: int = 0
    xsec_token: str = ""

    @property
    def likes_value(self) -> int:
        return parse_count_text(self.likes)

    def to_dict(self) -> dict:
        return {
            "note_id": self.note_id,
            "title": self.title,
            "author": self.author,
            "likes": self.likes,
            "likes_value": self.likes_value,
            "link": self.link,
            "cover_url": self.cover_url,
            "type": self.type,
            "position": self.position,
            "xsec_token": self.xsec_token,
        }


@dataclass
class XhsNote:
    note_id: str = ""
    url: str = ""
    title: str = ""
    author: str = ""
    content: str = ""
    hashtags: list[str] = field(default_factory=list)
    likes: str = ""
    favorites: str = ""
    comments_count: str = ""
    raw_text_excerpt: str = ""

    def to_dict(self) -> dict:
        return {
            "note_id": self.note_id,
            "url": normalize_url(self.url),
            "title": self.title,
            "author": self.author,
            "content": self.content,
            "content_summary": self.content[:500],
            "hashtags": self.hashtags[:12],
            "likes": self.likes,
            "likes_value": parse_count_text(self.likes),
            "favorites": self.favorites,
            "favorites_value": parse_count_text(self.favorites),
            "comments_count": self.comments_count,
            "comments_count_value": parse_count_text(self.comments_count),
            "raw_text_excerpt": self.raw_text_excerpt,
        }
