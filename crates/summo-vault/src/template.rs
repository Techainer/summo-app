//! What shape a summary should take.
//!
//! A standup, a customer call and a job interview want different write-ups, and no single prompt
//! serves all three. A template is a Markdown file the user can open and edit — the same rule as
//! ADR 0002 — holding the sections to produce and the instruction for each.
//!
//! ```markdown
//! ---
//! name: Họp tuần
//! language: vi
//! match: [weekly, standup]
//! ---
//! ## Tóm tắt
//! Hai đến ba câu về mục đích buổi họp.
//!
//! ## Quyết định
//! Mỗi quyết định một gạch đầu dòng. Bỏ qua mục này nếu không chốt được gì.
//!
//! ## Việc cần làm
//! `- [ ] @người — việc — hạn`. Chỉ ghi việc có người thực sự nhận.
//! ```
//!
//! The body under each heading is an instruction to the model, not text to copy. That is why a
//! template reads like a brief rather than a form: the thing being configured is what to ask for,
//! and a user who wants a different section can write one in their own words without touching any
//! code.
//!
//! Four ship by default. They are written to disk on first run rather than compiled in, so the way
//! to change one is to edit it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

/// One section the summary should contain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub heading: String,
    /// What the model should put under it.
    pub instruction: String,
}

/// A named summary shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    /// Slug from the file name, e.g. `weekly`.
    pub id: String,
    pub name: String,
    /// Language to write in. Empty means "follow the transcript".
    #[serde(default)]
    pub language: String,
    /// Tags or title words that suggest this template. Used to pick one automatically.
    #[serde(default)]
    pub match_on: Vec<String>,
    pub sections: Vec<Section>,
}

impl Template {
    /// The instruction block handed to the model, built from the sections.
    ///
    /// Deliberately restates the heading text verbatim so the model writes headings the vault can
    /// parse back out — `MeetingDoc::set_section` matches on them.
    #[must_use]
    pub fn instructions(&self) -> String {
        let mut out = String::from(
            "Structure the summary with exactly these sections, in order, \
                                    using these headings verbatim:\n",
        );
        for section in &self.sections {
            out.push_str(&format!(
                "\n## {}\n{}\n",
                section.heading, section.instruction
            ));
        }
        out.push_str(
            "\nOmit a section entirely if the transcript contains nothing for it, rather than \
             writing that there is nothing.",
        );
        out
    }

    /// How well this template fits a meeting, by tags and title. Zero means no signal.
    #[must_use]
    pub fn score(&self, title: &str, tags: &[String]) -> usize {
        let title = crate::index::fold(title);
        self.match_on
            .iter()
            .filter(|needle| {
                let needle = crate::index::fold(needle);
                tags.iter().any(|t| crate::index::fold(t) == needle) || title.contains(&needle)
            })
            .count()
    }
}

/// Every template on disk, plus the ability to pick one.
#[derive(Debug, Clone, Default)]
pub struct Templates {
    items: Vec<Template>,
}

impl Templates {
    /// Read `dir`, writing the built-in set first if it is empty.
    ///
    /// Seeding on first run rather than compiling the defaults in is what makes them editable: the
    /// user changes a heading by opening a file, not by finding a setting.
    pub fn load_or_seed(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let mut items = read_dir(dir)?;
        if items.is_empty() {
            for (id, markdown) in BUILT_IN {
                let path = dir.join(format!("{id}.md"));
                std::fs::write(&path, markdown).map_err(|e| Error::io(&path, e))?;
            }
            items = read_dir(dir)?;
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { items })
    }

    #[must_use]
    pub fn all(&self) -> &[Template] {
        &self.items
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Template> {
        self.items.iter().find(|t| t.id == id)
    }

    /// The best fit for a meeting, falling back to `standard`.
    ///
    /// Ties break towards the earlier template by name so the choice does not flicker between runs
    /// on a meeting that matches two equally well.
    #[must_use]
    pub fn best_for(&self, title: &str, tags: &[String]) -> Option<&Template> {
        let best = self
            .items
            .iter()
            .map(|t| (t.score(title, tags), t))
            .filter(|(score, _)| *score > 0)
            .max_by_key(|(score, _)| *score);
        best.map(|(_, t)| t)
            .or_else(|| self.get("standard"))
            .or_else(|| self.items.first())
    }
}

fn read_dir(dir: &Path) -> Result<Vec<Template>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        match parse(id, &text) {
            Ok(template) => out.push(template),
            // One malformed template must not stop the others being offered.
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping a template"),
        }
    }
    Ok(out)
}

/// Parse a template file: optional YAML frontmatter, then `##` sections.
pub fn parse(id: &str, markdown: &str) -> Result<Template> {
    let (front, body) = split_frontmatter(markdown);
    let meta: Meta = if front.is_empty() {
        Meta::default()
    } else {
        serde_yaml::from_str(front)
            .map_err(|e| Error::Other(format!("template {id} has bad frontmatter: {e}")))?
    };

    let mut sections = Vec::new();
    let mut heading: Option<String> = None;
    let mut buffer = String::new();

    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(previous) = heading.take() {
                sections.push(Section {
                    heading: previous,
                    instruction: buffer.trim().to_string(),
                });
            }
            heading = Some(rest.trim().to_string());
            buffer.clear();
        } else if heading.is_some() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    if let Some(previous) = heading {
        sections.push(Section {
            heading: previous,
            instruction: buffer.trim().to_string(),
        });
    }

    if sections.is_empty() {
        return Err(Error::Other(format!(
            "template {id} has no `## ` sections, so it describes no summary"
        )));
    }

    Ok(Template {
        id: id.to_string(),
        name: meta.name.unwrap_or_else(|| id.to_string()),
        language: meta.language.unwrap_or_default(),
        match_on: meta.match_on,
        sections,
    })
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    name: Option<String>,
    language: Option<String>,
    #[serde(default, rename = "match")]
    match_on: Vec<String>,
    /// Anything else in the frontmatter is kept out of the way rather than being an error.
    #[serde(flatten)]
    #[allow(dead_code)]
    rest: BTreeMap<String, serde_yaml::Value>,
}

