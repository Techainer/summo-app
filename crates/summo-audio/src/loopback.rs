//! Capturing what the computer is playing.
//!
//! A meeting recorder that only hears the microphone records half a conversation. The other half —
//! everyone on the call — comes out of the speakers, and capturing it is the difference between a
//! dictation tool and a meeting tool.
//!
//! Every platform solves this differently, and none of them the same way as microphone capture:
//!
//! * **Linux** exposes a `.monitor` source per output device through PulseAudio and PipeWire, which
//!   appears in the ordinary input device list. Nothing special is needed beyond finding it.
//! * **Windows** has WASAPI loopback, available on any render endpoint since Vista and per-process
//!   since Windows 10 2004.
//! * **macOS** has no built-in route. Until 14.4 the only options were ScreenCaptureKit — which
//!   demands the Screen Recording permission, alarming for an audio app — or a virtual device the
//!   user installs. macOS 14.4 added Core Audio process taps, which need only audio permission.
//!
//! This module handles the Linux path natively and reports the others with an actionable message
//! rather than a generic failure, because "no loopback device" tells a user nothing about what to do.

use summo_core::{Error, Result};

use crate::device::DeviceInfo;

/// Name fragments that identify a loopback or monitor input.
const MONITOR_MARKERS: &[&str] = &[
    ".monitor",
    "monitor of",
    "monitor source",
    "stereo mix",
    "what u hear",
    "wave out mix",
    // Virtual devices users install on macOS.
    "blackhole",
    "soundflower",
    "loopback audio",
    "vb-cable",
    "vb-audio",
];

/// Whether a device name looks like a loopback capture point rather than a microphone.
#[must_use]
pub fn looks_like_loopback(name: &str) -> bool {
    let lower = name.to_lowercase();
    MONITOR_MARKERS.iter().any(|m| lower.contains(m))
}

/// Pick the best loopback device from a list of inputs.
///
/// Prefers the monitor of the *default* output, since that is what the user is actually listening
/// to; a machine can expose monitors for HDMI outputs nobody is using.
#[must_use]
pub fn pick_loopback<'a>(
    devices: &'a [DeviceInfo],
    default_output: Option<&str>,
) -> Option<&'a DeviceInfo> {
    let candidates: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| looks_like_loopback(&d.name))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    if let Some(output) = default_output {
        let output = output.to_lowercase();
        if let Some(matching) = candidates
            .iter()
            .find(|d| d.name.to_lowercase().contains(&output))
        {
            return Some(matching);
        }
    }

    // Otherwise take the widest-band monitor available.
    candidates.into_iter().max_by_key(|d| d.max_sample_rate)
}

/// What a user has to do to enable loopback on this platform, when it is not already available.
///
/// Written out per platform because a generic "no loopback device found" leaves the user with
/// nothing to act on, and this is the single most common reason a first recording only captures
/// one side of a call.
#[must_use]
pub fn setup_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "No monitor source was found. On PipeWire or PulseAudio every output has a matching \
         `.monitor` input — check that a sound server is running (`pactl info`) and that an output \
         device is active."
    }
    #[cfg(target_os = "windows")]
    {
        "System audio capture uses WASAPI loopback and needs no setup. If no device was found, \
         check that an output device is enabled in Sound settings."
    }
    #[cfg(target_os = "macos")]
    {
        "macOS has no built-in way to record system audio. On macOS 14.4 or later Summo can use a \
         Core Audio process tap, which asks only for audio permission. On earlier versions, install \
         a virtual device such as BlackHole and route your call audio through it."
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        "System audio capture is not supported on this platform."
    }
}

/// Whether this build can capture system audio without the user installing anything.
#[must_use]
pub fn is_supported_natively() -> bool {
    cfg!(any(target_os = "linux", target_os = "windows"))
}

/// Explain why loopback is unavailable, with the platform's remedy attached.
#[must_use]
pub fn unavailable() -> Error {
    Error::Audio(format!(
        "system audio capture is unavailable. {}",
        setup_hint()
    ))
}

/// Resolve the loopback device to open, or explain why there is none.
pub fn resolve<'a>(
    devices: &'a [DeviceInfo],
    default_output: Option<&str>,
) -> Result<&'a DeviceInfo> {
    pick_loopback(devices, default_output).ok_or_else(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, rate: u32) -> DeviceInfo {
        DeviceInfo {
            id: name.into(),
            name: name.into(),
            max_sample_rate: rate,
            channels: 2,
            is_default: false,
            likely_bluetooth: false,
        }
    }

    #[test]
    fn monitor_sources_are_recognised_across_platforms() {
        for name in [
            "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor",
            "Monitor of Built-in Audio Analog Stereo",
            "Stereo Mix (Realtek High Definition Audio)",
            "BlackHole 2ch",
            "VB-Cable Output",
        ] {
            assert!(looks_like_loopback(name), "should have matched `{name}`");
        }
    }

    #[test]
    fn microphones_are_not_mistaken_for_loopback() {
        for name in [
            "MacBook Pro Microphone",
            "Blue Yeti",
            "alsa_input.pci-0000_00_1f.3.analog-stereo",
            "Headset Microphone",
        ] {
            assert!(
                !looks_like_loopback(name),
                "should not have matched `{name}`"
            );
        }
    }

    #[test]
    fn the_monitor_of_the_active_output_wins() {
        // A machine often exposes monitors for HDMI outputs nobody is listening to.
        let devices = vec![
            device("alsa_output.hdmi-stereo.monitor", 48_000),
            device("alsa_output.usb-headphones.monitor", 48_000),
        ];
        let chosen = pick_loopback(&devices, Some("usb-headphones")).unwrap();
        assert_eq!(chosen.name, "alsa_output.usb-headphones.monitor");
    }

    #[test]
    fn without_a_hint_the_widest_band_monitor_is_chosen() {
        let devices = vec![
            device("monitor of narrow", 16_000),
            device("monitor of wide", 48_000),
        ];
        assert_eq!(
            pick_loopback(&devices, None).unwrap().name,
            "monitor of wide"
        );
    }

    #[test]
    fn a_machine_with_no_monitor_gets_an_actionable_message() {
        let devices = vec![device("MacBook Pro Microphone", 48_000)];
        let err = resolve(&devices, None).unwrap_err().to_string();

        assert!(err.contains("system audio capture is unavailable"));
        // The message must say what to do, not just what failed.
        assert!(err.len() > 80, "hint is too short to be useful: {err}");
    }

    #[test]
    fn the_hint_names_this_platform() {
        let hint = setup_hint();
        #[cfg(target_os = "linux")]
        assert!(hint.contains("monitor"), "got: {hint}");
        #[cfg(target_os = "macos")]
        assert!(
            hint.contains("BlackHole") || hint.contains("process tap"),
            "got: {hint}"
        );
        #[cfg(target_os = "windows")]
        assert!(hint.contains("WASAPI"), "got: {hint}");
    }

    #[test]
    fn native_support_matches_the_platform() {
        // macOS is the one that needs the user to act; the others should not claim otherwise.
        #[cfg(target_os = "macos")]
        assert!(!is_supported_natively());
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        assert!(is_supported_natively());
    }

    #[test]
    fn an_empty_device_list_does_not_panic() {
        assert!(pick_loopback(&[], None).is_none());
    }
}
