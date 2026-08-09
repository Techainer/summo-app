//! Live capture from an input device.
//!
//! The audio callback runs on a real-time thread owned by the OS. Anything slow in it — an
//! allocation that hits the allocator's slow path, a mutex the pipeline thread happens to hold, a
//! model decode — shows up as a dropout in the recording. So the callback does the minimum:
//! downmix, resample, frame, push into a lock-free queue. Everything else happens elsewhere.
//!
//! When the queue is full the callback drops the frame and increments a counter rather than
//! blocking. A dropped frame is a small gap; a blocked callback is a corrupted recording, and on
//! some platforms a stream the OS never restarts.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use summo_core::{
    Error, Result,
    audio::{FRAME_LEN, SAMPLE_RATE},
    segment::Lane,
};

use crate::{
    convert::{Framer, Resampling, to_mono},
    device::{DeviceInfo, looks_bluetooth, pick_best},
};

/// How many frames may wait in the queue before the callback starts dropping them.
///
/// 200 frames is 20 seconds at 100 ms per frame — long enough to ride out a model reload or a
/// scheduling hiccup, short enough that a wedged consumer is noticed rather than swallowing memory.
const QUEUE_FRAMES: usize = 200;

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Device id to open, or `None` to pick automatically.
    pub device_id: Option<String>,
    pub lane: Lane,
    /// Samples per frame handed to the consumer.
    pub frame_len: usize,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            lane: Lane::Mic,
            frame_len: FRAME_LEN,
        }
    }
}

/// Counters for the performance HUD and for diagnosing a struggling machine.
#[derive(Debug, Default)]
pub struct CaptureStats {
    pub frames_captured: AtomicU64,
    /// Frames the callback discarded because the consumer was not keeping up.
    pub frames_dropped: AtomicU64,
}

impl CaptureStats {
    #[must_use]
    pub fn captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Relaxed)
    }

    /// Fraction of frames lost. Anything above zero is worth surfacing.
    #[must_use]
    pub fn drop_rate(&self) -> f64 {
        let captured = self.captured();
        let dropped = self.dropped();
        let total = captured + dropped;
        if total == 0 {
            0.0
        } else {
            dropped as f64 / total as f64
        }
    }
}

/// A running capture stream.
///
/// Dropping this stops the stream.
pub struct Capture {
    _stream: cpal::Stream,
    consumer: rtrb::Consumer<Frame>,
    stats: Arc<CaptureStats>,
    device: DeviceInfo,
    lane: Lane,
}

/// One frame of captured audio.
pub type Frame = Vec<f32>;

impl Capture {
    /// Enumerate input devices.
    pub fn devices() -> Result<Vec<DeviceInfo>> {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let devices = host
            .input_devices()
            .map_err(|e| Error::Audio(format!("cannot enumerate input devices: {e}")))?;

        let mut out = Vec::new();
        for device in devices {
            let Ok(name) = device.name() else { continue };
            // A device that refuses to describe its formats cannot be opened either.
            let Ok(configs) = device.supported_input_configs() else {
                continue;
            };
            let configs: Vec<_> = configs.collect();
            if configs.is_empty() {
                continue;
            }

            let max_sample_rate = configs
                .iter()
                .map(|c| c.max_sample_rate().0)
                .max()
                .unwrap_or(0);
            let channels = configs
                .iter()
                .map(cpal::SupportedStreamConfigRange::channels)
                .min()
                .unwrap_or(1);

            out.push(DeviceInfo {
                id: name.clone(),
                is_default: name == default_name,
                likely_bluetooth: looks_bluetooth(&name),
                name,
                max_sample_rate,
                channels,
            });
        }
        Ok(out)
    }

