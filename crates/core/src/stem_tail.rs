//! Incremental reader for a capture stem that is still being written.
//!
//! Native call capture (ScreenCaptureKit) writes microphone and system audio to
//! separate WAV stems as the call runs, but the recording path never started the
//! live-transcription sidecar, so live consumers received nothing during a
//! meeting even though the post-stop transcript was fine (#576).
//!
//! The sidecar is already decoupled: it consumes 16 kHz mono `f32` chunks over
//! an mpsc channel and does not care where they come from. This module supplies
//! them by tailing the growing stems, which avoids touching the Swift helper.
//!
//! Two properties of those files shape the implementation:
//!
//! - The header is not finalized until capture stops. The declared `data` size
//!   stays at its placeholder while samples accumulate (the same condition that
//!   made recovery reject healthy audio in #519), so the tailer measures the
//!   file on each poll instead of trusting the header.
//! - Reads land mid-frame. A poll can stop partway through a sample or a frame,
//!   so leftover bytes carry into the next poll rather than being dropped.

// Wired into the native call-recording path in the follow-up commit on this
// branch. Kept separate so the decode/tailing logic lands with its own tests
// rather than inside a larger desktop change.
#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Sample rate the live sidecar consumes.
const TARGET_RATE: u32 = 16_000;

/// Sample encodings the native call helper can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    F32,
    I16,
}

impl Encoding {
    fn bytes_per_sample(self) -> usize {
        match self {
            Encoding::F32 => 4,
            Encoding::I16 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StemFormat {
    channels: u16,
    sample_rate: u32,
    encoding: Encoding,
}

/// A growing stem, read forward from where the last poll stopped.
#[derive(Debug)]
pub(crate) struct StemTail {
    path: PathBuf,
    format: StemFormat,
    /// Absolute byte offset of the next unread audio byte.
    cursor: u64,
    /// Bytes of a frame that a previous poll could not complete.
    carry: Vec<u8>,
    /// Fractional read position for decimation, carried across polls so the
    /// resampled stream stays continuous instead of restarting each time.
    resample_pos: f64,
}

impl StemTail {
    /// Open a stem and position the cursor at the first audio byte.
    ///
    /// Fails while the header is too short to describe the format, which is
    /// normal in the first moments of a capture; the caller retries.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        let mut header = [0_u8; 4096];
        let read = file
            .read(&mut header)
            .map_err(|e| format!("read header {}: {e}", path.display()))?;
        let (format, data_offset) = parse_header(&header[..read])?;
        Ok(Self {
            path: path.to_path_buf(),
            format,
            cursor: data_offset,
            carry: Vec::new(),
            resample_pos: 0.0,
        })
    }

    /// Read whatever has been appended since the last poll, as 16 kHz mono.
    ///
    /// Returns an empty vector when the writer has not advanced. Errors are
    /// transient by nature here (the file is being written concurrently), so the
    /// caller should treat them as "nothing this round" rather than fatal.
    pub(crate) fn poll(&mut self) -> Result<Vec<f32>, String> {
        let mut file =
            File::open(&self.path).map_err(|e| format!("reopen {}: {e}", self.path.display()))?;
        let len = file
            .metadata()
            .map_err(|e| format!("stat {}: {e}", self.path.display()))?
            .len();
        if len <= self.cursor {
            return Ok(Vec::new());
        }

        let want = (len - self.cursor) as usize;
        let mut fresh = vec![0_u8; want];
        file.seek(SeekFrom::Start(self.cursor))
            .map_err(|e| format!("seek {}: {e}", self.path.display()))?;
        let got = file
            .read(&mut fresh)
            .map_err(|e| format!("read {}: {e}", self.path.display()))?;
        fresh.truncate(got);
        self.cursor += got as u64;

        if !self.carry.is_empty() {
            let mut joined = std::mem::take(&mut self.carry);
            joined.extend_from_slice(&fresh);
            fresh = joined;
        }

        let frame = self.format.channels as usize * self.format.encoding.bytes_per_sample();
        if frame == 0 {
            return Err("stem reports zero-width frames".into());
        }
        let usable = fresh.len() - (fresh.len() % frame);
        self.carry = fresh[usable..].to_vec();

        Ok(self.decode_to_mono_16k(&fresh[..usable]))
    }

    /// Decode whole frames, average channels to mono, and decimate to 16 kHz.
    fn decode_to_mono_16k(&mut self, bytes: &[u8]) -> Vec<f32> {
        let channels = self.format.channels as usize;
        let width = self.format.encoding.bytes_per_sample();
        let frames = bytes.len() / (channels * width);
        let ratio = self.format.sample_rate as f64 / TARGET_RATE as f64;

        let mut out = Vec::with_capacity(((frames as f64) / ratio).ceil() as usize + 1);
        for frame in 0..frames {
            // Average the channels rather than taking the first: the system stem
            // can carry content on one side only, and dropping a channel would
            // silence it.
            let mut sum = 0.0_f32;
            for ch in 0..channels {
                let at = (frame * channels + ch) * width;
                sum += match self.format.encoding {
                    Encoding::F32 => {
                        f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                    }
                    Encoding::I16 => {
                        i16::from_le_bytes([bytes[at], bytes[at + 1]]) as f32 / 32768.0
                    }
                };
            }
            let mono = sum / channels as f32;

            // Nearest-source decimation, matching the capture path's approach.
            // `resample_pos` persists across polls so chunk boundaries do not
            // restart the phase and introduce a click.
            if self.resample_pos <= frame as f64 {
                out.push(mono);
                self.resample_pos += ratio;
            }
        }
        // Rebase so the position stays relative to the next chunk.
        self.resample_pos = (self.resample_pos - frames as f64).max(0.0);
        out
    }
}

/// Locate the `fmt ` and `data` chunks in a RIFF/WAVE header.
///
/// Returns the format and the absolute offset of the first audio byte. The
/// declared `data` size is deliberately ignored: it is a placeholder until the
/// writer finalizes the file, and the caller measures the real length instead.
fn parse_header(bytes: &[u8]) -> Result<(StemFormat, u64), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE stem".into());
    }

