"""Local audio transcription helpers."""

from __future__ import annotations

import os
import shutil
import time
from pathlib import Path
from typing import Any

from socai.agent.backends import Backend

from .common import MediaConfig, MediaUnavailable, download_file, ensure_dir, run_command, url_suffix
from .timing import TimingRecord


class AudioProcessor:
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

    def transcribe_audio(self, source: str, *, referer: str = "", language: str = "") -> str:
        t0 = time.perf_counter()
        try:
            return self._transcribe_local(source, referer=referer, language=language)
        finally:
            if self.timing is not None:
                self.timing.record("whisper_transcribe", time.perf_counter() - t0)

    def _transcribe_local(self, source: str, *, referer: str = "", language: str = "") -> str:
        if not self.config.use_whisper:
            raise MediaUnavailable("Whisper transcription is disabled")
        source_path = self._local_audio_source(source, referer=referer)
        mlx_model = os.environ.get("SOCAI_MLX_WHISPER_MODEL", "").strip()
        if mlx_model:
            try:
                import mlx_whisper  # type: ignore

                result = mlx_whisper.transcribe(
                    str(source_path),
                    path_or_hf_repo=mlx_model,
                    language=language or self.config.default_language,
                )
                return str(result.get("text") or "").strip()
            except Exception as exc:  # noqa: BLE001 - optional backend
                raise RuntimeError(f"mlx-whisper transcription failed: {exc}") from exc

        whisper_cli = shutil.which("whisper")
        if whisper_cli:
            out_dir = ensure_dir(self.config.base_dir / "transcripts")
            cmd = [
                whisper_cli,
                str(source_path),
                "--language",
                language or self.config.default_language,
                "--output_format",
                "txt",
                "--output_dir",
                str(out_dir),
            ]
            run_command(cmd, timeout=self.config.whisper_timeout_s)
            txt = out_dir / (source_path.stem + ".txt")
            return txt.read_text(encoding="utf-8").strip() if txt.exists() else ""

        whisper_cpp = shutil.which("whisper-cli") or shutil.which("main")
        whisper_model = os.environ.get("SOCAI_WHISPER_MODEL", "").strip()
        if whisper_cpp and whisper_model:
            wav = self.extract_audio_wav(source_path)
            out_prefix = self.config.base_dir / "transcripts" / source_path.stem
            ensure_dir(out_prefix.parent)
            cmd = [
                whisper_cpp,
                "-m",
                whisper_model,
                "-f",
                str(wav),
                "-l",
                language or self.config.default_language,
                "-otxt",
                "-of",
                str(out_prefix),
            ]
            run_command(cmd, timeout=self.config.whisper_timeout_s)
            txt = out_prefix.with_suffix(".txt")
            return txt.read_text(encoding="utf-8").strip() if txt.exists() else ""

        raise MediaUnavailable(
            "No whisper backend configured. Install `whisper`, or set SOCAI_MLX_WHISPER_MODEL, "
            "or set SOCAI_WHISPER_MODEL for whisper.cpp."
        )

    def _local_audio_source(self, source: str, *, referer: str = "") -> Path:
        value = str(source or "").strip()
        if not value:
            raise ValueError("audio source is required")
        if value.startswith(("http://", "https://")):
            saved = download_file(
                self.config.base_dir,
                value,
                referer=referer,
                label="audio",
                suffix=url_suffix(value, ".mp4"),
                timeout=self.config.request_timeout_s,
            )
            return Path(saved)
        return Path(value)

    def extract_audio_wav(self, source_path: Path) -> Path:
        if not shutil.which("ffmpeg"):
            raise MediaUnavailable("ffmpeg is required for whisper.cpp audio extraction")
        out_dir = ensure_dir(self.config.base_dir / "audio")
        out = out_dir / f"{source_path.stem}.wav"
        cmd = [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-t",
            str(self.config.max_audio_seconds),
            "-i",
            str(source_path),
            "-ar",
            "16000",
            "-ac",
            "1",
            "-y",
            str(out),
        ]
        run_command(cmd, timeout=self.config.ffmpeg_timeout_s)
        return out

    def diagnostics(self) -> dict[str, Any]:
        return {
            "ffmpeg": shutil.which("ffmpeg") or "",
            "whisper_cli": shutil.which("whisper") or "",
            "whisper_cpp": shutil.which("whisper-cli") or shutil.which("main") or "",
            "whisper_cpp_model": os.environ.get("SOCAI_WHISPER_MODEL", ""),
            "mlx_whisper_model": os.environ.get("SOCAI_MLX_WHISPER_MODEL", ""),
        }
