//! A page per model, generated from its manifest.
//!
//! Ollama's model pages are the reason people trust `ollama pull` — you can see what you are about
//! to download, who made it, what licence it carries and roughly what it costs before you spend a
//! gigabyte finding out. A registry that is only JSON asks for the same trust and gives none of the
//! same evidence.
//!
//! Everything here is derived from the manifest, so a page cannot drift from the thing it describes
//! — there is no second copy of the size, the licence or the checksums to forget to update.
//!
//! Two things it says out loud that most model pages do not.
//!
//! **Who is distributing this.** A model marked `redistributable: false` is fetched from whoever
//! published it, under their terms; Summo is software that can load the file, the way a media
//! player is software that can open one. A gated model needs the user to accept terms upstream and
//! supply a token. Both are legitimate, and both belong above the download button rather than in a
//! FAQ.
//!
//! **What it actually costs here.** The measured real-time factors in the manifest's profile are
//! ours, taken on named hardware. A page that says "fast" is marketing; a page that says `0.31 on
//! 8 threads of an AVX-512 x86` is a number the reader can check.

use crate::{Manifest, Task};

/// Render one model's page as Markdown.
///
/// `readme` is the upstream project's own README, included verbatim when the registry has a copy
/// beside the manifest. Verbatim and clearly attributed rather than summarised: a paraphrase of
/// somebody else's documentation is how a licence notice quietly loses the sentence that mattered.
#[must_use]
pub fn render(manifest: &Manifest, readme: Option<&str>) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", manifest.name));
    // The id, not a command. This page is read inside the app now, where "run `summo pull x`" is
    // an instruction to open a terminal the reader does not have — and the id is the useful half of
    // that line anyway.
    out.push_str(&format!("`{}`\n\n", manifest.id));

    if let Some(description) = manifest.description.as_deref()
        && !description.trim().is_empty()
    {
        out.push_str(&format!("{}\n\n", description.trim()));
    }

    out.push_str(&facts(manifest));
    out.push_str(&provenance(manifest));
    out.push_str(&performance(manifest));
    out.push_str(&files(manifest));

    if let Some(readme) = readme {
        let readme = readme.trim();
        if !readme.is_empty() {
            out.push_str("## Upstream README\n\n");
            out.push_str(
                "Reproduced verbatim from the project that published these weights. Summo did not \
                 write it and has not edited it.\n\n",
            );
            out.push_str("<!-- upstream:begin -->\n\n");
            out.push_str(readme);
            out.push_str("\n\n<!-- upstream:end -->\n");
        }
    }

    out
}

fn facts(manifest: &Manifest) -> String {
    let langs = if manifest.langs.iter().any(|l| l == "*") {
        "any".to_string()
    } else if manifest.langs.is_empty() {
        "—".to_string()
    } else {
        manifest.langs.join(", ")
    };

    format!(
        "| | |\n|---|---|\n| Task | {task} |\n| Mode | {mode} |\n| Languages | {langs} |\n\
         | Size | {size} |\n| Licence | {licence} |\n| Runtime | `{runtime}` |\n\n",
        task = task_name(manifest.task),
        mode = format!("{:?}", manifest.mode).to_lowercase(),
        langs = langs,
        size = human_bytes(manifest.size_bytes),
        licence = manifest.license,
        runtime = manifest.runtime,
    )
}

/// Who made it, who is handing it to you, and what you have to agree to first.
fn provenance(manifest: &Manifest) -> String {
    let mut out = String::from("## Where this comes from\n\n");

    match &manifest.attribution {
        Some(who) => out.push_str(&format!("Published by {who}.\n\n")),
        None => out.push_str(
            "The manifest names no upstream author, which is a gap in the registry rather than a \
             claim that Summo made this.\n\n",
        ),
    }

    if manifest.gated {
        out.push_str(
            "**Gated.** The publisher serves these files only after you have accepted their terms \
             on their own site. Summo will ask for an access token and send it to that host and \
             nowhere else.\n\n",
        );
    }

    if manifest.redistributable {
        out.push_str(
            "Redistributable under its licence, so Summo mirrors it. The checksums below are what \
             you should get from either source.\n\n",
        );
    } else {
        out.push_str(
            "**Not redistributed by Summo.** The download goes to the original host under that \
             project's own terms. Summo is the software that can load the file; it is not the \
             distributor of it.\n\n",
        );
    }

    out
}

