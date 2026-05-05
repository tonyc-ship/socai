"""Video frame extraction and video-note enrichment."""

from __future__ import annotations

import asyncio
import hashlib
import shutil
import time
from pathlib import Path
from typing import Any

from .audio import AudioProcessor
from .common import MediaConfig, MediaUnavailable, USER_AGENT, ensure_dir, run_command, short, url_suffix
from .image import ImageProcessor
from .timing import TimingRecord


class VideoProcessor:
    def __init__(
        self,
        config: MediaConfig,
        *,
        images: ImageProcessor,
        audio: AudioProcessor,
        timing: TimingRecord | None = None,
    ):
        self.config = config
        self.images = images
        self.audio = audio
        self.timing = timing

    def _record(self, op: str, t0: float) -> None:
        if self.timing is not None:
            self.timing.record(op, time.perf_counter() - t0)

    def extract_video_frames(
        self,
        source: str,
        *,
        referer: str = "",
        max_seconds: int | None = None,
        num_frames: int = 4,
    ) -> list[str]:
        if not self.config.use_ffmpeg:
            raise MediaUnavailable("ffmpeg frame extraction is disabled")
        if not shutil.which("ffmpeg"):
            raise MediaUnavailable("ffmpeg is not installed or not on PATH")
        t0 = time.perf_counter()
        try:
            frame_dir = ensure_dir(self.config.base_dir / "frames" / hashlib.md5(str(source).encode()).hexdigest()[:10])
            pattern = str(frame_dir / "frame_%02d.jpg")
            safe_frames = max(1, int(num_frames or 1))
            safe_seconds = max(1, int(max_seconds or self.config.max_frame_seconds))
            interval = max(1, safe_seconds // safe_frames)
            cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error"]
            if referer and str(source).startswith(("http://", "https://")):
                cmd.extend(["-headers", f"Referer: {referer}\r\nUser-Agent: {USER_AGENT}\r\n"])
            cmd.extend(
                [
                    "-t",
                    str(safe_seconds),
                    "-i",
                    str(source),
                    "-vf",
                    f"fps=1/{interval},scale=min(960\\,iw):-2",
                    "-frames:v",
                    str(safe_frames),
                    pattern,
                ]
            )
            run_command(cmd, timeout=self.config.ffmpeg_timeout_s)
            return [str(path) for path in sorted(frame_dir.glob("frame_*.jpg"))]
        finally:
            self._record("video_frame_extract", t0)

    async def enrich_video_async(
        self,
        video: dict[str, Any],
        *,
        note_id: str = "",
        title: str = "",
        referer: str = "",
        max_frames: int = 4,
        run_vision: bool = False,
        vision_concurrency: int | None = None,
    ) -> dict[str, Any]:
        result = dict(video)
        source = str(result.get("resolved_url") or result.get("url") or "")
        poster_url = str(result.get("poster_url") or "")
        label = note_id or title or "video"
        concurrency = max(1, int(vision_concurrency or self.config.vision_concurrency or 1))

        async def poster_task() -> dict[str, Any]:
            if not poster_url:
                return {}
            out: dict[str, Any] = {}
            try:
                poster = await asyncio.to_thread(
                    self.images.download_bytes, poster_url, referer=referer
                )
                out["poster_local_path"] = self.images.save_bytes(
                    poster,
                    label=f"{label}_poster",
                    suffix=url_suffix(poster_url, ".jpg"),
                )
                if self.config.use_ocr:
                    try:
                        ocr = await asyncio.to_thread(self.images.ocr_image, poster)
                        out["poster_ocr"] = short(ocr, 800)
                    except Exception as exc:  # noqa: BLE001 - optional
                        out["poster_ocr_error"] = str(exc)
                if run_vision and self.config.use_vision:
                    try:
                        out["poster_description"] = await asyncio.to_thread(
                            self.images.describe_image,
                            poster,
                            f"Describe the poster image for Xiaohongshu video: {title}",
                        )
                    except Exception as exc:  # noqa: BLE001 - optional
                        out["poster_vision_error"] = str(exc)
            except Exception as exc:  # noqa: BLE001 - download fail
                out["poster_download_error"] = str(exc)
            return out

        async def transcript_task() -> dict[str, Any]:
            if not source:
                return {}
            try:
                transcript = await asyncio.to_thread(
                    self.audio.transcribe_audio, source, referer=referer
                )
                return {
                    "transcript": transcript,
                    "transcript_summary": short(transcript, 1200),
                }
            except Exception as exc:  # noqa: BLE001 - optional
                return {"transcript_error": str(exc)}

        async def frames_task() -> dict[str, Any]:
            if not source:
                return {}
            try:
                frame_paths = await asyncio.to_thread(
                    self.extract_video_frames, source, referer=referer, num_frames=max_frames
                )
            except Exception as exc:  # noqa: BLE001 - optional
                return {"frame_error": str(exc)}

            semaphore = asyncio.Semaphore(concurrency)

            async def describe_frame(frame_path: str) -> str:
                payload = await asyncio.to_thread(Path(frame_path).read_bytes)
                if run_vision and self.config.use_vision:
                    async with semaphore:
                        try:
                            t0 = time.perf_counter()
                            desc = await asyncio.to_thread(
                                self.images.describe_image,
                                payload,
                                f"Describe this sampled video frame for: {title}",
                            )
                            self._record("vision_video_frame", t0)
                            return desc
                        except Exception:
                            pass
                if self.config.use_ocr:
                    try:
                        return await asyncio.to_thread(self.images.ocr_image, payload)
                    except Exception:
                        return ""
                return ""

            descriptions = await asyncio.gather(*[describe_frame(p) for p in frame_paths])
            frame_notes = [d for d in descriptions if d]
            out: dict[str, Any] = {"frame_paths": frame_paths}
            if frame_notes:
                out["frame_descriptions"] = frame_notes
                out["visual_summary"] = short("\n".join(frame_notes), 1200)
            return out

        t_total = time.perf_counter()
        poster_out, transcript_out, frames_out = await asyncio.gather(
            poster_task(), transcript_task(), frames_task()
        )
        self._record("video_enrich_total", t_total)

        result.update(poster_out)
        result.update(transcript_out)
        result.update(frames_out)
        return result
