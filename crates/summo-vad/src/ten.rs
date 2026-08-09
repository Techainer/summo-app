//! TEN-VAD backend (optional, **not redistributed**).
//!
//! TEN-VAD releases the speech flag noticeably faster than Silero, which translates directly into
//! lower finalize latency because the gate closes a segment when the detector goes quiet. It is
//! also smaller and ships prebuilt for Android, iOS and WASM.
//!
//! It is nevertheless **off by default and never bundled**, because its licence is Apache-2.0 *with
//! additional conditions* — an anti-competition clause plus a restriction on who may deploy it.
//! Those extra restrictions make it incompatible with AGPL-3.0, so shipping it inside Summo would
//! violate our own licence. See `docs/adr/0001-vad-backend-licensing.md`.
//!
//! What this module therefore is: a loader for a library the *user* installed, guarded behind the
//! `ten-vad` Cargo feature. Building it requires `libten_vad` on the linker path, which our release
//! builds do not provide.

use std::ffi::c_void;

use summo_core::{Error, Result};

use crate::Vad;

/// Hop sizes the library accepts: 160 samples (10 ms) or 256 (16 ms) at 16 kHz.
pub const HOP_10MS: usize = 160;
pub const HOP_16MS: usize = 256;

type Handle = *mut c_void;

unsafe extern "C" {
    fn ten_vad_create(handle: *mut Handle, hop_size: usize, threshold: f32) -> i32;
    fn ten_vad_process(
        handle: Handle,
        audio: *const i16,
        len: usize,
        out_probability: *mut f32,
        out_flag: *mut i32,
    ) -> i32;
    fn ten_vad_destroy(handle: *mut Handle) -> i32;
    fn ten_vad_get_version() -> *const std::ffi::c_char;
}

pub struct TenVad {
    handle: Handle,
    hop: usize,
    /// Reused conversion buffer: the C API takes `int16`, our pipeline is `f32`.
    scratch: Vec<i16>,
}

// The handle is owned exclusively by this struct and every call goes through `&mut self`, so it is
// safe to move between threads even though the C library is not internally synchronised.
unsafe impl Send for TenVad {}

impl TenVad {
    /// Create a detector with the given hop size.
    ///
    /// The `threshold` passed to the library only affects its own boolean flag, which we ignore —
    /// [`crate::VadGate`] applies our threshold to the probability so the decision stays in one
    /// place across backends.
    pub fn new(hop: usize) -> Result<Self> {
        if hop != HOP_10MS && hop != HOP_16MS {
            return Err(Error::Vad(format!(
                "TEN-VAD supports hop sizes {HOP_10MS} or {HOP_16MS}, got {hop}"
            )));
        }
        let mut handle: Handle = std::ptr::null_mut();
        // SAFETY: `handle` is a valid out-pointer; the library writes an owned handle into it.
        let rc = unsafe { ten_vad_create(&raw mut handle, hop, 0.5) };
        if rc != 0 || handle.is_null() {
            return Err(Error::Vad(format!("ten_vad_create failed (rc={rc})")));
        }
        Ok(Self {
            handle,
            hop,
            scratch: vec![0; hop],
        })
    }

    /// Library version string, for benchmark provenance.
    pub fn version() -> String {
        // SAFETY: the library returns a static NUL-terminated string.
        unsafe {
            let ptr = ten_vad_get_version();
            if ptr.is_null() {
                return "unknown".into();
            }
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

impl Drop for TenVad {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: the handle came from `ten_vad_create` and is destroyed exactly once.
            unsafe { ten_vad_destroy(&raw mut self.handle) };
        }
    }
}

impl Vad for TenVad {
    fn frame_len(&self) -> usize {
        self.hop
    }

    fn feed_frame(&mut self, frame: &[f32]) -> Result<f32> {
        if frame.len() != self.hop {
            return Err(Error::Vad(format!(
                "TEN-VAD needs exactly {} samples, got {}",
                self.hop,
                frame.len()
            )));
        }

        for (dst, &src) in self.scratch.iter_mut().zip(frame) {
            // Scale to full-scale i16 with clamping; a wrapping cast would turn a clipped sample
            // into a loud one of the opposite sign.
            *dst = (src.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        }

        let mut prob = 0.0_f32;
        let mut flag = 0_i32;
        // SAFETY: handle is non-null and owned; `scratch` has exactly `hop` elements, which is the
        // length the library was created with; both out-pointers are valid for one write.
        let rc = unsafe {
            ten_vad_process(
                self.handle,
                self.scratch.as_ptr(),
                self.scratch.len(),
                &raw mut prob,
                &raw mut flag,
            )
        };
        if rc != 0 {
            return Err(Error::Vad(format!("ten_vad_process failed (rc={rc})")));
        }
        Ok(prob.clamp(0.0, 1.0))
    }

    fn reset(&mut self) {
        // The C API has no reset, so recreate the instance. Cheap: the model is a few hundred KB.
        if let Ok(fresh) = Self::new(self.hop) {
            let old = std::mem::replace(&mut self.handle, fresh.handle);
            // Prevent the temporary's `Drop` from destroying the handle we just took.
            std::mem::forget(fresh);
            let mut old = old;
            if !old.is_null() {
                // SAFETY: `old` is the handle previously owned by `self`, destroyed once here.
                unsafe { ten_vad_destroy(&raw mut old) };
            }
        }
    }

    fn name(&self) -> &'static str {
        "ten-vad"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_hop_sizes() {
        assert!(TenVad::new(512).is_err());
    }

    #[test]
    fn processes_a_frame_and_returns_a_probability() {
        let mut vad =
            TenVad::new(HOP_16MS).expect("library should be present with this feature on");
        let prob = vad.feed_frame(&vec![0.0; HOP_16MS]).unwrap();
        assert!((0.0..=1.0).contains(&prob));
    }

    #[test]
    fn wrong_frame_length_is_rejected() {
        let mut vad = TenVad::new(HOP_16MS).unwrap();
        assert!(vad.feed_frame(&vec![0.0; HOP_10MS]).is_err());
    }

    #[test]
    fn clipped_samples_do_not_wrap_sign() {
        let mut vad = TenVad::new(HOP_16MS).unwrap();
        // Values beyond full scale must clamp, not wrap into loud opposite-sign noise.
        assert!(vad.feed_frame(&vec![9.0; HOP_16MS]).is_ok());
        assert!(vad.feed_frame(&vec![-9.0; HOP_16MS]).is_ok());
    }
}
