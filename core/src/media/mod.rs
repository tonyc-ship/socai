//! Optional local/media processing used by site runtimes.
//!
//! The browser and agent core do not depend on OCR, whisper, or ffmpeg.
//! Site runtimes can opt into this crate for heavier media enrichment while
//! keeping plain DOM extraction fast and portable.

mod audio;
mod common;
mod image;
mod md5;
mod processor;
mod timing;
mod video;

pub use self::common::{MediaConfig, MediaUnavailable};
pub use self::processor::MediaProcessor;
pub use self::timing::{timing_delta, TimingRecord, TimingSnapshot};
