//! Writing captured audio to disk as Ogg Opus.
//!
//! Keeping the audio matters for a reason that is easy to forget while the transcript looks good:
//! ASR is wrong sometimes, and a person who wants to check what was actually said needs the
//! recording. A transcript without its audio is a claim nobody can verify.
//!
//! Opus rather than WAV because the difference decides whether keeping it is realistic at all. One
//! hour of 16 kHz mono is about 115 MB as 16-bit WAV and about 5 MB as Opus at 12 kb/s — a year of
//! daily meetings is 30 GB against 1.3 GB. Speech at this bitrate is what Opus was designed for,
//! and it is the same codec every conferencing tool already puts the audio through.
//!
//! The file is a real Ogg Opus stream, not a private format: `ffplay`, VLC, a browser and every
//! phone can open it without Summo.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use audiopus::coder::Encoder;
use audiopus::{Application, Bitrate, Channels, SampleRate};
use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use summo_core::{Error, Result};

/// Encoder frame length in milliseconds. 20 ms is Opus' default and the best trade of quality
/// against per-packet overhead for speech.
const FRAME_MS: usize = 20;

/// Opus always reports positions at 48 kHz whatever the input rate, so a 16 kHz sample advances the
/// stream clock by three.
const OGG_RATE: u64 = 48_000;

/// Target bitrate. Speech at 16 kHz mono is intelligible well below this; 12 kb/s leaves headroom
/// for two people talking over a fan.
const BITRATE: i32 = 12_000;

/// Largest packet Opus can produce, so encoding never needs to grow a buffer mid-recording.
const MAX_PACKET: usize = 4_000;

/// A recording being written.
pub struct OpusRecorder {
    encoder: Encoder,
    writer: PacketWriter<'static, BufWriter<File>>,
    path: PathBuf,
    serial: u32,
    /// Input samples per encoded frame.
    frame_len: usize,
    /// Input samples not yet forming a whole frame.
    pending: Vec<i16>,
    /// Encoded frames so far, which is the stream position in frames.
    frames: u64,
    /// Input samples accepted, which is the true duration.
    samples: u64,
    sample_rate: u32,
    /// Samples the decoder discards at the start; the final granule accounts for it.
    pre_skip: u64,
    finished: bool,
}

impl OpusRecorder {
    /// Create a recording at `path`, overwriting anything there.
    ///
    /// `sample_rate` must be one Opus accepts natively — 8, 12, 16, 24 or 48 kHz. Summo captures at
    /// 16 kHz, which is deliberate: resampling to fit the encoder would be a second resample after
    /// the one the capture already did, and each one costs a little intelligibility.
    pub fn create(path: impl Into<PathBuf>, sample_rate: u32) -> Result<Self> {
        let path = path.into();
        let rate = match sample_rate {
            8_000 => SampleRate::Hz8000,
            12_000 => SampleRate::Hz12000,
            16_000 => SampleRate::Hz16000,
            24_000 => SampleRate::Hz24000,
            48_000 => SampleRate::Hz48000,
            other => {
                return Err(Error::Audio(format!(
                    "Opus cannot encode {other} Hz; capture at 8, 12, 16, 24 or 48 kHz"
                )));
            }
        };

        let mut encoder = Encoder::new(rate, Channels::Mono, Application::Voip)
            .map_err(|e| Error::Audio(format!("cannot create an Opus encoder: {e}")))?;
        encoder
            .set_bitrate(Bitrate::BitsPerSecond(BITRATE))
            .map_err(|e| Error::Audio(format!("cannot set the Opus bitrate: {e}")))?;

        let pre_skip = u64::from(encoder.lookahead().unwrap_or(312));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let file = File::create(&path).map_err(|e| Error::io(&path, e))?;

        // The serial identifies the logical stream inside the file. There is exactly one, so any
        // value works; deriving it from the path keeps it stable across a re-run without pulling in
        // a random number generator.
        let serial = fnv(path.to_string_lossy().as_bytes());

        let mut recorder = Self {
            encoder,
            writer: PacketWriter::new(BufWriter::new(file)),
            path,
            serial,
            frame_len: sample_rate as usize * FRAME_MS / 1000,
            pending: Vec::new(),
            frames: 0,
            samples: 0,
            sample_rate,
            // Reported at 48 kHz like every other position in the stream.
            pre_skip: pre_skip * OGG_RATE / u64::from(sample_rate),
            finished: false,
        };
        recorder.write_headers()?;
        Ok(recorder)
    }

