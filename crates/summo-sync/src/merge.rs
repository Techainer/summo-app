//! Reconciling two versions of the same file.
//!
//! Line-based three-way merge, the same shape `diff3` and git use. Deliberately **not** aware of
//! what a meeting document is: a vault holds meetings, notes, agent briefs, agent memories, agent
//! task lists and whatever else a user drops in, and a merge that only understood transcripts would
//! be a merge that mangled `AGENTS.md`. The general algorithm is also the one whose failure mode
//! people already recognise.
//!
//! ## What it does
//!
//! Find the lines all three versions agree on. Between those anchors, look at each side's changes:
//!
//! * Only one side changed a region → take that side. This is the common case and it is exact —
//!   two people editing different parts of the same file both keep their work.
//! * Both sides changed a region identically → take it once.
//! * Both sides changed it differently → **conflict**.
//!
//! ## What it does about a conflict
//!
//! Not conflict markers in the file. `<<<<<<<` in the middle of a meeting note is a broken document
//! — Obsidian renders it as garbage, the transcript parser sees a stray line, and a user who has
//! never used git has no idea what happened or how to fix it.
//!
//! Instead the local file is left **exactly as it was**, and the other version is written beside it
//! as `<name>.conflict-<machine>.md`. Two whole files, both valid, both openable, and the user
//! decides. Conflict copies are excluded from the next scan (see [`crate::snapshot`]) so they do
//! not spread to every other machine.
//!
//! ## Why not last-writer-wins
//!
//! Because it is not a merge, it is a coin toss with a step that destroys evidence. Clocks disagree
//! between machines; the "later" write is often the one that was open in a stale editor tab.

/// What came out of a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    /// Both sides' changes fitted together. This is the text to write.
    Clean(String),
    /// The same region changed on both sides. Nothing has been chosen.
    Conflict {
        /// How many regions could not be reconciled — for the message, not for a decision.
        regions: usize,
    },
}

impl Merged {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean(_))
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Clean(text) => Some(text),
            Self::Conflict { .. } => None,
        }
    }
}

