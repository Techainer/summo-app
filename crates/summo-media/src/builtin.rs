//! Decoding without asking the user to install anything.
//!
//! The setup checklist used to carry a step that said, in effect, *go and get ffmpeg*. It was
//! marked non-blocking and it still read as a chore between somebody and the app they had just
//! downloaded — and it was there for a feature most people meet on their second day: importing a
//! recording they already had.
//!
//! Symphonia decodes the formats that recording actually arrives in — mp3, m4a/AAC, ALAC, FLAC,
//! WAV, Vorbis, and the audio track inside an mp4, mov or mkv — in this process, under MIT, with no
//! external program. `rubato` resamples, the same crate the microphone path already uses.
//!
//! ffmpeg is not gone and should not be. It reads things this does not: Opus in WebM, AC-3, WMA,
//! anything exotic, and containers a phone or a conferencing tool invented last year. So it stays
//! as the fallback, and the module docs in `lib.rs` still hold for why it is a subprocess rather
//! than a linked library. What changes is the order: try in-process, and reach for the program on
//! the machine only when that fails.
//!
//! What this deliberately does not do is guess. If Symphonia cannot decode a file, the error says
//! so and the caller falls through to ffmpeg — a half-decoded import is worse than a slow one.

use std::fs::File;
use std::path::Path;

use rubato::{FftFixedInOut, Resampler};
use summo_core::{Error, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{MediaInfo, TARGET_RATE};

/// Everything decoded from a file: mono samples and the rate they are at.
struct Decoded {
    samples: Vec<f32>,
    rate: u32,
    channels: u32,
}

/// Open a file and find its audio track.
fn open(path: &Path) -> Result<(symphonia::core::probe::ProbeResult, u32)> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    // The extension is a hint and nothing more — Symphonia sniffs the container either way, so a
    // `.mp4` holding something else still works and a file with no extension is not refused.
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| Error::Other(format!("{}: {e}", path.display())))?;

    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| Error::Other(format!("{}: no audio track", path.display())))?;
    let id = track.id;
    Ok((probed, id))
}

/// What is in the file, without decoding all of it.
pub fn info(path: &Path) -> Result<MediaInfo> {
    let (probed, track_id) = open(path)?;
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.id == track_id)
        .ok_or_else(|| Error::Other(format!("{}: no audio track", path.display())))?;

    let params = &track.codec_params;
    let rate = params.sample_rate;
    let channels = params.channels.map(|c| c.count() as u32);
    // Frames over rate, when the container recorded both. Symphonia does not estimate from bitrate
    // the way ffprobe does, so a stream with neither is reported as zero rather than as a guess —
    // and a zero-length import is refused upstream, which is the right outcome for a file we
    // cannot measure.
    let duration_s = match (params.n_frames, rate) {
        (Some(frames), Some(rate)) if rate > 0 => frames as f64 / f64::from(rate),
        _ => 0.0,
    };

    Ok(MediaInfo {
        duration_s,
        has_audio: true,
        // Not knowable here without walking every track, and no caller asks this of a file it is
        // about to transcribe: the question they ask is `has_audio`.
        has_video: false,
        sample_rate: rate,
        channels,
    })
}

/// Decode the whole file to interleaved mono.
fn decode(path: &Path) -> Result<Decoded> {
    let (mut probed, track_id) = open(path)?;
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.id == track_id)
        .ok_or_else(|| Error::Other(format!("{}: no audio track", path.display())))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| Error::Other(format!("{}: {e}", path.display())))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(0);
    let mut buffer: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match probed.format.next_packet() {
            Ok(packet) => packet,
            // The end of the stream arrives as an error in Symphonia's API. Anything else is a
            // real failure and stops the decode.
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(Error::Other(format!("{}: {e}", path.display()))),
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                rate = spec.rate;
                channels = spec.channels.count();
                let buf = buffer
                    .get_or_insert_with(|| SampleBuffer::<f32>::new(audio.capacity() as u64, spec));
                buf.copy_interleaved_ref(audio);

                // Downmixed, not one channel picked: a meeting recorded with one speaker per
                // channel would otherwise lose half the conversation. Same choice the ffmpeg
                // arguments make.
                if channels <= 1 {
                    samples.extend_from_slice(buf.samples());
                } else {
                    for frame in buf.samples().chunks_exact(channels) {
                        samples.push(frame.iter().sum::<f32>() / channels as f32);
                    }
                }
            }
            // A damaged packet in the middle of an hour of audio should cost that packet, not the
            // import. Symphonia says which errors are worth continuing past.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                return Err(Error::Other(format!(
                    "{}: the stream changed format part-way through",
                    path.display()
                )));
            }
            Err(e) => return Err(Error::Other(format!("{}: {e}", path.display()))),
        }
    }

    if samples.is_empty() || rate == 0 {
        return Err(Error::Other(format!(
            "{}: decoded no audio",
            path.display()
        )));
    }

    Ok(Decoded {
        samples,
        rate,
        channels: channels as u32,
    })
}