    /// Append captured samples. Anything short of a whole frame is held until the next call.
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        for &sample in samples {
            // Clamp before scaling: a sample above 1.0 would wrap to a loud click, which is worse
            // than the clipping it came from.
            self.pending
                .push((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
        }
        self.samples += samples.len() as u64;

        while self.pending.len() >= self.frame_len {
            let frame: Vec<i16> = self.pending.drain(..self.frame_len).collect();
            self.encode(&frame, PacketWriteEndInfo::NormalPacket)?;
        }
        Ok(())
    }

    /// Seconds of audio accepted so far.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.samples as f64 / f64::from(self.sample_rate)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flush the tail and close the stream, returning the file size.
    ///
    /// A recording that is never finished is still playable up to its last complete page — the
    /// point of writing pages as they are encoded rather than buffering the meeting in memory.
    pub fn finish(mut self) -> Result<u64> {
        self.close()?;
        std::fs::metadata(&self.path)
            .map(|m| m.len())
            .map_err(|e| Error::io(&self.path, e))
    }

    fn close(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;

        // Opus encodes whole frames, so the tail is padded with silence. The final granule says how
        // much of it is real, and a decoder trims the rest.
        let mut frame: Vec<i16> = std::mem::take(&mut self.pending);
        frame.resize(self.frame_len, 0);
        // `EndStream`, not `EndPage`: it sets the end-of-stream bit, and without that bit a
        // decoder ignores the final granule and plays the padding as audio.
        self.encode(&frame, PacketWriteEndInfo::EndStream)?;

        // Flush the buffer before syncing: `sync_all` pushes the kernel's copy to the platter, and
        // without the flush there is nothing in the kernel's copy to push.
        let buffered = self.writer.inner_mut();
        buffered.flush().map_err(|e| Error::io(&self.path, e))?;
        buffered
            .get_mut()
            .sync_all()
            .map_err(|e| Error::io(&self.path, e))?;
        Ok(())
    }

    fn encode(&mut self, frame: &[i16], end: PacketWriteEndInfo) -> Result<()> {
        let mut packet = vec![0u8; MAX_PACKET];
        let written = self
            .encoder
            .encode(frame, &mut packet)
            .map_err(|e| Error::Audio(format!("Opus encoding failed: {e}")))?;
        packet.truncate(written);
        self.frames += 1;

        // Position of the last sample this packet decodes to, in 48 kHz units. On the final packet
        // it is the true length plus the pre-skip, which is how the decoder learns to drop the
        // silence the encoder padded with.
        let granule = if matches!(end, PacketWriteEndInfo::EndStream) {
            self.pre_skip + self.samples * OGG_RATE / u64::from(self.sample_rate)
        } else {
            self.frames * (self.frame_len as u64) * OGG_RATE / u64::from(self.sample_rate)
        };

        self.writer
            .write_packet(packet, self.serial, end, granule)
            .map_err(|e| Error::io(&self.path, e))
    }

    fn write_headers(&mut self) -> Result<()> {
        // OpusHead, as RFC 7845 defines it. Every field is fixed here except the pre-skip, which
        // belongs to this encoder instance.
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(1); // channels
        // RFC 7845 measures the pre-skip in 48 kHz samples even when the input is not 48 kHz.
        // Writing it in input samples makes every file play back a few milliseconds long, which
        // ffprobe caught and no amount of "the header is present" testing would have.
        head.extend_from_slice(&(self.pre_skip as u16).to_le_bytes());
        // Informational only: the rate the audio was captured at, so a tool can report it.
        head.extend_from_slice(&self.sample_rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family
        self.writer
            .write_packet(head, self.serial, PacketWriteEndInfo::EndPage, 0)
            .map_err(|e| Error::io(&self.path, e))?;

        let vendor = concat!("summo ", env!("CARGO_PKG_VERSION"));
        let mut tags = Vec::with_capacity(32);
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor.as_bytes());
        tags.extend_from_slice(&0u32.to_le_bytes()); // no user comments
        self.writer
            .write_packet(tags, self.serial, PacketWriteEndInfo::EndPage, 0)
            .map_err(|e| Error::io(&self.path, e))
    }
}

impl Drop for OpusRecorder {
    /// Close the stream if the caller did not.
    ///
    /// A power cut or a panic mid-meeting should still leave a playable file, so the tail is
    /// written here too. Errors are logged rather than propagated — a drop cannot fail, and losing
    /// the last 20 ms is not worth aborting over.
    fn drop(&mut self) {
        if let Err(e) = self.close() {
            tracing::warn!(path = %self.path.display(), error = %e, "could not close the recording cleanly");
        }
    }
}

/// FNV-1a. Small, dependency-free, and only ever used to label one stream inside one file.
fn fnv(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A second of a 440 Hz tone at 16 kHz.
    fn tone(seconds: f32) -> Vec<f32> {
        (0..(16_000.0 * seconds) as usize)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.4)
            .collect()
    }

