//! What the app and the daemon say to each other.
//!
//! Two channels over one WebSocket. **Binary** frames carry PCM: one byte of lane tag followed by
//! little-endian `f32` samples at 16 kHz. **Text** frames carry JSON commands one way and
//! [`summo_core::Event`] the other.
//!
//! Audio is binary rather than base64-in-JSON for a reason worth stating: at 100 ms per frame the
//! app sends ten messages a second for an hour, and base64 would add a third to every one of them
//! plus an encode and a parse on each side, for no benefit.

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, audio::FRAME_LEN, segment::Lane};

/// Commands from the app to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Begin a recording session. Loads models if they are not already resident.
    SessionStart(Box<SessionSpec>),
    /// End the session, flushing any utterance still open.
    SessionStop,
    /// What the user has typed into the meeting so far.
    ///
    /// The whole section every time, not a diff: the app holds the text and this is a save, so a
    /// dropped frame costs nothing and there is no order to get wrong.
    ///
    /// Over the socket rather than as an HTTP route, because the recorder lives in the socket's own
    /// task and holds the document in memory. A second writer reaching the file directly would be
    /// overwritten by the next autosave — silently, and only for the person who typed while the
    /// meeting was running.
    Notes { text: String },
    /// Load a model without starting a session, so the first recording is not delayed by it.
    ModelLoad { id: String },
    /// Fetch and install a model from the registry.
    ModelPull { id: String },
    /// Change what is listening, without ending the meeting.
    ///
    /// Both fields are optional and mean "leave this as it is": a user who realises the call is in
    /// English changes the language, and the model follows from it; a user comparing two models
    /// changes the model and keeps the language. The recording, the file and everything already
    /// transcribed are untouched — only the next utterance is decoded differently.
    ///
    /// This exists because the alternative is stopping and starting again, which costs the part of
    /// the meeting where somebody noticed. A meeting is not always in the language its owner's
    /// settings say, and finding that out is something that happens *during* it.
    ModelSwap {
        #[serde(default)]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// Keepalive. Some proxies drop an idle WebSocket, and a dropped socket mid-meeting is data loss.
    Ping,
}

/// How a session should run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSpec {
    /// Model driving live text.
    ///
    /// May be empty, meaning "whatever the settings say" — see `server::resolve_models`. The
    /// interface leaves it empty; a client that knows exactly which model it wants, such as the
    /// import job, names one.
    #[serde(default)]
    pub live_model: String,
    /// Slower model that re-decodes finished utterances, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_model: Option<String>,
    /// Which tracks to capture.
    #[serde(default = "default_lanes")]
    pub lanes: Vec<Lane>,
    /// ISO language code, or `None` to let the model detect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Attribute speakers within the remote lane.
    #[serde(default)]
    pub diarize: bool,
    /// Translate finished lines into this language as they land.
    ///
    /// This is the "watch a talk in another language" switch. There is no separate feature behind
    /// it: system-audio capture already hears whatever is playing, so turning this on while
    /// something plays gives live bilingual subtitles for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_to: Option<String>,
    /// Capture device id, or `None` to pick the best one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

fn default_lanes() -> Vec<Lane> {
    vec![Lane::Mic]
}

impl SessionSpec {
    #[must_use]
    pub fn new(live_model: impl Into<String>) -> Self {
        Self {
            live_model: live_model.into(),
            refine_model: None,
            lanes: default_lanes(),
            language: None,
            diarize: false,
            translate_to: None,
            device_id: None,
        }
    }