fn split_frontmatter(markdown: &str) -> (&str, &str) {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return ("", markdown);
    };
    match rest.split_once("\n---\n") {
        Some((front, body)) => (front, body),
        None => ("", markdown),
    }
}

/// The templates written on first run.
const BUILT_IN: &[(&str, &str)] = &[
    ("standard", include_str!("templates/standard.md")),
    ("standup", include_str!("templates/standup.md")),
    ("one-on-one", include_str!("templates/one-on-one.md")),
    ("sales", include_str!("templates/sales.md")),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_template_parses_into_sections() {
        let template = parse(
            "weekly",
            "---\nname: Họp tuần\nlanguage: vi\nmatch: [weekly, standup]\n---\n\
             ## Tóm tắt\nHai ba câu.\n\n## Việc cần làm\nMỗi việc một dòng.\n",
        )
        .expect("parse");

        assert_eq!(template.id, "weekly");
        assert_eq!(template.name, "Họp tuần");
        assert_eq!(template.language, "vi");
        assert_eq!(template.match_on, vec!["weekly", "standup"]);
        assert_eq!(template.sections.len(), 2);
        assert_eq!(template.sections[0].heading, "Tóm tắt");
        assert_eq!(template.sections[0].instruction, "Hai ba câu.");
        assert_eq!(template.sections[1].heading, "Việc cần làm");
    }

    #[test]
    fn frontmatter_is_optional() {
        let template = parse("x", "## A\nDo a thing.\n").expect("parse");
        assert_eq!(template.name, "x", "the id stands in for a missing name");
        assert_eq!(template.sections.len(), 1);
    }

    /// A file with no sections describes no summary, which is a mistake worth reporting.
    #[test]
    fn a_template_without_sections_is_an_error() {
        assert!(parse("x", "just some prose\n").is_err());
    }

    #[test]
    fn bad_frontmatter_is_reported_rather_than_ignored() {
        let err = parse("x", "---\nname: [unclosed\n---\n## A\nb\n").unwrap_err();
        assert!(err.to_string().contains("frontmatter"), "{err}");
    }

    #[test]
    fn instructions_restate_the_headings_verbatim() {
        // The vault parses summaries back out by heading, so the model must write them exactly.
        let template = parse("x", "## Quyết định\nMỗi quyết định một dòng.\n").expect("parse");
        let instructions = template.instructions();
        assert!(instructions.contains("## Quyết định"), "{instructions}");
        assert!(instructions.contains("Mỗi quyết định một dòng."));
    }

    #[test]
    fn a_template_scores_on_tags_and_title() {
        let template = parse("x", "---\nmatch: [standup]\n---\n## A\nb\n").expect("parse");
        assert_eq!(template.score("Weekly sync", &["standup".into()]), 1);
        assert_eq!(template.score("Daily standup", &[]), 1);
        assert_eq!(template.score("Sales call", &["customer".into()]), 0);
    }

    /// Vietnamese users write tags with and without diacritics; both should match.
    #[test]
    fn matching_folds_diacritics() {
        let template = parse("x", "---\nmatch: [\"họp tuần\"]\n---\n## A\nb\n").expect("parse");
        assert_eq!(template.score("Hop tuan 32", &[]), 1);
        assert_eq!(template.score("", &["Họp Tuần".into()]), 1);
    }

    #[test]
    fn first_run_seeds_the_built_in_templates() {
        let dir = TempDir::new().unwrap();
        let templates = Templates::load_or_seed(dir.path()).expect("seed");

        assert!(templates.all().len() >= 4, "expected the built-in set");
        assert!(templates.get("standard").is_some());
        assert!(
            dir.path().join("standard.md").exists(),
            "seeded to disk, so it is editable"
        );
    }

    #[test]
    fn an_edited_template_is_not_overwritten_on_the_next_run() {
        let dir = TempDir::new().unwrap();
        Templates::load_or_seed(dir.path()).expect("seed");
        std::fs::write(dir.path().join("standard.md"), "## Của tôi\nTheo ý tôi.\n").unwrap();

        let reloaded = Templates::load_or_seed(dir.path()).expect("reload");
        let standard = reloaded.get("standard").expect("still there");
        assert_eq!(standard.sections[0].heading, "Của tôi");
    }

    #[test]
    fn a_broken_template_does_not_hide_the_others() {
        let dir = TempDir::new().unwrap();
        Templates::load_or_seed(dir.path()).expect("seed");
        std::fs::write(dir.path().join("broken.md"), "no sections here\n").unwrap();

        let templates = Templates::load_or_seed(dir.path()).expect("reload");
        assert!(templates.get("standard").is_some());
        assert!(templates.get("broken").is_none());
    }

    #[test]
    fn the_best_template_is_chosen_by_tag() {
        let dir = TempDir::new().unwrap();
        let templates = Templates::load_or_seed(dir.path()).expect("seed");
        let chosen = templates
            .best_for("Daily standup", &["standup".into()])
            .expect("a template");
        assert_eq!(chosen.id, "standup");
    }

    #[test]
    fn a_meeting_that_matches_nothing_falls_back_to_standard() {
        let dir = TempDir::new().unwrap();
        let templates = Templates::load_or_seed(dir.path()).expect("seed");
        let chosen = templates.best_for("Random chat", &[]).expect("a template");
        assert_eq!(chosen.id, "standard");
    }
}
