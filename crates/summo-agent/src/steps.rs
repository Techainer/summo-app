//! Writing the agent's own plan back into the task it came from.
//!
//! An agent task's steps are its board. They live as indented checkboxes under the task, which
//! means the record of what the agent did is the same file the user reads, editable and greppable
//! like everything else in the vault.
//!
//! The important property is that a step is written **before** it runs and ticked **after**. A
//! crash therefore leaves a step unticked rather than losing it, and the user sees the truth: this
//! is the point it stopped. Writing the whole plan at the end would leave a crashed task looking
//! like it never started.

use summo_core::{Error, Result, paths::Paths};
use summo_vault::tasks::{Status, Step, Task};

/// Rewrite one agent task's step list in the file it lives in.
///
/// Replaces the block of indented checkboxes that follows the task, leaving every other byte alone.
pub fn write(paths: &Paths, task: &Task, steps: &[Step]) -> Result<()> {
    let path = paths.vault().join(&task.file);
    let body = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let rewritten = replace_steps(&body, task.line, steps)?;
    summo_vault::write::write_atomically(&path, rewritten.as_bytes())
}

/// Add one step and mark the task as running.
pub fn begin_step(paths: &Paths, task: &Task, text: &str) -> Result<Task> {
    let mut task = task.clone();
    task.steps.push(Step {
        text: text.to_string(),
        done: false,
    });
    if task.status != Status::Doing {
        task.status = Status::Doing;
        summo_vault::tasks_io::update(paths, &task.id, Some(Status::Doing), None, None)?;
    }
    write(paths, &task, &task.steps)?;
    Ok(task)
}

/// Tick the last step.
pub fn finish_step(paths: &Paths, task: &Task) -> Result<Task> {
    let mut task = task.clone();
    if let Some(last) = task.steps.last_mut() {
        last.done = true;
    }
    write(paths, &task, &task.steps)?;
    Ok(task)
}

/// Replace the indented block after `line` with `steps`.
///
/// Public for testing: this is pure string work and deserves to be checked without a filesystem.
pub fn replace_steps(markdown: &str, line: usize, steps: &[Step]) -> Result<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let parent = lines
        .get(line)
        .ok_or_else(|| Error::Other(format!("line {line} is past the end of the document")))?;

    let parent_indent = parent.len() - parent.trim_start().len();
    let indent = " ".repeat(parent_indent + 2);

    // Everything immediately below that is indented deeper is the previous plan.
    let mut end = line + 1;
    while let Some(next) = lines.get(end) {
        let deeper = !next.trim().is_empty()
            && next.len() - next.trim_start().len() > parent_indent
            && next.trim_start().starts_with(['-', '*']);
        if !deeper {
            break;
        }
        end += 1;
    }

    let mut out: Vec<String> = lines[..=line].iter().map(|l| (*l).to_string()).collect();
    for step in steps {
        out.push(format!(
            "{indent}- [{}] {}",
            if step.done { "x" } else { " " },
            step.text
        ));
    }
    out.extend(lines[end..].iter().map(|l| (*l).to_string()));

    let mut joined = out.join("\n");
    if markdown.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(items: &[(&str, bool)]) -> Vec<Step> {
        items
            .iter()
            .map(|(text, done)| Step {
                text: (*text).to_string(),
                done: *done,
            })
            .collect()
    }

    const DOC: &str = "\
## Việc cần làm
- [ ] @agent Tạo lịch <!-- id:T1 status:running -->
  - [x] Quét ghi chú
  - [ ] Soạn sự kiện
- [ ] @ngoc Việc khác <!-- id:T2 -->
";

    #[test]
    fn a_plan_is_written_under_its_task() {
        let out = replace_steps(
            "## Việc cần làm\n- [ ] @agent X <!-- id:T1 -->\n",
            1,
            &steps(&[("một", false)]),
        )
        .expect("replace");
        assert!(out.contains("  - [ ] một"), "{out}");
    }

    #[test]
    fn an_existing_plan_is_replaced_not_appended() {
        let out = replace_steps(DOC, 1, &steps(&[("mới", false)])).expect("replace");
        assert!(out.contains("  - [ ] mới"), "{out}");
        assert!(
            !out.contains("Quét ghi chú"),
            "the old plan survived: {out}"
        );
        assert!(!out.contains("Soạn sự kiện"));
    }

    #[test]
    fn the_task_below_is_left_alone() {
        let out = replace_steps(DOC, 1, &steps(&[("mới", true)])).expect("replace");
        assert!(out.contains("- [ ] @ngoc Việc khác"), "{out}");
        assert!(out.contains("## Việc cần làm"));
    }

    #[test]
    fn a_finished_step_is_ticked() {
        let out =
            replace_steps(DOC, 1, &steps(&[("xong", true), ("chưa", false)])).expect("replace");
        assert!(out.contains("  - [x] xong"), "{out}");
        assert!(out.contains("  - [ ] chưa"));
    }

    #[test]
    fn clearing_the_plan_removes_every_step() {
        let out = replace_steps(DOC, 1, &[]).expect("replace");
        assert!(!out.contains("- [x] Quét"), "{out}");
        assert!(out.contains("- [ ] @agent Tạo lịch"));
        assert!(out.contains("- [ ] @ngoc Việc khác"));
    }

    /// The steps round-trip through the parser they will be read back with.
    #[test]
    fn written_steps_parse_back_out() {
        let out = replace_steps(DOC, 1, &steps(&[("một", true), ("hai", false)])).expect("replace");
        let tasks = summo_vault::tasks::parse(&out, "m.md");
        let agent = tasks.iter().find(|t| t.is_agent()).expect("the agent task");
        assert_eq!(agent.steps.len(), 2);
        assert_eq!(agent.steps[0].text, "một");
        assert!(agent.steps[0].done);
        assert!(!agent.steps[1].done);
    }

    #[test]
    fn the_trailing_newline_is_preserved_either_way() {
        assert!(replace_steps(DOC, 1, &[]).unwrap().ends_with('\n'));
        let no_newline = DOC.trim_end();
        assert!(!replace_steps(no_newline, 1, &[]).unwrap().ends_with('\n'));
    }

    #[test]
    fn a_line_past_the_end_is_refused() {
        assert!(replace_steps(DOC, 999, &[]).is_err());
    }

    #[test]
    fn a_blank_line_ends_the_plan() {
        let doc = "- [ ] @agent X <!-- id:T1 -->\n  - [x] cũ\n\n## Phần khác\nvăn xuôi\n";
        let out = replace_steps(doc, 0, &steps(&[("mới", false)])).expect("replace");
        assert!(out.contains("## Phần khác"), "{out}");
        assert!(out.contains("văn xuôi"));
        assert!(!out.contains("cũ"));
    }
}
