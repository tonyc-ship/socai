"""Shared helpers for optional local/media processing."""

from __future__ import annotations

import hashlib
import mimetypes
import subprocess
import urllib.request
from dataclasses import dataclass
from pathlib import Path


class MediaUnavailable(RuntimeError):
    """Raised when an optional local/media capability is not configured."""


USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123 Safari/537.36"
)


@dataclass
class MediaConfig:
    base_dir: Path
    request_timeout_s: int = 25
    ffmpeg_timeout_s: int = 180
    whisper_timeout_s: int = 300
    max_audio_seconds: int = 90
    max_frame_seconds: int = 60
    default_language: str = "zh"
    use_ocr: bool = True
    use_vision: bool = True
    use_whisper: bool = True
    use_ffmpeg: bool = True
    vision_concurrency: int = 3


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def short(text: str, max_chars: int = 1200) -> str:
    value = str(text or "").strip()
    return value if len(value) <= max_chars else value[:max_chars] + "... [truncated]"


def url_suffix(url: str, fallback: str = ".bin") -> str:
    suffix = Path(str(url).split("?", 1)[0]).suffix.lower()
    if suffix and len(suffix) <= 8:
        return suffix
    return fallback


def run_command(cmd: list[str], *, timeout: int, cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        check=True,
        text=True,
        capture_output=True,
        timeout=timeout,
    )


def detect_media_type(payload: bytes, *, fallback: str = "application/octet-stream") -> str:
    if payload.startswith(b"\xff\xd8"):
        return "image/jpeg"
    if payload.startswith(b"\x89PNG"):
        return "image/png"
    if payload.startswith(b"RIFF") and payload[8:12] == b"WEBP":
        return "image/webp"
    guessed = mimetypes.guess_type("media.bin")[0]
    return guessed or fallback


def download_bytes(url: str, *, referer: str = "", timeout: int = 25) -> bytes:
    target = str(url or "").strip()
    if not target:
        return b""
    request = urllib.request.Request(
        target,
        headers={
            "User-Agent": USER_AGENT,
            **({"Referer": referer} if referer else {}),
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def save_bytes(base_dir: Path, payload: bytes, *, label: str, suffix: str = ".bin") -> str:
    digest = hashlib.md5(payload).hexdigest()[:10]
    safe_label = "".join(ch if ch.isalnum() or ch in {"_", "-"} else "_" for ch in label).strip("_") or "media"
    path = ensure_dir(base_dir / safe_label) / f"{safe_label}_{digest}{suffix}"
    path.write_bytes(payload)
    return str(path)


def download_file(
    base_dir: Path,
    url: str,
    *,
    referer: str = "",
    label: str = "media",
    suffix: str = "",
    timeout: int = 25,
) -> str:
    payload = download_bytes(url, referer=referer, timeout=timeout)
    if not payload:
        return ""
    return save_bytes(base_dir, payload, label=label, suffix=suffix or url_suffix(url))
