//! Writing the meeting down.
//!
//! Everything upstream of this produces events. Without it, a recording exists only in the app's
//! memory and the promise that your data is a folder of files you own is not kept.
//!
//! Two properties matter more than the format:
//!
//! **A crash costs seconds, not the meeting.** The document is written every
//! [`AUTOSAVE_INTERVAL`], so a power cut loses at most that much. Waiting until the user presses
//! stop would mean an hour of conversation living in RAM, which is exactly the sort of thing that
//! is fine until the one time it is not.
//!
//! **A crash mid-write cannot corrupt what was already saved.** Writes go to a temporary file and
//! are renamed over the target, which is atomic on every filesystem Summo runs on. Writing in place
//! would mean a process killed halfway through leaves a truncated meeting, which is worse than an
//! older complete one.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use summo_core::{Error, Event, MeetingId, Result, SpeakerId, paths::Paths, segment::Segment};
use summo_vault::write_atomically;
use summo_vault::{MeetingDoc, meeting::Frontmatter, slug::meeting_stem};

/// How often the document is flushed while recording.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(10);

/// Accumulates a meeting and keeps it on disk.
pub struct Recorder {
    doc: MeetingDoc,
    path: PathBuf,
    last_save: Instant,
    /// Set when an event changed the document since the last write, so an idle meeting does not
    /// rewrite the same bytes every ten seconds.
    dirty: bool,
    saves: u64,
}

