"""Optional local/media processing used by site runtimes.

The browser and agent core do not depend on OCR, whisper, or ffmpeg. Site
runtimes can opt into this package for heavier media enrichment while keeping
plain DOM extraction fast and portable.
"""

from .common import MediaConfig, MediaUnavailable
from .processor import MediaProcessor
from .timing import TimingRecord

__all__ = ["MediaConfig", "MediaProcessor", "MediaUnavailable", "TimingRecord"]