/// Merge `mine` and `theirs`, using `base` as what they were both derived from.
///
/// `base` may be empty — two machines that each created the same path have no common ancestor. That
/// is handled the same way: with nothing in common, any difference is a conflict, which is the
/// correct answer rather than a special case.
#[must_use]
pub fn merge(base: &str, mine: &str, theirs: &str) -> Merged {
    if mine == theirs {
        return Merged::Clean(mine.to_string());
    }
    // One side never moved. Taking the other is exact, not a guess.
    if base == mine {
        return Merged::Clean(theirs.to_string());
    }
    if base == theirs {
        return Merged::Clean(mine.to_string());
    }

    let base_lines: Vec<&str> = base.lines().collect();
    let mine_lines: Vec<&str> = mine.lines().collect();
    let theirs_lines: Vec<&str> = theirs.lines().collect();

    // Each side's changes, expressed against the base. Working in *hunks* rather than in the gaps
    // between shared lines is what lets two nearby-but-separate edits both land: when I rewrite
    // line 2 and you append line 3, those are two hunks that do not overlap, and the region between
    // the last shared line and the end of the file contains both.
    let mine_hunks = hunks(&base_lines, &mine_lines);
    let theirs_hunks = hunks(&base_lines, &theirs_lines);

    let mut out: Vec<String> = Vec::new();
    let mut conflicts = 0;
    let mut at = 0usize; // how far through the base we have written
    let (mut i, mut j) = (0usize, 0usize);

    while i < mine_hunks.len() || j < theirs_hunks.len() {
        // Take whichever hunk starts first; if they start together, group them.
        let next = match (mine_hunks.get(i), theirs_hunks.get(j)) {
            (Some(a), Some(b)) => a.start.min(b.start),
            (Some(a), None) => a.start,
            (None, Some(b)) => b.start,
            (None, None) => break,
        };

        // Everything before the next change is untouched base.
        out.extend(base_lines[at..next].iter().map(|l| (*l).to_string()));

        // Collect the hunks from each side that overlap this position, widening while they do.
        let mut end = next;
        let (start_i, start_j) = (i, j);
        loop {
            let mut grew = false;
            // Overlapping means `start < end`, strictly. A hunk starting exactly where another
            // ended is *adjacent*, and grouping those was what turned "I rewrote line 2, you
            // appended line 3" into a conflict. The `== next` case is the one exception: two pure
            // insertions at the same point have zero width and would never overlap by that rule.
            while let Some(hunk) = mine_hunks.get(i) {
                if hunk.start < end || hunk.start == next {
                    end = end.max(hunk.end);
                    i += 1;
                    grew = true;
                } else {
                    break;
                }
            }
            while let Some(hunk) = theirs_hunks.get(j) {
                if hunk.start < end || hunk.start == next {
                    end = end.max(hunk.end);
                    j += 1;
                    grew = true;
                } else {
                    break;
                }
            }
            if !grew {
                break;
            }
        }

        let mine_side = replacement(&base_lines, &mine_hunks[start_i..i], next, end);
        let theirs_side = replacement(&base_lines, &theirs_hunks[start_j..j], next, end);
        let base_side: Vec<&str> = base_lines[next..end].to_vec();

        if mine_side == theirs_side {
            // Both made the same change. Take it once rather than twice.
            out.extend(mine_side.iter().map(|l| (*l).to_string()));
        } else if mine_side == base_side {
            out.extend(theirs_side.iter().map(|l| (*l).to_string()));
        } else if theirs_side == base_side {
            out.extend(mine_side.iter().map(|l| (*l).to_string()));
        } else if base_side.is_empty() && !base_lines.is_empty() {
            // Both sides *inserted* at the same point and nothing was removed. Keeping both loses
            // nothing, which is not true of picking one — and this is the shape of the conflicts
            // this vault actually produces: two machines each appending an action item, or each
            // remembering a different fact. Mine first, so the result does not depend on which
            // side happens to be running the sync.
            out.extend(mine_side.iter().map(|l| (*l).to_string()));
            out.extend(
                theirs_side
                    .iter()
                    .filter(|line| !mine_side.contains(line))
                    .map(|l| (*l).to_string()),
            );
        } else {
            // Both changed or removed the same existing lines. There is no answer that does not
            // discard somebody's edit, so nothing is chosen.
            conflicts += 1;
        }
        at = end;
    }
    out.extend(base_lines[at..].iter().map(|l| (*l).to_string()));

    if conflicts > 0 {
        return Merged::Conflict { regions: conflicts };
    }

    let mut text = out.join("\n");
    // A trailing newline is not a change worth reporting, but losing one rewrites every file that
    // had one — which shows up as a diff on the next sync, for ever.
    if mine.ends_with('\n') || theirs.ends_with('\n') {
        text.push('\n');
    }
    Merged::Clean(text)
}

/// One side's change to a stretch of the base.
///
/// `start..end` is the base range it replaces — equal when the change is a pure insertion, which is
/// most of what happens to an append-only file like an agent's memory.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    start: usize,
    end: usize,
    lines: Vec<String>,
}

/// What one side did to the base, as a list of replacements.
///
/// Derived from the alignment: base lines that survived are the fixed points, and everything
/// between two consecutive fixed points is one hunk if either side of it differs.
fn hunks(base: &[&str], side: &[&str]) -> Vec<Hunk> {
    let matched = align(base, side);

    let mut out: Vec<Hunk> = Vec::new();
    let mut pending_start = 0usize; // base index the current hunk starts at
    let mut side_at = 0usize; // how far through `side` the matched lines have taken us

    let push = |start: usize, end: usize, lines: Vec<String>, out: &mut Vec<Hunk>| {
        if start == end && lines.is_empty() {
            return;
        }
        if base[start..end] == lines.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            return;
        }
        out.push(Hunk { start, end, lines });
    };

    for (b, at) in matched.iter().enumerate() {
        let Some(at) = *at else { continue };
        // Base lines `pending_start..b` are gone; side lines `side_at..at` are new.
        let replacement: Vec<String> = side[side_at..at].iter().map(|l| (*l).to_string()).collect();
        push(pending_start, b, replacement, &mut out);
        pending_start = b + 1;
        side_at = at + 1;
    }
    let tail: Vec<String> = side[side_at..].iter().map(|l| (*l).to_string()).collect();
    push(pending_start, base.len(), tail, &mut out);
    out
}

