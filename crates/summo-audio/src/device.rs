//! Choosing an input device, and refusing a bad one.
//!
//! The single largest quality cliff in this whole pipeline is not the model — it is a Bluetooth
//! headset. When a Bluetooth device is used for *input*, the link switches from the high-quality
//! A2DP profile to HFP/HSP, a telephony profile that runs at 8 or 16 kHz with heavy compression.
//! Recognition accuracy collapses, and to the user it looks like the app is broken, because the
//! same headset sounded fine a moment earlier while playing music.
//!
//! Worse, operating systems pick that device *by default* when it is connected. So the choice
//! cannot be left to the OS: this module scores devices and warns loudly about narrowband ones.

use serde::{Deserialize, Serialize};

/// Below this, speech recognition degrades badly. Telephony-band audio sits under it by definition.
pub const NARROWBAND_HZ: u32 = 16_000;

/// An input device offered to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    /// Highest sample rate the device advertises.
    pub max_sample_rate: u32,
    pub channels: u16,
    pub is_default: bool,
    /// Name matched a known Bluetooth pattern.
    pub likely_bluetooth: bool,
}

impl DeviceInfo {
    /// Whether this device is too narrowband for usable recognition.
    #[must_use]
    pub fn is_narrowband(&self) -> bool {
        self.max_sample_rate < NARROWBAND_HZ
    }

    /// Rank for automatic selection; higher is better.
    ///
    /// The ordering encodes hard-won preferences: never auto-select Bluetooth, prefer wideband,
    /// and only then respect the system default. A user can still choose anything manually.
    #[must_use]
    pub fn score(&self) -> i32 {
        let mut score = 0;

        if self.likely_bluetooth {
            // Decisive: a Bluetooth headset in HFP mode is worse than a laptop's built-in mic in a
            // noisy room, and no amount of denoising recovers the missing bandwidth.
            score -= 1_000;
        }
        if self.is_narrowband() {
            score -= 500;
        }

        score += match self.max_sample_rate {
            r if r >= 48_000 => 100,
            r if r >= 44_100 => 90,
            r if r >= 32_000 => 60,
            r if r >= 16_000 => 30,
            _ => 0,
        };

        if self.is_default {
            score += 20;
        }
        // Mono inputs are typical of purpose-built microphones; multichannel devices are often
        // interfaces or virtual loopbacks. A mild preference only.
        if self.channels == 1 {
            score += 5;
        }
        score
    }

    /// A warning to show next to this device, if it deserves one.
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        if self.likely_bluetooth {
            return Some(
                "Bluetooth headsets switch to a low-quality telephony mode when recording. \
                 Recognition will be noticeably worse — prefer a built-in or USB microphone."
                    .into(),
            );
        }
        if self.is_narrowband() {
            return Some(format!(
                "This device only offers {} kHz. Speech recognition needs at least {} kHz to work well.",
                self.max_sample_rate / 1000,
                NARROWBAND_HZ / 1000
            ));
        }
        None
    }
}

/// Substrings that identify a Bluetooth audio endpoint across platforms.
///
/// Matching on names is crude, but neither CoreAudio, WASAPI nor ALSA exposes transport type
/// through `cpal`, and a false positive only costs a warning.
const BLUETOOTH_MARKERS: &[&str] = &[
    "bluetooth",
    "bluez",
    "airpods",
    "hands-free",
    "handsfree",
    "headset",
    "hfp",
    "hsp",
    "a2dp",
    "wh-1000",
    "wf-1000",
    "buds",
    "jabra",
    "beats",
];

/// Guess whether a device name refers to a Bluetooth endpoint.
#[must_use]
pub fn looks_bluetooth(name: &str) -> bool {
    let lower = name.to_lowercase();
    BLUETOOTH_MARKERS.iter().any(|m| lower.contains(m))
}

/// Choose the best device from a list, or `None` if the list is empty.
#[must_use]
pub fn pick_best(devices: &[DeviceInfo]) -> Option<&DeviceInfo> {
    devices.iter().max_by_key(|d| d.score())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, rate: u32, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: name.into(),
            name: name.into(),
            max_sample_rate: rate,
            channels: 1,
            is_default,
            likely_bluetooth: looks_bluetooth(name),
        }
    }

    #[test]
    fn bluetooth_names_are_recognised() {
        for name in [
            "AirPods Pro",
            "WH-1000XM5 (Hands-Free AG Audio)",
            "Jabra Evolve2 65",
            "bluez_input.AA_BB_CC",
            "Galaxy Buds2",
        ] {
            assert!(looks_bluetooth(name), "should flag `{name}`");
        }
    }

    #[test]
    fn wired_devices_are_not_flagged() {
        for name in [
            "MacBook Pro Microphone",
            "Blue Yeti",
            "Realtek High Definition Audio",
            "USB Audio Device",
        ] {
            assert!(!looks_bluetooth(name), "should not flag `{name}`");
        }
    }

    #[test]
    fn a_connected_headset_never_wins_over_the_built_in_mic() {
        // The exact situation the OS gets wrong: the headset is the system default.
        let devices = vec![
            device("AirPods Pro (Hands-Free)", 16_000, true),
            device("MacBook Pro Microphone", 48_000, false),
        ];
        assert_eq!(
            pick_best(&devices).unwrap().name,
            "MacBook Pro Microphone",
            "a default Bluetooth headset must not be auto-selected"
        );
    }

    #[test]
    fn the_system_default_wins_between_comparable_devices() {
        let devices = vec![
            device("Blue Yeti", 48_000, false),
            device("MacBook Pro Microphone", 48_000, true),
        ];
        assert_eq!(pick_best(&devices).unwrap().name, "MacBook Pro Microphone");
    }

    #[test]
    fn wideband_beats_narrowband() {
        let devices = vec![
            device("Cheap USB Mic", 8_000, true),
            device("Studio Interface", 48_000, false),
        ];
        assert_eq!(pick_best(&devices).unwrap().name, "Studio Interface");
    }

    #[test]
    fn narrowband_and_bluetooth_devices_carry_a_warning() {
        assert!(device("AirPods Pro", 16_000, false).warning().is_some());
        assert!(device("Telephony Mic", 8_000, false).warning().is_some());
        assert!(
            device("MacBook Pro Microphone", 48_000, false)
                .warning()
                .is_none()
        );
    }

    #[test]
    fn the_narrowband_warning_names_the_actual_rate() {
        let w = device("Telephony Mic", 8_000, false).warning().unwrap();
        assert!(w.contains("8 kHz"), "warning should be specific: {w}");
    }

    #[test]
    fn picking_from_nothing_is_not_a_panic() {
        assert!(pick_best(&[]).is_none());
    }

    #[test]
    fn a_bluetooth_headset_is_still_selectable_by_hand() {
        // Scoring only drives the automatic choice; the device stays in the list with a warning so
        // a user who knows what they are doing can pick it.
        let bt = device("AirPods Pro", 16_000, true);
        assert!(bt.score() < 0);
        assert!(bt.warning().is_some());
    }
}
