"""Image download, OCR, and current-backend vision helpers."""

from __future__ import annotations

import asyncio
import base64
import hashlib
import time
from typing import Any

from socai.agent.backends import Backend

from .common import (
    MediaConfig,
    MediaUnavailable,
    detect_media_type,
    download_bytes,
    save_bytes,
    short,
    url_suffix,
)
from .timing import TimingRecord


class ImageProcessor:
    def __init__(
        self,
        config: MediaConfig,
        *,
        backend: Backend | None = None,
        timing: TimingRecord | None = None,
    ):
        self.config = config
        self.backend = backend
        self.timing = timing

    def _record(self, op: str, t0: float) -> None:
        if self.timing is not None:
            self.timing.record(op, time.perf_counter() - t0)

    def download_bytes(self, url: str, *, referer: str = "") -> bytes:
        t0 = time.perf_counter()
        try:
            return download_bytes(url, referer=referer, timeout=self.config.request_timeout_s)
        finally:
            self._record("image_download", t0)

    def save_bytes(self, payload: bytes, *, label: str, suffix: str = ".bin") -> str:
        return save_bytes(self.config.base_dir, payload, label=label, suffix=suffix)

    def ocr_image(self, payload: bytes) -> str:
        if not self.config.use_ocr:
            raise MediaUnavailable("OCR is disabled")
        if not payload:
            return ""
        t0 = time.perf_counter()
        try:
            try:
                import Foundation  # type: ignore
                import Vision  # type: ignore
            except Exception as exc:  # noqa: BLE001 - optional dependency
                raise MediaUnavailable("Apple Vision OCR is unavailable; install PyObjC Vision bindings") from exc

            data = Foundation.NSData.dataWithBytes_length_(payload, len(payload))
            handler = Vision.VNImageRequestHandler.alloc().initWithData_options_(data, {})
            request = Vision.VNRecognizeTextRequest.alloc().init()
            try:
                request.setRecognitionLanguages_(["zh-Hans", "zh-Hant", "en-US"])
                request.setRecognitionLevel_(Vision.VNRequestTextRecognitionLevelAccurate)
            except Exception:
                pass
            ok, error = handler.performRequests_error_([request], None)
            if not ok:
                raise RuntimeError(f"Apple Vision OCR failed: {error}")
            lines: list[str] = []
            for observation in request.results() or []:
                try:
                    candidates = observation.topCandidates_(1)
                    if candidates:
                        text = str(candidates[0].string() or "").strip()
                        if text:
                            lines.append(text)
                except Exception:
                    continue
            return "\n".join(lines)
        finally:
            self._record("ocr_image", t0)

    def describe_image(self, payload: bytes, prompt: str, *, max_tokens: int = 180) -> str:
        if not self.config.use_vision:
            raise MediaUnavailable("Vision is disabled")
        if self.backend is None:
            raise MediaUnavailable("No agent LLM backend was provided for image vision")
        if not payload:
            return ""

        t0 = time.perf_counter()
        try:
            media_type = detect_media_type(payload) or "image/jpeg"
            response = self.backend.create_message(
                system="You describe images concisely and only state visible evidence.",
                messages=[
                    {
                        "role": "user",
                        "content": [
                            {"type": "text", "text": prompt},
                            {
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": base64.b64encode(payload).decode("ascii"),
                                },
                            },
                        ],
                    }
                ],
                tools=[],
                max_tokens=max_tokens,
            )
            return "\n".join(response.text_blocks).strip()
        finally:
            self._record("vision_image", t0)

    async def enrich_images_async(
        self,
        images: list[dict[str, Any]],
        *,
        referer: str = "",
        label: str = "image",
        run_vision: bool = False,
        vision_concurrency: int | None = None,
    ) -> list[dict[str, Any]]:
        if not images:
            return []

        concurrency = max(1, int(vision_concurrency or self.config.vision_concurrency or 1))

        # Parallel downloads.
        t_batch = time.perf_counter()
        download_results = await asyncio.gather(
            *[
                asyncio.to_thread(self._safe_download, str(image.get("url") or ""), referer)
                for image in images
            ],
            return_exceptions=False,
        )
        if self.timing is not None:
            self.timing.record("image_download_batch", time.perf_counter() - t_batch)

        # Dedup + persist.
        deduped: list[tuple[dict[str, Any], bytes]] = []
        seen: set[str] = set()
        for image, (payload, error) in zip(images, download_results):
            if not str(image.get("url") or "").strip():
                continue
            item = dict(image)
            if error:
                item["download_error"] = error
                deduped.append((item, b""))
                continue
            if not payload:
                continue
            digest = hashlib.md5(payload).hexdigest()
            if digest in seen:
                continue
            seen.add(digest)
            item["local_path"] = self.save_bytes(
                payload,
                label=f"{label}_{len(deduped) + 1}",
                suffix=url_suffix(str(image.get("url") or ""), ".jpg"),
            )
            deduped.append((item, payload))

        # Concurrent OCR + vision (vision rate-limited by semaphore).
        semaphore = asyncio.Semaphore(concurrency)

        async def enrich_one(item: dict[str, Any], payload: bytes) -> None:
            if not payload:
                return
            if self.config.use_ocr and not item.get("ocr_text"):
                try:
                    ocr_text = await asyncio.to_thread(self.ocr_image, payload)
                    if ocr_text and ocr_text.strip():
                        item["ocr_text"] = short(ocr_text, 800)
                except Exception as exc:  # noqa: BLE001 - optional capability
                    item["ocr_error"] = str(exc)
            if run_vision and self.config.use_vision and not item.get("vision_description"):
                async with semaphore:
                    try:
                        item["vision_description"] = await asyncio.to_thread(
                            self.describe_image,
                            payload,
                            "Describe this Xiaohongshu image for the note. Focus on concrete visible facts.",
                        )
                    except Exception as exc:  # noqa: BLE001 - optional capability
                        item["vision_error"] = str(exc)

        t_enrich = time.perf_counter()
        await asyncio.gather(
            *[enrich_one(item, payload) for item, payload in deduped],
            return_exceptions=False,
        )
        if self.timing is not None:
            self.timing.record("image_enrich_batch", time.perf_counter() - t_enrich)

        return [item for item, _ in deduped]

    def _safe_download(self, url: str, referer: str) -> tuple[bytes, str]:
        if not url:
            return b"", ""
        try:
            return self.download_bytes(url, referer=referer), ""
        except Exception as exc:  # noqa: BLE001 - per-media best effort
            return b"", f"{type(exc).__name__}: {exc}"

    def diagnostics(self) -> dict[str, Any]:
        return {
            "apple_vision_ocr": self._module_importable("Vision") and self._module_importable("Foundation"),
            "agent_vision_backend": type(self.backend).__name__ if self.backend else "",
        }

    @staticmethod
    def _module_importable(name: str) -> bool:
        try:
            __import__(name)
            return True
        except Exception:
            return False