/// What one side's hunks make of `start..end` of the base.
///
/// Base lines not covered by any of this side's hunks pass through unchanged, which is what makes
/// two overlapping-but-not-identical groups comparable.
fn replacement<'a>(base: &[&'a str], hunks: &'a [Hunk], start: usize, end: usize) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut at = start;
    for hunk in hunks {
        out.extend(base[at..hunk.start.max(at)].iter().copied());
        out.extend(hunk.lines.iter().map(String::as_str));
        at = hunk.end.max(at);
    }
    out.extend(base[at.min(end)..end].iter().copied());
    out
}

/// Where each base line ended up in one side, or `None` if it is gone.
///
/// Longest common subsequence, which is what a diff *is*. The table is quadratic, so a size guard
/// falls back to matching on lines that occur exactly once on both sides — patience diff's
/// anchoring. A vault holds eight-hour transcripts of tens of thousands of lines, and a merge that
/// allocates a ten-billion-cell table is a merge that hangs.
fn align(base: &[&str], side: &[&str]) -> Vec<Option<usize>> {
    const MAX_CELLS: usize = 4_000_000;

    if base.is_empty() || side.is_empty() {
        return vec![None; base.len()];
    }
    if base.len().saturating_mul(side.len()) > MAX_CELLS {
        return align_by_unique_lines(base, side);
    }

    // `lengths[i][j]` is the LCS of `base[i..]` and `side[j..]`, filled from the end so the
    // traceback below runs forwards and yields matches in order.
    let (n, m) = (base.len(), side.len());
    let mut lengths = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] = if base[i] == side[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }

    let mut out = vec![None; n];
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if base[i] == side[j] {
            out[i] = Some(j);
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Alignment for files too large to build a table for.
///
/// Only lines occurring exactly once on each side are matched. A blank line or `## Transcript`
/// occurs everywhere and pins nothing; `**[00:04:12] Ngọc** — …` occurs once and pins the place it
/// came from. Fewer anchors than a true LCS, so more of the file falls into regions — which costs
/// merge quality, never correctness: an unmatched region conflicts rather than being guessed at.
fn align_by_unique_lines(base: &[&str], side: &[&str]) -> Vec<Option<usize>> {
    fn once<'a>(lines: &[&'a str]) -> std::collections::HashMap<&'a str, (usize, usize)> {
        let mut seen = std::collections::HashMap::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            let slot = seen.entry(*line).or_insert((0, i));
            slot.0 += 1;
            slot.1 = i;
        }
        seen
    }

    let base_once = once(base);
    let side_once = once(side);

    let mut out = vec![None; base.len()];
    let mut last = None::<usize>;
    for (b, line) in base.iter().enumerate() {
        if base_once.get(line).map(|(count, _)| *count) != Some(1) {
            continue;
        }
        let Some(&(1, j)) = side_once.get(line) else {
            continue;
        };
        if last.is_some_and(|last| j <= last) {
            continue;
        }
        out[b] = Some(j);
        last = Some(j);
    }
    out
}

/// The name to write the other side's version under.
///
/// The machine's name is in it because the useful question is "whose is this", and a timestamp is
/// not: two conflicts on the same file from the same machine should overwrite rather than pile up.
/// The extension is preserved so the copy opens in whatever opens the original.
#[must_use]
pub fn conflict_name(path: &str, machine: &str) -> String {
    let machine = sanitize(machine);
    match path.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains('/') => {
            format!("{stem}.conflict-{machine}.{ext}")
        }
        _ => format!("{path}.conflict-{machine}"),
    }
}