    let mut at = 12_usize;
    let mut format: Option<StemFormat> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;

        if id == b"fmt " {
            if body + 16 > bytes.len() {
                return Err("fmt chunk truncated".into());
            }
            let tag = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let sample_rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            // 0xFFFE is WAVE_FORMAT_EXTENSIBLE; the helper writes float there,
            // and bit depth disambiguates the rest.
            let encoding = match (tag, bits) {
                (3, 32) | (0xFFFE, 32) => Encoding::F32,
                (1, 16) | (0xFFFE, 16) => Encoding::I16,
                _ => return Err(format!("unsupported stem format (tag {tag}, {bits} bits)")),
            };
            if channels == 0 || channels > 32 || sample_rate == 0 {
                return Err("implausible stem format".into());
            }
            format = Some(StemFormat {
                channels,
                sample_rate,
                encoding,
            });
        } else if id == b"data" {
            let format = format.ok_or_else(|| "data chunk before fmt".to_string())?;
            return Ok((format, body as u64));
        }

        // Chunks are word-aligned.
        at = body + size + (size & 1);
    }
    Err("no data chunk in header window".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a stem whose header is unfinalized, matching what the native
    /// helper leaves on disk while a call is in progress.
    fn write_growing_stem(path: &Path, channels: u16, rate: u32, encoding: Encoding) {
        let mut f = File::create(path).unwrap();
        let (tag, bits) = match encoding {
            Encoding::F32 => (3_u16, 32_u16),
            Encoding::I16 => (1_u16, 16_u16),
        };
        let block_align = channels * (bits / 8);
        let byte_rate = rate * block_align as u32;

        f.write_all(b"RIFF").unwrap();
        f.write_all(&4088_u32.to_le_bytes()).unwrap(); // placeholder
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16_u32.to_le_bytes()).unwrap();
        f.write_all(&tag.to_le_bytes()).unwrap();
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bits.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&0_u32.to_le_bytes()).unwrap(); // placeholder, never updated
    }

    fn append_f32(path: &Path, samples: &[f32]) {
        let mut f = File::options().append(true).open(path).unwrap();
        for s in samples {
            f.write_all(&s.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn reads_only_what_was_appended_since_the_last_poll() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("voice.wav");
        write_growing_stem(&stem, 1, 16_000, Encoding::F32);

        let mut tail = StemTail::open(&stem).unwrap();
        assert!(tail.poll().unwrap().is_empty(), "nothing written yet");

        append_f32(&stem, &[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(tail.poll().unwrap().len(), 4);

        // A second poll with no new bytes must not replay the same audio.
        assert!(tail.poll().unwrap().is_empty());

        append_f32(&stem, &[0.5, 0.6]);
        assert_eq!(tail.poll().unwrap().len(), 2);
    }

    #[test]
    fn carries_a_partial_frame_across_polls() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("voice.wav");
        write_growing_stem(&stem, 1, 16_000, Encoding::F32);
        let mut tail = StemTail::open(&stem).unwrap();

        // Three bytes: less than one 4-byte f32 frame.
        {
            let mut f = File::options().append(true).open(&stem).unwrap();
            f.write_all(&[1, 2, 3]).unwrap();
        }
        assert!(
            tail.poll().unwrap().is_empty(),
            "a partial frame must not be decoded"
        );

        // The fourth byte completes it.
        {
            let mut f = File::options().append(true).open(&stem).unwrap();
            f.write_all(&[4]).unwrap();
        }
        assert_eq!(
            tail.poll().unwrap().len(),
            1,
            "the carried bytes complete one frame"
        );
    }

    #[test]
    fn downmixes_channels_rather_than_dropping_one() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("system.wav");
        write_growing_stem(&stem, 2, 16_000, Encoding::F32);
        let mut tail = StemTail::open(&stem).unwrap();

        // Content only on the right channel: taking channel 0 would silence it.
        append_f32(&stem, &[0.0, 1.0]);
        let out = tail.poll().unwrap();
        assert_eq!(out.len(), 1);
        assert!(
            (out[0] - 0.5).abs() < 1e-6,
            "expected the average, got {out:?}"
        );
    }

    #[test]
    fn decimates_to_16k_and_keeps_phase_across_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("voice.wav");
        write_growing_stem(&stem, 1, 48_000, Encoding::F32);
        let mut tail = StemTail::open(&stem).unwrap();

        // 48 kHz in, 16 kHz out: expect roughly a third of the frames.
        append_f32(&stem, &vec![0.25_f32; 48]);
        let first = tail.poll().unwrap().len();
        append_f32(&stem, &vec![0.25_f32; 48]);
        let second = tail.poll().unwrap().len();

        assert!((15..=17).contains(&first), "first chunk: {first}");
        assert!((15..=17).contains(&second), "second chunk: {second}");
    }

    #[test]
    fn decodes_i16_stems() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("voice.wav");
        write_growing_stem(&stem, 1, 16_000, Encoding::I16);
        let mut tail = StemTail::open(&stem).unwrap();

        let mut f = File::options().append(true).open(&stem).unwrap();
        f.write_all(&16384_i16.to_le_bytes()).unwrap();
        drop(f);

        let out = tail.poll().unwrap();
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.5).abs() < 1e-3, "got {out:?}");
    }

    #[test]
    fn rejects_a_header_that_is_not_a_wav() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("bogus.wav");
        std::fs::write(&stem, b"not a wav at all").unwrap();
        assert!(StemTail::open(&stem).is_err());
    }
}