/// What it costs on real hardware, or an honest admission that nobody has measured it.
fn performance(manifest: &Manifest) -> String {
    let profile = &manifest.profile;
    let mut out = String::from("## What it costs\n\n");

    out.push_str(&format!(
        "Memory: about {} MB resident, {} MB at peak. Needs at least {} MB free.\n\n",
        profile.rss_mb.idle, profile.rss_mb.peak, profile.min_ram_mb
    ));

    if profile.rtf.is_empty() {
        out.push_str(
            "No real-time factor has been measured yet. Rather than guess, the registry says so — \
             run `summo-bench` and send the numbers.\n\n",
        );
    } else {
        out.push_str("Real-time factor, measured — below 1.0 keeps up with live audio:\n\n");
        out.push_str("| Machine | RTF |\n|---|---|\n");
        let mut rows: Vec<_> = profile.rtf.iter().collect();
        rows.sort_by_key(|(k, _)| k.as_str());
        for (machine, rtf) in rows {
            out.push_str(&format!("| `{machine}` | {rtf:.3} |\n"));
        }
        out.push('\n');
    }

    if !profile.quality.is_empty() {
        // Accuracy, in the same shape as the speed table above: a number somebody can check
        // against another model, with the benchmark it came from named. `wer_fleurs_vi` is word
        // error rate on FLEURS Vietnamese — lower is better — and the reader should be told that
        // rather than left to infer it from the key.
        out.push_str("Accuracy, measured — word error rate, lower is better:\n\n");
        out.push_str("| Benchmark | Score |\n|---|---|\n");
        let mut rows: Vec<_> = profile.quality.iter().collect();
        rows.sort_by_key(|(k, _)| k.as_str());
        for (benchmark, score) in rows {
            out.push_str(&format!("| `{benchmark}` | {:.1}% |\n", score * 100.0));
        }
        out.push('\n');
    }

    if !profile.accel.is_empty() {
        out.push_str(&format!(
            "Accelerators: {}.\n\n",
            profile
                .accel
                .iter()
                .map(|a| format!("{a:?}").to_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    out
}

/// The checksums, so a download can be verified by hand.
fn files(manifest: &Manifest) -> String {
    if manifest.files.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Files\n\n| Name | Size | sha256 |\n|---|---|---|\n");
    for file in &manifest.files {
        out.push_str(&format!(
            "| `{}` | {} | `{}` |\n",
            file.name,
            human_bytes(file.size),
            file.sha256
        ));
    }
    out.push('\n');
    out
}

/// A task as a person would say it, not as the enum spells it.
///
/// `{:?}` gives `SpeakerEmbed`, and lowercasing that gives `speakerembed` — a word that exists in
/// no language. Exported so the index and the page agree.
#[must_use]
pub fn task_name(task: Task) -> &'static str {
    match task {
        Task::Asr => "speech recognition",
        Task::Vad => "voice activity detection",
        Task::SpeakerEmbed => "speaker embedding",
        Task::Denoise => "noise suppression",
        Task::DiarizeSeg => "speaker segmentation",
        Task::Embed => "text embedding",
        Task::Translate => "translation",
    }
}

/// Bytes as a person reads them. Binary units, matching what a download manager shows.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{} {}", value.round() as u64, UNITS[unit])
    }
}

/// The filename a model's page is written to.
#[must_use]
pub fn file_name(manifest: &Manifest) -> String {
    format!("{}.md", manifest.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileEntry, Mode, Profile};
    use summo_core::ModelId;

    fn manifest() -> Manifest {
        Manifest {
            schema: 1,
            id: ModelId::parse("whisper-tiny").unwrap(),
            name: "Whisper tiny".into(),
            task: Task::Asr,
            mode: Mode::Batch,
            runtime: "sherpa-onnx/whisper".into(),
            langs: vec!["vi".into(), "en".into()],
            domains: vec![],
            license: "MIT".into(),
            attribution: Some("OpenAI".into()),
            redistributable: true,
            gated: false,
            size_bytes: 75_000_000,
            description: Some("Small multilingual recogniser.".into()),
            profile: Profile::default(),
            files: vec![FileEntry {
                name: "model.onnx".into(),
                sha256: "abc123".into(),
                size: 75_000_000,
                url: "https://example.invalid/model.onnx".into(),
                mirror: vec![],
                platform: None,
                variant: None,
            }],
            variants: Vec::new(),
            installed_variant: None,
            params: Default::default(),
        }
    }

    #[test]
    fn a_page_leads_with_the_model_id_rather_than_a_command() {
        // The page is read inside the app now, where "run `summo pull whisper-tiny`" is an
        // instruction to open a terminal that is not there. The id is the half of that line a
        // reader can use — to recognise the model, to search for it, to type it into a config.
        let page = render(&manifest(), None);
        assert!(page.contains("`whisper-tiny`"), "{page}");
        assert!(
            !page.contains("summo pull"),
            "no terminal commands on a page in a GUI:\n{page}"
        );
    }

    #[test]
    fn the_licence_and_the_author_are_on_the_page() {
        let page = render(&manifest(), None);
        assert!(page.contains("MIT"));
        assert!(page.contains("OpenAI"));
    }

    /// The distinction the whole registry policy rests on. A reader has to be able to see which one
    /// they are agreeing to before they click, not after.
    #[test]
    fn a_model_summo_does_not_redistribute_says_so_plainly() {
        let mut m = manifest();
        m.redistributable = false;
        let page = render(&m, None);
        assert!(page.contains("Not redistributed by Summo"), "{page}");
        assert!(page.contains("original host"));
    }

    #[test]
    fn a_mirrored_model_says_that_instead() {
        let page = render(&manifest(), None);
        assert!(page.contains("Summo mirrors it"));
        assert!(!page.contains("Not redistributed"));
    }

    #[test]
    fn a_gated_model_warns_before_the_download_not_after() {
        let mut m = manifest();
        m.gated = true;
        let page = render(&m, None);
        assert!(page.contains("Gated"), "{page}");
        assert!(page.contains("token"));
    }

    /// The token is a credential. A page describing a gated model must not suggest it travels
    /// anywhere except the host that gates the file.
    #[test]
    fn the_gated_notice_says_where_the_token_goes() {
        let mut m = manifest();
        m.gated = true;
        assert!(render(&m, None).contains("nowhere else"));
    }

    /// A missing measurement is a fact about the registry, not something to paper over with an
    /// adjective.
    #[test]
    fn an_unmeasured_model_admits_it_rather_than_claiming_to_be_fast() {
        let page = render(&manifest(), None);
        assert!(
            page.contains("No real-time factor has been measured"),
            "{page}"
        );
        assert!(!page.to_lowercase().contains("blazing"));
    }

    #[test]
    fn measured_numbers_are_printed_with_the_machine_they_came_from() {
        let mut m = manifest();
        m.profile.rtf.insert("cpu_x86_avx512vnni_8t".into(), 0.312);
        let page = render(&m, None);
        assert!(page.contains("cpu_x86_avx512vnni_8t"), "{page}");
        assert!(page.contains("0.312"), "{page}");
    }

    #[test]
    fn the_checksums_are_on_the_page_so_a_download_can_be_verified_by_hand() {
        let page = render(&manifest(), None);
        assert!(page.contains("abc123"));
        assert!(page.contains("model.onnx"));
    }

    /// Paraphrasing somebody else's documentation is how a licence notice quietly loses the
    /// sentence that mattered.
    #[test]
    fn an_upstream_readme_is_included_verbatim_and_marked_as_theirs() {
        let page = render(&manifest(), Some("# Their project\n\nTheir words."));
        assert!(page.contains("Their words."));
        assert!(page.contains("verbatim"));
        assert!(page.contains("<!-- upstream:begin -->"));
    }

    #[test]
    fn an_empty_readme_adds_no_empty_section() {
        let page = render(&manifest(), Some("   \n  "));
        assert!(!page.contains("Upstream README"));
    }

    #[test]
    fn a_model_for_every_language_says_so_rather_than_printing_a_star() {
        let mut m = manifest();
        m.langs = vec!["*".into()];
        assert!(render(&m, None).contains("| Languages | any |"));
    }

    #[test]
    fn sizes_read_as_a_person_would_write_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(75_000_000), "72 MB");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn a_page_is_named_after_the_model_id_which_is_what_a_client_requests() {
        assert_eq!(file_name(&manifest()), "whisper-tiny.md");
    }

    /// Generated from the manifest and nothing else, so a page cannot drift from what it describes.
    #[test]
    fn nothing_on_the_page_is_hardcoded_per_model() {
        let mut m = manifest();
        m.name = "Something Else".into();
        m.license = "Apache-2.0".into();
        let page = render(&m, None);
        assert!(page.contains("Something Else"));
        assert!(page.contains("Apache-2.0"));
        assert!(!page.contains("Whisper tiny"));
    }
}