/// A machine name that is safe as part of a filename.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "other".to_string()
    } else {
        trimmed.chars().take(32).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(base: &str, mine: &str, theirs: &str) -> String {
        match merge(base, mine, theirs) {
            Merged::Clean(text) => text,
            Merged::Conflict { regions } => panic!("expected a clean merge, got {regions} conflicts"),
        }
    }

    #[test]
    fn identical_versions_need_no_thought() {
        assert_eq!(clean("a\nb\n", "a\nb\n", "a\nb\n"), "a\nb\n");
    }

    #[test]
    fn a_change_on_one_side_only_is_taken_exactly() {
        assert_eq!(clean("a\nb\n", "a\nB\n", "a\nb\n"), "a\nB\n");
        assert_eq!(clean("a\nb\n", "a\nb\n", "a\nB\n"), "a\nB\n");
    }

    /// The case that makes a three-way merge worth having: two people work on the same file and
    /// both keep their work.
    #[test]
    fn edits_in_different_places_both_survive() {
        let base = "# Meeting\n\nfirst\n\nlast\n";
        let mine = "# Meeting\n\nFIRST\n\nlast\n";
        let theirs = "# Meeting\n\nfirst\n\nLAST\n";

        let merged = clean(base, mine, theirs);
        assert!(merged.contains("FIRST"), "{merged}");
        assert!(merged.contains("LAST"), "{merged}");
    }

    #[test]
    fn a_line_added_at_the_end_by_one_side_survives() {
        let merged = clean("a\nb\n", "a\nb\nc\n", "a\nb\n");
        assert_eq!(merged, "a\nb\nc\n");
    }

    #[test]
    fn lines_added_at_opposite_ends_both_survive() {
        let merged = clean("b\n", "a\nb\n", "b\nc\n");
        assert!(merged.contains('a'), "{merged}");
        assert!(merged.contains('c'), "{merged}");
    }

    #[test]
    fn both_sides_making_the_same_change_does_not_duplicate_it() {
        assert_eq!(clean("a\nb\n", "a\nB\n", "a\nB\n"), "a\nB\n");
    }

    #[test]
    fn a_deletion_on_one_side_is_taken() {
        assert_eq!(clean("a\nb\nc\n", "a\nc\n", "a\nb\nc\n"), "a\nc\n");
    }

    // ---- conflicts ---------------------------------------------------------------------------

    #[test]
    fn the_same_line_changed_differently_is_a_conflict() {
        let merged = merge("a\nb\n", "a\nMINE\n", "a\nTHEIRS\n");
        assert!(!merged.is_clean(), "{merged:?}");
        assert!(merged.text().is_none(), "nothing may be chosen");
    }

    /// Two machines that each made `notes/idea.md` before ever syncing. There is no ancestor, so
    /// any difference is a conflict — which is right, not a special case.
    #[test]
    fn two_files_created_independently_conflict_rather_than_one_winning() {
        let merged = merge("", "my idea\n", "their idea\n");
        assert!(!merged.is_clean());
    }

    #[test]
    fn two_files_created_independently_with_the_same_text_do_not_conflict() {
        assert_eq!(clean("", "same\n", "same\n"), "same\n");
    }

    /// Markers in the file would make a meeting note render as garbage in Obsidian and confuse
    /// somebody who has never used git.
    #[test]
    fn a_conflict_never_produces_marker_text() {
        let merged = merge("a\n", "mine\n", "theirs\n");
        assert!(merged.text().is_none());
        if let Merged::Conflict { regions } = merged {
            assert!(regions >= 1);
        }
    }

    // ---- the shapes a vault actually contains -------------------------------------------------

    /// Two machines adding a different action item to the same meeting. Both must survive; this is
    /// the single most likely real conflict in this product.
    #[test]
    fn two_machines_adding_different_action_items_both_keep_them() {
        let base = "# Họp\n\n## Việc cần làm\n\n- [ ] @ngoc Chốt spec <!-- id:T1 status:todo -->\n";
        let mine = "# Họp\n\n## Việc cần làm\n\n- [ ] @ngoc Chốt spec <!-- id:T1 status:todo -->\n- [ ] @minh Đo trên M1 <!-- id:T2 status:todo -->\n";
        let theirs = "# Họp\n\n## Việc cần làm\n\n- [ ] @ngoc Chốt spec <!-- id:T1 status:todo -->\n- [ ] @viet Viết release note <!-- id:T3 status:todo -->\n";

        let merged = clean(base, mine, theirs);
        assert!(merged.contains("T2"), "{merged}");
        assert!(merged.contains("T3"), "{merged}");
        assert!(merged.contains("T1"), "{merged}");
    }

    /// An agent's memory is append-only in practice, so two machines remembering different things
    /// should end up knowing both.
    #[test]
    fn two_agents_remembering_different_facts_keep_both() {
        let base = "# Memory\n\n- 2026-08-10 — Ngọc leads product\n";
        let mine = "# Memory\n\n- 2026-08-10 — Ngọc leads product\n- 2026-08-11 — Minh is on leave\n";
        let theirs = "# Memory\n\n- 2026-08-10 — Ngọc leads product\n- 2026-08-11 — the team ships on Fridays\n";

        let merged = clean(base, mine, theirs);
        assert!(merged.contains("Minh is on leave"), "{merged}");
        assert!(merged.contains("ships on Fridays"), "{merged}");
    }

    /// A transcript is long and only ever appended to during a recording, so a merge of two
    /// partial syncs must not be quadratic *or* lossy.
    #[test]
    fn a_long_transcript_with_an_edit_at_each_end_merges() {
        let mut base = String::from("# Họp\n\n## Transcript\n");
        for i in 0..500 {
            base.push_str(&format!("**[00:0{}:00] Ngọc** — dòng {i}\n", i % 10));
        }
        let mine = format!("{base}**[00:09:00] Ngọc** — của tôi\n");
        let theirs = base.replace("# Họp", "# Họp tuần");

        let merged = clean(&base, &mine, &theirs);
        assert!(merged.contains("của tôi"), "the appended line must survive");
        assert!(merged.contains("# Họp tuần"), "the retitle must survive");
    }

    /// Losing a trailing newline rewrites the file, which shows up as a diff on every later sync.
    #[test]
    fn a_trailing_newline_is_preserved() {
        assert!(clean("a\nb\n", "a\nB\n", "a\nb\nc\n").ends_with('\n'));
    }

    // ---- naming the copy ----------------------------------------------------------------------

    #[test]
    fn a_conflict_copy_keeps_the_extension_so_it_still_opens() {
        assert_eq!(
            conflict_name("meetings/2026-08-10-sync.md", "laptop"),
            "meetings/2026-08-10-sync.conflict-laptop.md"
        );
    }

    #[test]
    fn a_file_with_no_extension_still_gets_a_name() {
        assert_eq!(conflict_name("NOTES", "laptop"), "NOTES.conflict-laptop");
    }

    /// The machine name reaches a filename, and it comes from configuration a user typed.
    #[test]
    fn a_machine_name_cannot_escape_the_filename() {
        let name = conflict_name("a.md", "../../etc");
        assert!(!name.contains(".."), "{name}");
        assert!(!name.contains('/'), "{name}");
    }

    #[test]
    fn a_blank_machine_name_still_produces_a_usable_file() {
        assert_eq!(conflict_name("a.md", "   "), "a.conflict-other.md");
    }

    /// Two conflicts on the same file from the same machine should overwrite, not pile up.
    #[test]
    fn the_same_conflict_twice_names_the_same_file() {
        assert_eq!(conflict_name("a.md", "laptop"), conflict_name("a.md", "laptop"));
    }
}
