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

/// The words around the numbers, in one language.
///
/// The numbers, the checksums and the upstream README are the same in every language; the sentences
/// that introduce them are not. This page is read inside the app — a Vietnamese app, mostly — and it
/// was English throughout, so somebody comparing two models was reading "Where this comes from"
/// under a Vietnamese heading that said Chi tiết.
///
/// Fixed strings rather than a translation framework: there are fifteen of them, they change when
/// this file changes, and a `.json` catalogue two crates away is how they would drift apart.
pub struct Words {
    pub facts: [&'static str; 6],
    pub any_language: &'static str,
    pub provenance: &'static str,
    pub published_by: &'static str,
    pub no_author: &'static str,
    pub gated: &'static str,
    pub mirrored: &'static str,
    pub not_mirrored: &'static str,
    pub cost: &'static str,
    pub memory: &'static str,
    pub no_rtf: &'static str,
    pub rtf: &'static str,
    pub rtf_head: [&'static str; 2],
    pub accuracy: &'static str,
    pub accuracy_head: [&'static str; 2],
    pub accel: &'static str,
    pub files: &'static str,
    pub files_head: [&'static str; 3],
    pub readme: &'static str,
    pub readme_note: &'static str,
}

/// English, which is what the CLI and the generated documentation use.
pub const EN: Words = Words {
    facts: ["Task", "Mode", "Languages", "Size", "Licence", "Runtime"],
    any_language: "any",
    provenance: "Where this comes from",
    published_by: "Published by",
    no_author: "The manifest names no upstream author, which is a gap in the registry rather than a claim that Summo made this.",
    gated: "**Gated.** The publisher serves these files only after you have accepted their terms on their own site. Summo will ask for an access token and send it to that host and nowhere else.",
    mirrored: "Redistributable under its licence, so Summo mirrors it. The checksums below are what you should get from either source.",
    not_mirrored: "**Not redistributed by Summo.** The download goes to the original host under that project's own terms. Summo is the software that can load the file; it is not the distributor of it.",
    cost: "What it costs",
    memory: "Memory: about {idle} MB resident, {peak} MB at peak. Needs at least {min} MB free.",
    no_rtf: "Nobody has measured how fast this runs yet, and the registry says so rather than guessing.",
    rtf: "Real-time factor, measured — below 1.0 keeps up with live audio:",
    rtf_head: ["Machine", "RTF"],
    accuracy: "Accuracy, measured — word error rate, lower is better:",
    accuracy_head: ["Benchmark", "Score"],
    accel: "Accelerators",
    files: "Files",
    files_head: ["Name", "Size", "sha256"],
    readme: "From the publisher",
    readme_note: "Reproduced verbatim from the project that published these weights. Summo did not write it and has not edited it.",
};

/// Vietnamese, which is what most people reading this in the app are reading everything else in.
pub const VI: Words = Words {
    facts: [
        "Việc",
        "Chế độ",
        "Ngôn ngữ",
        "Dung lượng",
        "Giấy phép",
        "Runtime",
    ],
    any_language: "mọi ngôn ngữ",
    provenance: "Mô hình này từ đâu",
    published_by: "Phát hành bởi",
    no_author: "Manifest không ghi tác giả gốc. Đó là thiếu sót của kho mô hình, không phải Summo tự làm ra mô hình này.",
    gated: "**Cần chấp thuận điều khoản.** Nơi phát hành chỉ cho tải sau khi bạn đồng ý điều khoản trên trang của họ. Summo sẽ hỏi token và chỉ gửi token đó tới đúng nơi phát hành.",
    mirrored: "Giấy phép cho phép phân phối lại, nên Summo có bản sao. Checksum bên dưới đúng cho cả hai nguồn.",
    not_mirrored: "**Summo không phân phối lại mô hình này.** File tải thẳng từ nơi phát hành, theo điều khoản của họ.",
    cost: "Chi phí trên máy",
    memory: "Bộ nhớ: khoảng {idle} MB khi chạy, {peak} MB lúc cao nhất. Cần tối thiểu {min} MB trống.",
    no_rtf: "Chưa ai đo tốc độ của mô hình này, và kho mô hình nói thẳng như vậy thay vì đoán.",
    rtf: "Tốc độ đã đo — dưới 1.0 là theo kịp âm thanh trực tiếp:",
    rtf_head: ["Máy", "RTF"],
    accuracy: "Độ chính xác đã đo — tỉ lệ lỗi từ, càng thấp càng tốt:",
    accuracy_head: ["Bộ đo", "Kết quả"],
    accel: "Tăng tốc phần cứng",
    files: "File",
    files_head: ["Tên", "Dung lượng", "sha256"],
    readme: "Mô tả của nơi phát hành",
    readme_note: "Giữ nguyên văn từ dự án phát hành trọng số này. Summo không viết và không sửa.",
};

/// The words for a language tag, falling back to English for anything else.
///
/// Japanese and Chinese fall back rather than getting a machine translation of a licensing position
/// nobody checked. Wrong-language English is a smaller failure than confident wrong Japanese.
#[must_use]
pub fn words_for(lang: &str) -> &'static Words {
    if lang.trim().to_ascii_lowercase().starts_with("vi") {
        &VI
    } else {
        &EN
    }
}

/// Render one model's page as Markdown, in English.
///
/// `readme` is the upstream project's own README, included verbatim when the registry has a copy
/// beside the manifest. Verbatim and clearly attributed rather than summarised: a paraphrase of
/// somebody else's documentation is how a licence notice quietly loses the sentence that mattered.
#[must_use]
pub fn render(manifest: &Manifest, readme: Option<&str>) -> String {
    render_in(manifest, readme, &EN)
}

/// Render one model's page as Markdown, in the given language.
#[must_use]
pub fn render_in(manifest: &Manifest, readme: Option<&str>, words: &Words) -> String {
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

    out.push_str(&facts(manifest, words));
    out.push_str(&provenance(manifest, words));
    out.push_str(&performance(manifest, words));
    out.push_str(&files(manifest, words));

    if let Some(readme) = readme {
        let readme = readme.trim();
        if !readme.is_empty() {
            out.push_str(&format!("## {}\n\n", words.readme));
            out.push_str(words.readme_note);
            out.push_str("\n\n");
            out.push_str("<!-- upstream:begin -->\n\n");
            out.push_str(readme);
            out.push_str("\n\n<!-- upstream:end -->\n");
        }
    }

    out
}

fn facts(manifest: &Manifest, words: &Words) -> String {
    let langs = if manifest.langs.iter().any(|l| l == "*") {
        words.any_language.to_string()
    } else if manifest.langs.is_empty() {
        "—".to_string()
    } else {
        manifest.langs.join(", ")
    };

    let [
        task_label,
        mode_label,
        langs_label,
        size_label,
        licence_label,
        runtime_label,
    ] = words.facts;
    format!(
        "| | |\n|---|---|\n| {task_label} | {task} |\n| {mode_label} | {mode} |\n\
         | {langs_label} | {langs} |\n| {size_label} | {size} |\n| {licence_label} | {licence} |\n\
         | {runtime_label} | `{runtime}` |\n\n",
        task = task_name(manifest.task),
        mode = format!("{:?}", manifest.mode).to_lowercase(),
        langs = langs,
        size = human_bytes(manifest.size_bytes),
        licence = manifest.license,
        runtime = manifest.runtime,
    )
}

/// Who made it, who is handing it to you, and what you have to agree to first.
fn provenance(manifest: &Manifest, words: &Words) -> String {
    let mut out = format!("## {}\n\n", words.provenance);

    match &manifest.attribution {
        Some(who) => out.push_str(&format!("{} {who}.\n\n", words.published_by)),
        None => {
            out.push_str(words.no_author);
            out.push_str("\n\n");
        }
    }

    if manifest.gated {
        out.push_str(words.gated);
        out.push_str("\n\n");
    }

    out.push_str(if manifest.redistributable {
        words.mirrored
    } else {
        words.not_mirrored
    });
    out.push_str("\n\n");

    out
}

/// What it costs on real hardware, or an honest admission that nobody has measured it.
fn performance(manifest: &Manifest, words: &Words) -> String {
    let profile = &manifest.profile;
    let mut out = format!("## {}\n\n", words.cost);

    out.push_str(
        &words
            .memory
            .replace("{idle}", &profile.rss_mb.idle.to_string())
            .replace("{peak}", &profile.rss_mb.peak.to_string())
            .replace("{min}", &profile.min_ram_mb.to_string()),
    );
    out.push_str("\n\n");

    if profile.rtf.is_empty() {
        // No instruction to run a benchmark binary. This page is read inside the app, where "run
        // `summo-bench` and send the numbers" is a task for somebody who already has a checkout.
        out.push_str(words.no_rtf);
        out.push_str("\n\n");
    } else {
        out.push_str(words.rtf);
        out.push_str("\n\n");
        out.push_str(&format!(
            "| {} | {} |\n|---|---|\n",
            words.rtf_head[0], words.rtf_head[1]
        ));
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
        out.push_str(words.accuracy);
        out.push_str("\n\n");
        out.push_str(&format!(
            "| {} | {} |\n|---|---|\n",
            words.accuracy_head[0], words.accuracy_head[1]
        ));
        let mut rows: Vec<_> = profile.quality.iter().collect();
        rows.sort_by_key(|(k, _)| k.as_str());
        for (benchmark, score) in rows {
            out.push_str(&format!("| `{benchmark}` | {:.1}% |\n", score * 100.0));
        }
        out.push('\n');
    }

    if !profile.accel.is_empty() {
        out.push_str(&format!(
            "{}: {}.\n\n",
            words.accel,
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
fn files(manifest: &Manifest, words: &Words) -> String {
    if manifest.files.is_empty() {
        return String::new();
    }

    let mut out = format!(
        "## {}\n\n| {} | {} | {} |\n|---|---|---|\n",
        words.files, words.files_head[0], words.files_head[1], words.files_head[2]
    );
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
        Task::Tts => "speech synthesis",
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
                archive: None,
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
            page.contains("Nobody has measured how fast this runs"),
            "{page}"
        );
        assert!(!page.to_lowercase().contains("blazing"));
        // And no instruction to run a benchmark binary. This page is read inside the app, where
        // "run `summo-bench` and send the numbers" is a task for somebody with a checkout.
        assert!(!page.contains("summo-bench"), "{page}");
    }

    #[test]
    fn the_words_follow_the_reader_and_the_numbers_do_not() {
        let vi = render_in(&manifest(), None, words_for("vi-VN"));
        let en = render_in(&manifest(), None, words_for("en"));

        assert!(vi.contains("## Mô hình này từ đâu"), "{vi}");
        assert!(vi.contains("Phát hành bởi OpenAI."), "{vi}");
        assert!(!vi.contains("Where this comes from"), "{vi}");
        assert!(en.contains("## Where this comes from"), "{en}");

        // The facts themselves are language-independent, and both pages have to carry them.
        for page in [&vi, &en] {
            assert!(page.contains("whisper-tiny"), "{page}");
            assert!(page.contains("abc123"), "{page}");
            assert!(page.contains("MIT"), "{page}");
        }
    }

    #[test]
    fn a_language_with_no_words_of_its_own_reads_english_rather_than_a_guess() {
        let ja = render_in(&manifest(), None, words_for("ja"));
        assert!(ja.contains("## Where this comes from"), "{ja}");
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