impl Recorder {
    /// Start recording into the vault.
    ///
    /// `title` is what the file is named after. A meeting usually has no title when it starts, so
    /// the caller passes something derived from the clock and renames later if a summary suggests
    /// better.
    pub fn start(
        paths: &Paths,
        id: MeetingId,
        title: &str,
        date: &str,
        models: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let dir = paths.meetings();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

        let mut frontmatter = Frontmatter::new(id, date);
        frontmatter.models = models.into_iter().collect();

        let doc = MeetingDoc::new(frontmatter, title);
        let path = unique_path(&dir, &meeting_stem(date, title));

        Ok(Self {
            doc,
            path,
            last_save: Instant::now(),
            dirty: true,
            saves: 0,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn document(&self) -> &MeetingDoc {
        &self.doc
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.doc.transcript.len()
    }

    /// Times the document has been written. Exposed so a test can prove autosave actually fires.
    #[must_use]
    pub fn save_count(&self) -> u64 {
        self.saves
    }

    /// Fold one event into the document.
    ///
    /// Partials are deliberately ignored. They are cosmetic, they arrive several times a second,
    /// and writing them would mean the saved file spends most of its life containing half-finished
    /// sentences.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Final(segment) => {
                self.upsert(segment);
                self.dirty = true;
            }
            Event::Revise(segment) => {
                // A revision may arrive after the file has already been written with the earlier
                // text; upsert handles both cases.
                self.upsert(segment);
                self.dirty = true;
            }
            // Offline diarization can rename a speaker on a meeting whose lines are already saved.
            Event::SpeakerRename { from, to } if self.rename_speaker(from, to) => {
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn upsert(&mut self, incoming: &Segment) {
        match self
            .doc
            .transcript
            .iter_mut()
            .find(|existing| existing.seq == incoming.seq && existing.lane == incoming.lane)
        {
            // Reuses the same precedence rules the UI applies, so the saved file and the screen
            // cannot disagree about which text won.
            Some(existing) => {
                existing.merge(incoming);
            }
            None => {
                let position = self.doc.transcript.partition_point(|s| s.t0 <= incoming.t0);
                self.doc.transcript.insert(position, incoming.clone());
            }
        }
    }

    fn rename_speaker(&mut self, from: &SpeakerId, to: &SpeakerId) -> bool {
        let mut changed = false;
        for segment in &mut self.doc.transcript {
            if segment.speaker.as_ref() == Some(from) {
                segment.speaker = Some(to.clone());
                changed = true;
            }
        }
        changed
    }

    /// Replace or add a section, such as the summary produced after the meeting ends.
    pub fn set_section(&mut self, heading: &str, body: impl Into<String>) {
        self.doc.set_section(heading, body);
        self.dirty = true;
    }

    /// Record who took part, for the people pages and their backlinks.
    pub fn set_participants(&mut self, participants: Vec<String>) {
        self.doc.frontmatter.participants = participants;
        self.dirty = true;
    }

    /// Write if enough time has passed and something changed.
    ///
    /// Called from the event loop, so it must be cheap when there is nothing to do.
    pub fn maybe_save(&mut self) -> Result<bool> {
        if !self.dirty || self.last_save.elapsed() < AUTOSAVE_INTERVAL {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Write the document now.
    pub fn save(&mut self) -> Result<()> {
        let markdown = self.doc.to_markdown()?;
        write_atomically(&self.path, markdown.as_bytes())?;
        self.last_save = Instant::now();
        self.dirty = false;
        self.saves += 1;
        Ok(())
    }

    /// Finish: stamp the duration and write one last time.
    pub fn finish(mut self, duration_s: f64) -> Result<PathBuf> {
        self.doc.frontmatter.duration = duration_s.max(0.0) as u64;
        self.dirty = true;
        self.save()?;
        Ok(self.path)
    }
}

/// A path that does not collide with an existing meeting.
///
/// Two meetings a day can easily share a title — "standup" every morning — and silently overwriting
/// yesterday's would be data loss disguised as a naming convention.
fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.md"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..1000 {
        let candidate = dir.join(format!("{stem}-{suffix}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}.md", uuid::Uuid::now_v7().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::segment::{Lane, SegmentSource};

    fn recorder(dir: &Path) -> Recorder {
        Recorder::start(
            &Paths::at(dir),
            MeetingId::from("m1".to_string()),
            "Weekly Sync",
            "2026-08-10",
            [("live".to_string(), "gipformer-65m".to_string())],
        )
        .unwrap()
    }

    fn segment(seq: u64, text: &str, t0: f64, source: SegmentSource) -> Segment {
        let mut s = Segment::new(seq, Lane::Mic, text, t0, t0 + 1.0);
        s.source = source;
        s.speaker = Some(SpeakerId::me());
        s
    }

    #[test]
    fn a_finished_utterance_reaches_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());

        rec.apply(&Event::Final(segment(
            0,
            "Anh nghĩ mình nên dùng Rust",
            1.0,
            SegmentSource::Final,
        )));
        rec.save().unwrap();

        let body = std::fs::read_to_string(rec.path()).unwrap();
        assert!(body.contains("Anh nghĩ mình nên dùng Rust"), "got: {body}");
        assert!(body.starts_with("---"), "frontmatter should lead the file");
        assert!(
            body.contains("gipformer-65m"),
            "the model used should be recorded"
        );
    }

    #[test]
    fn partial_text_is_never_written() {
        // Partials arrive several times a second and are cosmetic; saving them would mean the file
        // spends most of its life containing half-finished sentences.
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());

        rec.apply(&Event::Partial(segment(
            0,
            "Anh nghĩ mình",
            1.0,
            SegmentSource::Partial,
        )));
        rec.save().unwrap();

        let body = std::fs::read_to_string(rec.path()).unwrap();
        assert!(
            !body.contains("Anh nghĩ mình"),
            "a partial reached the file: {body}"
        );
        assert_eq!(rec.segment_count(), 0);
    }

    #[test]
    fn a_revision_replaces_the_text_already_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());

        rec.apply(&Event::Final(segment(
            0,
            "toi nghi minh nen",
            1.0,
            SegmentSource::Final,
        )));
        rec.save().unwrap();
        rec.apply(&Event::Revise(segment(
            0,
            "Tôi nghĩ mình nên",
            1.0,
            SegmentSource::Revised,
        )));
        rec.save().unwrap();

        let body = std::fs::read_to_string(rec.path()).unwrap();
        assert!(body.contains("Tôi nghĩ mình nên"));
        assert!(
            !body.contains("toi nghi minh nen"),
            "the old text survived: {body}"
        );
        assert_eq!(
            rec.segment_count(),
            1,
            "a revision must not add a second line"
        );
    }

    #[test]
    fn segments_are_kept_in_time_order_even_if_they_arrive_out_of_order() {
        // A refine pass on one lane can land after a later utterance on another.
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());

        rec.apply(&Event::Final(segment(1, "sau", 10.0, SegmentSource::Final)));
        rec.apply(&Event::Final(segment(
            0,
            "trước",
            2.0,
            SegmentSource::Final,
        )));

        let times: Vec<f64> = rec.document().transcript.iter().map(|s| s.t0).collect();
        assert_eq!(times, vec![2.0, 10.0]);
    }

