//! Capturing audio and normalising it.
//!
//! Everything downstream of this crate sees one format — 16 kHz mono `f32`, in fixed-size frames —
//! regardless of what the device actually produced. The conversion happens once, at the boundary,
//! inside the audio callback.
//!
//! Two things here are less obvious than they look:
//!
//! * **Device choice is not the OS's to make.** When a Bluetooth headset is connected the system
//!   makes it the default input, which silently switches the link to a telephony profile and
//!   destroys recognition accuracy. [`device`] scores devices to avoid that.
//! * **The audio callback must never block.** It cannot allocate, decode, or wait on a lock, so
//!   captured frames go into a lock-free queue and the pipeline runs on another thread.

pub mod capture;
pub mod convert;
pub mod device;
pub mod loopback;
pub mod record;

pub use capture::{Capture, CaptureConfig, CaptureStats};
pub use convert::{Framer, Resampling, to_mono};
pub use device::{DeviceInfo, NARROWBAND_HZ, looks_bluetooth, pick_best};
pub use loopback::{looks_like_loopback, pick_loopback, setup_hint};
pub use record::OpusRecorder;
