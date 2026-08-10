//! Keeping the audio of a meeting alongside its transcript.
//!
//! One file per lane, written as the audio arrives rather than buffered until the end: an
//! eight-hour meeting must not need eight hours of audio in memory, and a crash should cost the
//! last page rather than the recording.
//!
//! Lanes stay separate on purpose. Mixing the microphone and the system output into one file would
//! throw away the cleanest speaker signal there is — who was on which side of the call — and it is
//! not recoverable afterwards.

use std::collections::BTreeMap;
use std::path::PathBuf;

use summo_audio::OpusRecorder;
use summo_core::{MeetingId, Result, SAMPLE_RATE, paths::Paths, segment::Lane};

/// The audio files of one meeting.
pub struct AudioArchive {
    dir: PathBuf,
    lanes: BTreeMap<Lane, OpusRecorder>,
    enabled: bool,
}

/// What one lane cost on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneFile {
    pub lane: Lane,
    pub path: PathBuf,
    pub bytes: u64,
}

impl AudioArchive {
    /// Prepare to archive a meeting's audio.
    ///
    /// `enabled` is the user's `storage.keep_audio` setting. When it is off nothing is written and
    /// no directory is created — a setting that leaves empty folders behind has not been respected.
    #[must_use]
    pub fn new(paths: &Paths, meeting: &MeetingId, enabled: bool) -> Self {
        Self {
            dir: paths.audio_for(meeting),
            lanes: BTreeMap::new(),
            enabled,
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Append captured samples for one lane, opening its file the first time it is used.
    ///
    /// A lane that never carries audio — system output on a machine with no loopback — leaves no
    /// file, rather than a valid but empty one that looks like a failed recording.
    pub fn write(&mut self, lane: Lane, samples: &[f32]) -> Result<()> {
        if !self.enabled || samples.is_empty() {
            return Ok(());
        }
        let dir = &self.dir;
        let recorder = match self.lanes.entry(lane) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let path = dir.join(format!("{}.opus", lane.as_str()));
                e.insert(OpusRecorder::create(path, SAMPLE_RATE)?)
            }
        };
        recorder.write(samples)
    }

    /// Close every lane and report what was written.
    pub fn finish(self) -> Vec<LaneFile> {
        let mut files = Vec::new();
        for (lane, recorder) in self.lanes {
            let path = recorder.path().to_path_buf();
            match recorder.finish() {
                Ok(bytes) => files.push(LaneFile { lane, path, bytes }),
                Err(e) => {
                    // The transcript is already saved; a failure here costs the audio, not the
                    // meeting, and is not worth failing the stop for.
                    tracing::error!(path = %path.display(), error = %e, "could not close a recording");
                }
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn samples(seconds: f32) -> Vec<f32> {
        (0..(SAMPLE_RATE as f32 * seconds) as usize)
            .map(|i| (i as f32 * 300.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * 0.3)
            .collect()
    }

    #[test]
    fn each_lane_becomes_its_own_file() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();

        let mut archive = AudioArchive::new(&paths, &id, true);
        archive.write(Lane::Mic, &samples(0.5)).unwrap();
        archive.write(Lane::System, &samples(0.5)).unwrap();
        let files = archive.finish();

        assert_eq!(files.len(), 2);
        assert!(paths.audio_for(&id).join("mic.opus").exists());
        assert!(paths.audio_for(&id).join("system.opus").exists());
        assert!(files.iter().all(|f| f.bytes > 0));
    }

    #[test]
    fn a_lane_that_never_carries_audio_leaves_no_file() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();

        let mut archive = AudioArchive::new(&paths, &id, true);
        archive.write(Lane::Mic, &samples(0.2)).unwrap();
        archive.finish();

        assert!(paths.audio_for(&id).join("mic.opus").exists());
        assert!(
            !paths.audio_for(&id).join("system.opus").exists(),
            "an empty file looks like a recording that failed"
        );
    }

    #[test]
    fn turning_audio_off_writes_nothing_at_all() {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();

        let mut archive = AudioArchive::new(&paths, &id, false);
        archive.write(Lane::Mic, &samples(0.5)).unwrap();
        assert!(archive.finish().is_empty());
        assert!(
            !paths.audio_for(&id).exists(),
            "a setting that leaves an empty folder behind was not respected"
        );
    }

    #[test]
    fn an_hour_of_audio_stays_within_the_size_it_was_promised() {
        // The whole storage argument rests on this: roughly 5 MB per lane-hour. Ten seconds is
        // enough to measure the rate without a slow test.
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let id = MeetingId::new();

        let mut archive = AudioArchive::new(&paths, &id, true);
        for _ in 0..10 {
            archive.write(Lane::Mic, &samples(1.0)).unwrap();
        }
        let bytes = archive.finish()[0].bytes;
        let per_hour = bytes * 360;

        assert!(
            per_hour < 9 * 1024 * 1024,
            "an hour would be {} MB, which breaks the storage budget",
            per_hour / 1024 / 1024
        );
    }
}