    #[test]
    fn renaming_a_speaker_rewrites_every_line_they_said() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());

        for (seq, text) in [(0, "một"), (1, "hai")] {
            let mut s = segment(seq, text, seq as f64, SegmentSource::Final);
            s.speaker = Some(SpeakerId::auto(0));
            rec.apply(&Event::Final(s));
        }

        rec.apply(&Event::SpeakerRename {
            from: SpeakerId::auto(0),
            to: SpeakerId::from("Ngọc".to_string()),
        });
        rec.save().unwrap();

        let body = std::fs::read_to_string(rec.path()).unwrap();
        assert_eq!(body.matches("Ngọc").count(), 2, "got: {body}");
        assert!(!body.contains("S1"));
    }

    #[test]
    fn autosave_fires_on_its_interval_and_not_before() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());
        rec.apply(&Event::Final(segment(0, "câu", 1.0, SegmentSource::Final)));

        assert!(!rec.maybe_save().unwrap(), "should not save immediately");

        // Reach back in time rather than sleeping ten seconds in a unit test.
        rec.last_save = Instant::now() - AUTOSAVE_INTERVAL - Duration::from_millis(1);
        assert!(
            rec.maybe_save().unwrap(),
            "should save once the interval has passed"
        );
        assert_eq!(rec.save_count(), 1);
    }

    #[test]
    fn an_idle_meeting_does_not_rewrite_the_same_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());
        rec.save().unwrap();

        rec.last_save = Instant::now() - AUTOSAVE_INTERVAL - Duration::from_millis(1);
        assert!(
            !rec.maybe_save().unwrap(),
            "nothing changed, so nothing should be written"
        );
        assert_eq!(rec.save_count(), 1);
    }

    #[test]
    fn a_saved_file_is_never_left_half_written() {
        // The temporary file must not survive, and the target must always parse.
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());
        rec.apply(&Event::Final(segment(
            0,
            "nội dung",
            1.0,
            SegmentSource::Final,
        )));
        rec.save().unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(rec.path().parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "a temporary file was left behind");

        let body = std::fs::read_to_string(rec.path()).unwrap();
        assert!(
            MeetingDoc::parse(&body).is_ok(),
            "the saved file does not parse back"
        );
    }

    #[test]
    fn two_meetings_with_the_same_title_do_not_overwrite_each_other() {
        // "standup" happens every morning; silently replacing yesterday's would be data loss
        // dressed up as a naming convention.
        let tmp = tempfile::tempdir().unwrap();
        let mut first = recorder(tmp.path());
        first.save().unwrap();

        let mut second = recorder(tmp.path());
        second.save().unwrap();

        assert_ne!(first.path(), second.path());
        assert!(first.path().exists() && second.path().exists());
    }

    #[test]
    fn finishing_stamps_the_duration_and_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());
        rec.apply(&Event::Final(segment(0, "xong", 1.0, SegmentSource::Final)));

        let path = rec.finish(2538.4).unwrap();
        let doc = MeetingDoc::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(doc.frontmatter.duration, 2538);
        assert_eq!(doc.transcript.len(), 1);
    }

    #[test]
    fn a_summary_written_after_the_meeting_lands_in_the_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut rec = recorder(tmp.path());
        rec.apply(&Event::Final(segment(
            0,
            "nội dung",
            1.0,
            SegmentSource::Final,
        )));
        rec.set_section("Tóm tắt", "Chốt dùng Rust cho phần lõi.");
        rec.set_participants(vec!["[[Bạn]]".into(), "[[Ngọc]]".into()]);

        let path = rec.finish(60.0).unwrap();
        let doc = MeetingDoc::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(doc.section("Tóm tắt"), Some("Chốt dùng Rust cho phần lõi."));
        assert_eq!(doc.frontmatter.participants.len(), 2);
        assert_eq!(
            doc.transcript.len(),
            1,
            "the transcript must survive alongside the summary"
        );
    }

    #[test]
    fn the_file_is_named_after_the_date_and_title() {
        let tmp = tempfile::tempdir().unwrap();
        let rec = recorder(tmp.path());
        let name = rec
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "2026-08-10-weekly-sync.md");
    }
}