/// 16 kHz mono, whatever went in.
fn to_target_rate(decoded: Decoded) -> Result<Vec<f32>> {
    if decoded.rate == TARGET_RATE {
        return Ok(decoded.samples);
    }

    // Fixed-size FFT resampling, the same crate the capture path uses. The chunk size is a
    // trade between allocation count and latency, and there is no latency to trade here — this
    // runs over a whole file — so it is sized for throughput.
    let chunk = 4096;
    let mut resampler =
        FftFixedInOut::<f32>::new(decoded.rate as usize, TARGET_RATE as usize, chunk, 1)
            .map_err(|e| Error::Other(format!("cannot resample {} Hz: {e}", decoded.rate)))?;

    let needed = resampler.input_frames_next();
    let mut out = Vec::with_capacity(
        decoded.samples.len() * TARGET_RATE as usize / decoded.rate as usize + needed,
    );
    let mut input = decoded.samples;
    // Padded to a whole chunk: dropping the tail would cut the last fraction of a second off every
    // import, which is exactly where somebody says the thing they wanted transcribed.
    let remainder = input.len() % needed;
    if remainder != 0 {
        input.resize(input.len() + (needed - remainder), 0.0);
    }

    for block in input.chunks(needed) {
        let resampled = resampler
            .process(&[block.to_vec()], None)
            .map_err(|e| Error::Other(format!("resampling failed: {e}")))?;
        out.extend_from_slice(&resampled[0]);
    }
    Ok(out)
}

/// Decode to the 16 kHz mono WAV every recogniser here wants.
pub fn to_wav(input: &Path, output: &Path) -> Result<()> {
    let decoded = decode(input)?;
    let samples = to_target_rate(decoded)?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output, spec)
        .map_err(|e| Error::Other(format!("cannot write {}: {e}", output.display())))?;
    for sample in samples {
        let clamped = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(clamped)
            .map_err(|e| Error::Other(format!("cannot write {}: {e}", output.display())))?;
    }
    writer
        .finalize()
        .map_err(|e| Error::Other(format!("cannot finish {}: {e}", output.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of a 440 Hz tone at 44.1 kHz stereo, written as a WAV.
    fn tone(path: &Path, rate: u32, channels: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..rate {
            let value =
                ((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
            for _ in 0..channels {
                writer.write_sample(value).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn a_stereo_file_comes_out_mono_at_the_rate_the_recogniser_wants() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        let target = dir.path().join("out/tone16.wav");
        tone(&source, 44_100, 2);

        to_wav(&source, &target).unwrap();

        let reader = hound::WavReader::open(&target).unwrap();
        assert_eq!(reader.spec().channels, 1, "the recogniser wants mono");
        assert_eq!(reader.spec().sample_rate, TARGET_RATE);
        // A second in, a second out — within a chunk of padding.
        let seconds = reader.duration() as f64 / f64::from(TARGET_RATE);
        assert!(
            (0.95..1.35).contains(&seconds),
            "a second of audio came out as {seconds:.2}s"
        );
    }

    #[test]
    fn a_file_already_at_16k_is_not_resampled_into_something_else() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        let target = dir.path().join("tone16.wav");
        tone(&source, TARGET_RATE, 1);

        to_wav(&source, &target).unwrap();

        let reader = hound::WavReader::open(&target).unwrap();
        assert_eq!(
            reader.duration(),
            TARGET_RATE,
            "exactly one second, sample for sample"
        );
    }

    #[test]
    fn info_reads_the_rate_and_the_channel_count() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        tone(&source, 48_000, 2);

        let info = info(&source).unwrap();
        assert!(info.has_audio);
        assert_eq!(info.sample_rate, Some(48_000));
        assert_eq!(info.channels, Some(2));
        assert!((info.duration_s - 1.0).abs() < 0.05, "{}", info.duration_s);
    }

    #[test]
    fn something_that_is_not_audio_is_refused_rather_than_half_read() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.txt");
        std::fs::write(&source, "this is not a recording").unwrap();
        assert!(info(&source).is_err());
        assert!(to_wav(&source, &dir.path().join("out.wav")).is_err());
    }
}
