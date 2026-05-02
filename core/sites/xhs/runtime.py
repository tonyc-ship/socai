"""Minimal Xiaohongshu runtime on top of CDP PageSession."""

from __future__ import annotations

import asyncio
import re
from urllib.parse import quote

from core.browser.cdp.page import PageSession

from .entities import XhsNote, XhsNoteCard


XHS_HOME_URL = "https://www.xiaohongshu.com/explore"


SEARCH_CARDS_JS = r"""
return (() => {
  const text = (el) => (el ? (el.innerText || el.textContent || '').trim() : '');
  const fromState = [];
  try {
    const feeds = window.__INITIAL_STATE__?.search?.feeds?._value || [];
    for (let i = 0; i < feeds.length; i++) {
      const item = feeds[i] || {};
      const card = item.noteCard || item.note_card || null;
      if (!card) continue;
      const id = item.id || card.id || card.noteId || '';
      const token = item.xsecToken || item.xsec_token || '';
      fromState.push({
        note_id: id,
        title: card.displayTitle || card.title || '',
        author: card.user?.nickname || card.user?.nickName || '',
        likes: String(card.interactInfo?.likedCount || card.interactInfo?.likes || ''),
        cover_url: card.cover?.urlDefault || card.cover?.urlPre || '',
        type: card.type || '',
        position: i,
        xsec_token: token,
        link: id && token
          ? `https://www.xiaohongshu.com/explore/${id}?xsec_token=${encodeURIComponent(token)}&xsec_source=pc_search`
          : (id ? `https://www.xiaohongshu.com/explore/${id}` : '')
      });
    }
  } catch (e) {}
  if (fromState.length) return fromState;

  const cards = Array.from(document.querySelectorAll('section.note-item, [data-note-id], .feeds-page .note-item'));
  return cards.map((card, i) => {
    const linkEl = card.querySelector('a[href*="/explore/"], a[href*="/search_result/"]') || card.closest('a') || card.querySelector('a');
    const link = linkEl ? linkEl.href : '';
    const idMatch = link.match(/\/(?:explore|search_result|discovery)\/([^/?#]+)/);
    const noteId = card.dataset?.noteId || (idMatch ? idMatch[1] : '');
    return {
      note_id: noteId,
      title: text(card.querySelector('.title, .note-title, a.title span')),
      author: text(card.querySelector('.author-wrapper .name, .author .name, .nick-name')),
      likes: text(card.querySelector('.like-wrapper .count, .engagement .like .count, .count')),
      cover_url: card.querySelector('.cover img, .note-cover img, img')?.src || '',
      type: card.querySelector('video, .play-icon, .video-icon, svg[class*="video"], .duration') ? 'video' : 'image',
      position: i,
      xsec_token: '',
      link
    };
  }).filter((card) => card.note_id || card.title || card.link);
})();
"""


NOTE_JS = r"""
return (() => {
  const norm = (s) => String(s || '').replace(/\s+\n/g, '\n').replace(/\n\s+/g, '\n').trim();
  const text = (el) => norm(el ? (el.innerText || el.textContent || '') : '');
  const first = (selectors) => {
    for (const sel of selectors) {
      const el = document.querySelector(sel);
      const value = text(el);
      if (value) return value;
    }
    return '';
  };
  const url = location.href;
  const idMatch = url.match(/\/(?:explore|search_result|discovery)\/([^/?#]+)/);
  const raw = norm(document.body ? document.body.innerText || '' : '');
  const title = first(['#detail-title', '.note-content .title', '.note-scroller .title', '.note-detail .title', 'h1']);
  const author = first(['.author-container .username', '.author-wrapper .username', '.info .username', '.user-name']);
  const content = first([
    '#detail-desc .note-text',
    '#detail-desc',
    '.note-content .note-text',
    '.note-scroller .note-text',
    '.note-content .desc',
    '.note-scroller .desc'
  ]);
  const hashtags = Array.from(document.querySelectorAll('.hash-tag a, a[href*="/page/topics/"], #detail-desc a.tag'))
    .map((el) => text(el)).filter(Boolean);
  return {
    note_id: idMatch ? idMatch[1] : '',
    url,
    title,
    author,
    content: content || raw.slice(0, 2000),
    hashtags,
    likes: first(['.like-wrapper .count', '.engage-bar .like .count', '[data-type="like"] .count']),
    favorites: first(['.collect-wrapper .count', '.engage-bar .collect .count', '[data-type="collect"] .count']),
    comments_count: first(['.chat-wrapper .count', '.engage-bar .chat .count', '[data-type="chat"] .count']),
    raw_text_excerpt: raw.slice(0, 2000)
  };
})();
"""