    /// Open a device and start capturing.
    pub fn start(cfg: CaptureConfig) -> Result<Self> {
        let host = cpal::default_host();
        let available = Self::devices()?;
        if available.is_empty() {
            return Err(Error::NoInputDevice);
        }

        let chosen = match &cfg.device_id {
            Some(id) => available
                .iter()
                .find(|d| &d.id == id)
                .ok_or_else(|| Error::Audio(format!("input device `{id}` not found")))?,
            None => pick_best(&available).ok_or(Error::NoInputDevice)?,
        }
        .clone();

        if let Some(warning) = chosen.warning() {
            tracing::warn!(device = %chosen.name, "{warning}");
        }

        let device = host
            .input_devices()
            .map_err(|e| Error::Audio(e.to_string()))?
            .find(|d| d.name().is_ok_and(|n| n == chosen.id))
            .ok_or_else(|| Error::Audio(format!("device `{}` disappeared", chosen.id)))?;

        let supported = device
            .default_input_config()
            .map_err(|e| Error::Audio(format!("no usable input config: {e}")))?;
        let source_rate = supported.sample_rate().0;
        let channels = supported.channels();

        tracing::info!(
            device = %chosen.name,
            rate = source_rate,
            channels,
            lane = cfg.lane.as_str(),
            "opening input"
        );

        let (mut producer, consumer) = rtrb::RingBuffer::<Frame>::new(QUEUE_FRAMES);
        let stats = Arc::new(CaptureStats::default());

        let mut resampler = Resampling::new(source_rate)?;
        let mut framer = Framer::new(cfg.frame_len);
        // Reused across callbacks so the real-time path does not allocate.
        let mut mono = Vec::with_capacity(4096);
        let mut resampled = Vec::with_capacity(4096);

        let cb_stats = Arc::clone(&stats);
        let err_stats = Arc::clone(&stats);

        let stream = device
            .build_input_stream(
                &supported.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    to_mono(data, channels, &mut mono);
                    resampled.clear();
                    if let Err(e) = resampler.process(&mono, &mut resampled) {
                        tracing::error!(error = %e, "resample failed in audio callback");
                        return;
                    }
                    framer.push(&resampled, |frame| {
                        // A full queue means the consumer has stalled. Dropping is the least-bad
                        // option: blocking here would stall the device thread as well.
                        if producer.push(frame.to_vec()).is_err() {
                            cb_stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        } else {
                            cb_stats.frames_captured.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                },
                move |err| {
                    tracing::error!(error = %err, "audio stream error");
                    err_stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                },
                None,
            )
            .map_err(|e| map_build_error(&e))?;

        stream
            .play()
            .map_err(|e| Error::Audio(format!("cannot start stream: {e}")))?;

        Ok(Self {
            _stream: stream,
            consumer,
            stats,
            device: chosen,
            lane: cfg.lane,
        })
    }

    /// Take the next captured frame, if one is ready. Never blocks.
    pub fn try_recv(&mut self) -> Option<Frame> {
        self.consumer.pop().ok()
    }

    /// Frames waiting to be consumed.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.consumer.slots()
    }

    /// Backlog in milliseconds, for the HUD's "falling behind" indicator.
    #[must_use]
    pub fn queued_ms(&self) -> u32 {
        let samples = self.queued() * FRAME_LEN;
        u32::try_from(samples as u64 * 1000 / u64::from(SAMPLE_RATE)).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn stats(&self) -> &CaptureStats {
        &self.stats
    }

    #[must_use]
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    #[must_use]
    pub fn lane(&self) -> Lane {
        self.lane
    }
}

/// Translate a stream-build failure into something a user can act on.
///
/// The distinction that matters is permission: on macOS and Windows a denied microphone permission
/// arrives as an ordinary device error, and telling the user "device unavailable" sends them to
/// check their hardware instead of their privacy settings.
fn map_build_error(err: &cpal::BuildStreamError) -> Error {
    match err {
        cpal::BuildStreamError::DeviceNotAvailable => Error::PermissionDenied(
            "the microphone is unavailable. If this is the first recording, grant microphone \
             permission in system settings and try again."
                .into(),
        ),
        other => Error::Audio(format!("cannot open input stream: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_rate_is_zero_before_anything_is_captured() {
        let stats = CaptureStats::default();
        assert_eq!(stats.drop_rate(), 0.0);
    }

    #[test]
    fn drop_rate_counts_dropped_against_the_total() {
        let stats = CaptureStats::default();
        stats.frames_captured.fetch_add(90, Ordering::Relaxed);
        stats.frames_dropped.fetch_add(10, Ordering::Relaxed);
        assert!((stats.drop_rate() - 0.1).abs() < 1e-9);
        assert_eq!(stats.captured(), 90);
        assert_eq!(stats.dropped(), 10);
    }

    #[test]
    fn a_denied_permission_is_not_reported_as_a_hardware_fault() {
        let err = map_build_error(&cpal::BuildStreamError::DeviceNotAvailable);
        assert!(matches!(err, Error::PermissionDenied(_)));
        assert!(
            err.to_string().contains("permission"),
            "the message must point at settings, not hardware: {err}"
        );
    }

    #[test]
    fn other_stream_errors_stay_generic() {
        let err = map_build_error(&cpal::BuildStreamError::StreamConfigNotSupported);
        assert!(matches!(err, Error::Audio(_)));
    }

    #[test]
    fn default_config_uses_the_pipeline_frame_size() {
        let cfg = CaptureConfig::default();
        assert_eq!(cfg.frame_len, FRAME_LEN);
        assert_eq!(cfg.lane, Lane::Mic);
        assert!(cfg.device_id.is_none(), "no device means pick the best one");
    }

    /// Enumeration must not panic on a machine with no sound hardware, which is every CI runner.
    #[test]
    fn enumerating_devices_never_panics() {
        let _ = Capture::devices();
    }
}
