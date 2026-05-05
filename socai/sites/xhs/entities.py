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
    author_id: str = ""
    author_url: str = ""
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
            "author_id": self.author_id,
            "author_url": normalize_url(self.author_url),
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
    type: str = ""
    title: str = ""
    author: str = ""
    author_id: str = ""
    author_url: str = ""
    content: str = ""
    content_source: str = ""
    hashtags: list[str] = field(default_factory=list)
    date: str = ""
    location: str = ""
    ip_location: str = ""
    likes: str = ""
    favorites: str = ""
    comments_count: str = ""
    shares: str = ""
    image_count: int = 0
    images: list[dict] = field(default_factory=list)
    video: dict = field(default_factory=dict)
    extraction_level: str = "lite"
    stale_warning: str = ""
    wait_meta: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        payload: dict = {
            "note_id": self.note_id,
            "url": normalize_url(self.url),
            "type": self.type,
            "title": self.title,
            "author": self.author,
            "author_id": self.author_id,
            "author_url": normalize_url(self.author_url),
            "content": self.content,
            "content_summary": self.content[:500],
            "content_source": self.content_source,
            "hashtags": self.hashtags[:12],
            "date": self.date,
            "location": self.location,
            "ip_location": self.ip_location,
            "likes": self.likes,
            "likes_value": parse_count_text(self.likes),
            "favorites": self.favorites,
            "favorites_value": parse_count_text(self.favorites),
            "comments_count": self.comments_count,
            "comments_count_value": parse_count_text(self.comments_count),
            "shares": self.shares,
            "shares_value": parse_count_text(self.shares),
            "image_count": self.image_count or len(self.images),
            "images": self.images,
            "video": self.video,
            "extraction_level": self.extraction_level,
        }
        if self.wait_meta:
            payload["wait"] = self.wait_meta
        if self.stale_warning:
            payload["stale_warning"] = self.stale_warning
        return payload


@dataclass
class XhsAuthorProfile:
    display_name: str = ""
    xhs_id: str = ""
    profile_url: str = ""
    bio: str = ""
    followers: str = ""
    following: str = ""
    likes_and_collections: str = ""
    note_cards: list[XhsNoteCard] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "entity_type": "author",
            "display_name": self.display_name,
            "title": self.display_name,
            "xhs_id": self.xhs_id,
            "profile_url": normalize_url(self.profile_url),
            "url": normalize_url(self.profile_url),
            "bio": self.bio,
            "followers": self.followers,
            "followers_value": parse_count_text(self.followers),
            "following": self.following,
            "following_value": parse_count_text(self.following),
            "likes_and_collections": self.likes_and_collections,
            "likes_and_collections_value": parse_count_text(self.likes_and_collections),
            "note_count": len(self.note_cards),
            "note_cards": [card.to_dict() for card in self.note_cards],
        }