class XhsRuntime:
    """Site-aware operations for Xiaohongshu."""

    def __init__(self, page: PageSession):
        self.page = page

    async def current_url(self) -> str:
        info = await self.page.page_info()
        return str(info.get("url") or "")

    async def ensure_xhs(self, *, navigate_if_needed: bool = False) -> None:
        url = await self.current_url()
        if "xiaohongshu.com" in url:
            return
        if navigate_if_needed:
            await self.page.navigate(XHS_HOME_URL)
            return
        raise RuntimeError(f"Current page is not Xiaohongshu: {url or 'unknown'}")

    async def search_notes(self, query: str, *, wait_seconds: float = 2.0) -> dict:
        keyword = str(query or "").strip()
        if not keyword:
            raise ValueError("query is required")
        url = f"https://www.xiaohongshu.com/search_result?keyword={quote(keyword)}&source=web_explore_feed"
        await self.page.navigate(url)
        if wait_seconds > 0:
            await asyncio.sleep(min(float(wait_seconds), 6.0))
        cards = await self.extract_search_cards()
        return {
            "ok": True,
            "query": keyword,
            "url": await self.current_url(),
            "count": len(cards),
            "cards": [card.to_dict() for card in cards],
        }

    async def extract_search_cards(self) -> list[XhsNoteCard]:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.page.evaluate(SEARCH_CARDS_JS)
        if not isinstance(raw, list):
            return []
        cards: list[XhsNoteCard] = []
        for index, item in enumerate(raw):
            if not isinstance(item, dict):
                continue
            cards.append(
                XhsNoteCard(
                    note_id=str(item.get("note_id") or ""),
                    title=str(item.get("title") or ""),
                    author=str(item.get("author") or ""),
                    likes=str(item.get("likes") or ""),
                    link=str(item.get("link") or ""),
                    cover_url=str(item.get("cover_url") or ""),
                    type=str(item.get("type") or ""),
                    position=int(item.get("position", index) or index),
                    xsec_token=str(item.get("xsec_token") or ""),
                )
            )
        return cards

    async def open_note(self, *, note_id: str = "", index: int | None = None) -> dict:
        cards = await self.extract_search_cards()
        selected: XhsNoteCard | None = None
        if note_id:
            selected = next((card for card in cards if card.note_id == note_id), None)
        if selected is None and index is not None and 0 <= int(index) < len(cards):
            selected = cards[int(index)]
        if selected is None:
            raise RuntimeError("Could not resolve note target from current search cards.")
        if not selected.link:
            raise RuntimeError(f"Selected note has no openable link: {selected.to_dict()}")
        await self.page.navigate(selected.link)
        return {"ok": True, "target": selected.to_dict(), "url": await self.current_url()}

    async def extract_note(self) -> XhsNote:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.page.evaluate(NOTE_JS)
        if not isinstance(raw, dict):
            raw = {}
        hashtags = raw.get("hashtags") if isinstance(raw.get("hashtags"), list) else []
        return XhsNote(
            note_id=str(raw.get("note_id") or ""),
            url=str(raw.get("url") or await self.current_url()),
            title=str(raw.get("title") or ""),
            author=str(raw.get("author") or ""),
            content=str(raw.get("content") or ""),
            hashtags=[str(tag) for tag in hashtags if str(tag).strip()],
            likes=str(raw.get("likes") or ""),
            favorites=str(raw.get("favorites") or ""),
            comments_count=str(raw.get("comments_count") or ""),
            raw_text_excerpt=str(raw.get("raw_text_excerpt") or ""),
        )

    async def read_note(self, *, note_id: str = "", index: int | None = None) -> dict:
        if note_id or index is not None:
            await self.open_note(note_id=note_id, index=index)
        note = await self.extract_note()
        return {"ok": True, "entity": note.to_dict()}


def extract_note_id_from_url(url: str) -> str:
    match = re.search(r"/(?:explore|search_result|discovery)/([^/?#]+)", str(url or ""))
    return match.group(1) if match else ""