    /// Reject a specification that cannot produce a working session.
    pub fn validate(&self) -> Result<()> {
        if self.live_model.trim().is_empty() {
            // Coded, because this is the one a *new user* hits: press record before installing a
            // model and the app said `configuration error: session needs a live model` — English,
            // raw, in a Vietnamese interface, on somebody's first minute. A code lets the screen
            // say what to do about it instead.
            return Err(Error::msg("session.no_model", "session needs a live model"));
        }
        if self.lanes.is_empty() {
            return Err(Error::Config("session needs at least one lane".into()));
        }
        if self.refine_model.as_deref() == Some(self.live_model.as_str()) {
            return Err(Error::Config(
                "refine model is the same as the live model, which would decode everything twice \
                 for no benefit"
                    .into(),
            ));
        }
        // Diarization on the microphone lane alone is wasted work: that lane is the local user by
        // construction, so clustering it can only ever produce one speaker.
        if self.diarize && !self.lanes.contains(&Lane::System) {
            return Err(Error::Config(
                "diarization needs the system lane; the microphone lane is always the local user"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Decode a binary audio frame: one lane tag byte, then little-endian `f32` samples.
pub fn decode_frame(bytes: &[u8]) -> Result<(Lane, Vec<f32>)> {
    let Some((&tag, rest)) = bytes.split_first() else {
        return Err(Error::Other("empty audio frame".into()));
    };
    let lane =
        Lane::from_tag(tag).ok_or_else(|| Error::Other(format!("unknown lane tag {tag}")))?;

    if !rest.len().is_multiple_of(4) {
        return Err(Error::Other(format!(
            "audio frame payload of {} bytes is not a whole number of f32 samples",
            rest.len()
        )));
    }

    let samples: Vec<f32> = rest
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    // A frame far larger than expected is either a protocol error or an attempt to make the daemon
    // allocate without bound; either way it should not be decoded silently.
    if samples.len() > FRAME_LEN * 64 {
        return Err(Error::Other(format!(
            "audio frame of {} samples is implausibly large",
            samples.len()
        )));
    }
    Ok((lane, samples))
}

/// Encode a binary audio frame, for the app side and for tests.
#[must_use]
pub fn encode_frame(lane: Lane, samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + samples.len() * 4);
    out.push(lane.tag());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_as_tagged_json() {
        let cases = vec![
            Command::SessionStart(Box::new(SessionSpec::new("gipformer-65m"))),
            Command::SessionStop,
            Command::ModelLoad { id: "x".into() },
            Command::ModelPull { id: "x".into() },
            Command::ModelSwap {
                id: "x".into(),
                language: None,
            },
            Command::ModelSwap {
                id: String::new(),
                language: Some("en".into()),
            },
            Command::Ping,
        ];
        for cmd in cases {
            let json = serde_json::to_string(&cmd).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
        }
    }

    #[test]
    fn session_start_is_readable_on_the_wire() {
        let json =
            serde_json::to_value(Command::SessionStart(Box::new(SessionSpec::new("m")))).unwrap();
        assert_eq!(json["cmd"], "session_start");
        assert_eq!(json["live_model"], "m");
    }

    #[test]
    fn a_session_without_a_model_is_refused() {
        let mut spec = SessionSpec::new("");
        assert!(spec.validate().is_err());
        spec.live_model = "m".into();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn refining_with_the_same_model_is_refused() {
        let mut spec = SessionSpec::new("gipformer-65m");
        spec.refine_model = Some("gipformer-65m".into());
        let err = spec.validate().unwrap_err().to_string();
        assert!(err.contains("twice for no benefit"), "got: {err}");
    }

    #[test]
    fn diarizing_only_the_microphone_lane_is_refused() {
        // It would burn a model on a lane whose speaker is already known with certainty.
        let mut spec = SessionSpec::new("m");
        spec.diarize = true;
        assert!(spec.validate().is_err());

        spec.lanes = vec![Lane::Mic, Lane::System];
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn a_session_with_no_lanes_is_refused() {
        let mut spec = SessionSpec::new("m");
        spec.lanes.clear();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn audio_frames_round_trip() {
        let samples: Vec<f32> = (0..FRAME_LEN).map(|i| (i as f32 / 100.0).sin()).collect();
        for lane in [Lane::Mic, Lane::System] {
            let encoded = encode_frame(lane, &samples);
            let (back_lane, back) = decode_frame(&encoded).unwrap();
            assert_eq!(back_lane, lane);
            assert_eq!(back, samples);
        }
    }

    #[test]
    fn an_empty_frame_is_an_error_not_a_silent_no_op() {
        assert!(decode_frame(&[]).is_err());
    }

    #[test]
    fn an_unknown_lane_tag_is_rejected() {
        assert!(decode_frame(&[9, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn a_truncated_sample_is_rejected_rather_than_dropped() {
        // Three trailing bytes cannot be an f32. Silently discarding them would shift every
        // subsequent sample and quietly corrupt the audio.
        let mut frame = encode_frame(Lane::Mic, &[1.0, 2.0]);
        frame.truncate(frame.len() - 1);
        assert!(decode_frame(&frame).is_err());
    }

    #[test]
    fn an_implausibly_large_frame_is_refused() {
        // Guards a loopback daemon against a client that asks it to allocate without bound.
        let huge = encode_frame(Lane::Mic, &vec![0.0; FRAME_LEN * 65]);
        assert!(decode_frame(&huge).is_err());
    }

    #[test]
    fn lanes_default_to_the_microphone() {
        let spec: SessionSpec = serde_json::from_str(r#"{"live_model":"m"}"#).unwrap();
        assert_eq!(spec.lanes, vec![Lane::Mic]);
        assert!(!spec.diarize);
    }
}
