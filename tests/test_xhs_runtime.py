from __future__ import annotations

import unittest

from core.sites.xhs import XhsRuntime
from core.sites.xhs.entities import parse_count_text


class FakeXhsPage:
    def __init__(self) -> None:
        self.url = "about:blank"
        self.navigated: list[str] = []

    async def navigate(self, url: str, **kwargs) -> dict:
        self.url = url
        self.navigated.append(url)
        return {"ok": True}

    async def page_info(self) -> dict:
        return {"url": self.url, "title": "XHS"}

    async def evaluate(self, expression: str, **kwargs):
        if "window.__INITIAL_STATE__" in expression:
            return [
                {
                    "note_id": "note-1",
                    "title": "尚酷 R",
                    "author": "tester",
                    "likes": "1.2万",
                    "link": "https://www.xiaohongshu.com/explore/note-1?xsec_token=token&xsec_source=pc_search",
                    "cover_url": "https://img.example/cover.jpg",
                    "type": "image",
                    "position": 0,
                    "xsec_token": "token",
                }
            ]
        if "#detail-title" in expression:
            return {
                "note_id": "note-1",
                "url": self.url,
                "title": "尚酷 R 体验",
                "author": "tester",
                "content": "一条笔记内容",
                "hashtags": ["#车"],
                "likes": "9",
                "favorites": "3",
                "comments_count": "2",
                "raw_text_excerpt": "一条笔记内容",
            }
        raise AssertionError("Unhandled XHS evaluation")


class XhsRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_search_notes_navigates_to_search_result_and_extracts_cards(self) -> None:
        page = FakeXhsPage()
        runtime = XhsRuntime(page)

        payload = await runtime.search_notes("尚酷 R", wait_seconds=0)

        self.assertTrue(payload["ok"])
        self.assertIn("keyword=%E5%B0%9A%E9%85%B7%20R", page.navigated[0])
        self.assertEqual(payload["count"], 1)
        self.assertEqual(payload["cards"][0]["note_id"], "note-1")
        self.assertEqual(payload["cards"][0]["likes_value"], 12000)

    async def test_read_note_opens_card_by_index_and_extracts_entity(self) -> None:
        page = FakeXhsPage()
        runtime = XhsRuntime(page)
        await runtime.search_notes("尚酷 R", wait_seconds=0)

        payload = await runtime.read_note(index=0)

        self.assertTrue(payload["ok"])
        self.assertEqual(page.navigated[-1], "https://www.xiaohongshu.com/explore/note-1?xsec_token=token&xsec_source=pc_search")
        self.assertEqual(payload["entity"]["note_id"], "note-1")
        self.assertEqual(payload["entity"]["title"], "尚酷 R 体验")
        self.assertEqual(payload["entity"]["hashtags"], ["#车"])

    def test_parse_count_text_supports_common_xhs_units(self) -> None:
        self.assertEqual(parse_count_text("1.2万"), 12000)
        self.assertEqual(parse_count_text("3k+"), 3000)
        self.assertEqual(parse_count_text(""), 0)


if __name__ == "__main__":
    unittest.main()