    #[test]
    fn a_recording_is_a_real_ogg_opus_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        let mut recorder = OpusRecorder::create(&path, 16_000).unwrap();
        recorder.write(&tone(2.0)).unwrap();
        let size = recorder.finish().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"OggS", "not an Ogg container");
        assert!(
            bytes.windows(8).any(|w| w == b"OpusHead"),
            "the identification header is missing"
        );
        assert!(bytes.windows(8).any(|w| w == b"OpusTags"));
        assert_eq!(size, bytes.len() as u64);
    }

    #[test]
    fn two_seconds_of_speech_costs_a_few_kilobytes() {
        // The whole reason for Opus. As 16-bit WAV this would be 64 KB.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        let mut recorder = OpusRecorder::create(&path, 16_000).unwrap();
        recorder.write(&tone(2.0)).unwrap();
        let size = recorder.finish().unwrap();

        assert!(
            size < 12_000,
            "2 s should be a few kilobytes at 12 kb/s, got {size} bytes"
        );
        assert!(size > 1_000, "a file this small is probably empty: {size}");
    }

    #[test]
    fn the_duration_is_the_audio_that_went_in() {
        let dir = TempDir::new().unwrap();
        let mut recorder = OpusRecorder::create(dir.path().join("mic.opus"), 16_000).unwrap();
        // Deliberately not a multiple of the 320-sample frame, so the tail is padded.
        recorder.write(&tone(1.0)[..15_500]).unwrap();
        assert!((recorder.duration() - 0.96875).abs() < 1e-6);
    }

    #[test]
    fn samples_arriving_in_odd_sized_chunks_are_all_kept() {
        // Capture delivers whatever the device gives it, which is rarely a whole Opus frame.
        let dir = TempDir::new().unwrap();
        let mut recorder = OpusRecorder::create(dir.path().join("mic.opus"), 16_000).unwrap();
        let samples = tone(1.0);
        for chunk in samples.chunks(137) {
            recorder.write(chunk).unwrap();
        }
        assert_eq!(recorder.duration(), 1.0);
        assert!(recorder.finish().unwrap() > 500);
    }

    #[test]
    fn a_recording_dropped_without_finishing_is_still_playable() {
        // A panic or a power cut mid-meeting must not produce a file nothing can open.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        {
            let mut recorder = OpusRecorder::create(&path, 16_000).unwrap();
            recorder.write(&tone(0.5)).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"OggS");
        assert!(bytes.len() > 500, "the dropped recording is empty");
    }

    /// The last Ogg page's end-of-stream flag and granule position.
    fn last_page(bytes: &[u8]) -> (bool, u64) {
        let mut at = None;
        for i in 0..bytes.len().saturating_sub(27) {
            if &bytes[i..i + 4] == b"OggS" {
                at = Some(i);
            }
        }
        let i = at.expect("no Ogg page found");
        let eos = bytes[i + 5] & 0x04 != 0;
        let granule = u64::from_le_bytes(bytes[i + 6..i + 14].try_into().unwrap());
        (eos, granule)
    }

    #[test]
    fn the_final_page_says_exactly_how_much_audio_is_real() {
        // Opus pads the tail to a whole frame. The decoder learns to drop that padding from the
        // final granule — but only on a page marked end-of-stream. Without the flag, ffmpeg decodes
        // 3.0135 s for 3.000 s of input, which is how this was caught.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        let mut recorder = OpusRecorder::create(&path, 16_000).unwrap();
        // Not a multiple of the 320-sample frame, so there is padding to trim.
        recorder.write(&tone(1.0)[..15_500]).unwrap();
        let pre_skip = recorder.pre_skip;
        recorder.finish().unwrap();

        let (eos, granule) = last_page(&std::fs::read(&path).unwrap());
        assert!(eos, "the final page is not marked end-of-stream");
        assert_eq!(
            granule,
            pre_skip + 15_500 * 3,
            "the final granule does not describe the audio that went in"
        );
    }

    #[test]
    fn the_header_reports_the_pre_skip_in_48_khz_samples() {
        // RFC 7845 measures it at 48 kHz whatever the input rate. In input samples every file
        // plays back a few milliseconds long.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mic.opus");
        let recorder = OpusRecorder::create(&path, 16_000).unwrap();
        let pre_skip = recorder.pre_skip;
        recorder.finish().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let at = bytes.windows(8).position(|w| w == b"OpusHead").unwrap();
        assert_eq!(
            u16::from_le_bytes(bytes[at + 10..at + 12].try_into().unwrap()),
            pre_skip as u16
        );
        assert_eq!(
            u32::from_le_bytes(bytes[at + 12..at + 16].try_into().unwrap()),
            16_000,
            "the input rate field should report what was captured"
        );
    }

    #[test]
    fn loud_samples_clip_rather_than_wrap() {
        // Scaling without clamping turns a sample above 1.0 into a loud click at the other end of
        // the range — much worse than the clipping it came from.
        let dir = TempDir::new().unwrap();
        let mut recorder = OpusRecorder::create(dir.path().join("mic.opus"), 16_000).unwrap();
        recorder.write(&[2.0, -2.0, 1.5]).unwrap();
        assert_eq!(recorder.pending, vec![i16::MAX, -i16::MAX, i16::MAX]);
    }

    #[test]
    fn a_rate_opus_cannot_encode_is_refused_with_a_remedy() {
        let dir = TempDir::new().unwrap();
        let Err(err) = OpusRecorder::create(dir.path().join("x.opus"), 44_100) else {
            panic!("44.1 kHz should have been refused");
        };
        assert!(err.to_string().contains("16"), "got: {err}");
    }
}
