//! Summarising a real meeting file against a socket, because the seam between the two was untested.
//!
//! Both ends of this path are covered and the wire between them was not. `summo_llm::prompt` has
//! twenty-one tests for building the request; `summo_llm::provider` has thirty for resolving an
//! endpoint; `tests/http.rs` has nine driving the client against a real socket, including
//! server-sent events split mid-line. And `summarize.rs` has thirteen for turning a response into a
//! document — every one of which is handed a **hand-written response string**.
//!
//! Nothing called [`summarize::run`], which is the function that does everything in between: find
//! the file, render the transcript, read the notes out of the document, refuse to spend a request
//! on a meeting too short to summarise, load and choose a template, resolve which of three places
//! decides the language, build the prompt, send it, apply what came back, and write the file
//! atomically. Nor [`summarize::spawn`], which is the path the daemon takes on its own after every
//! recording stops.
//!
//! The browser suites do not cover it either: `e2e/draft.mjs` writes the `summo:draft` marker into
//! the file by hand, precisely because nothing in the suite can produce one.
//!
//! These tests assert on the bytes sent and the file afterwards. A stub rather than a real
//! provider, and not because a real one is unavailable — a test that needs somebody's gateway to
//! have credit on it is a test CI cannot run.

mod stub;

use summo_core::{
    MeetingId,
    segment::{Lane, Segment},
};
use summo_engine::summarize;
use summo_llm::{LlmClient, Provider};
use summo_vault::{MeetingDoc, meeting::Frontmatter};

/// A vault holding one meeting, and the id to ask for.
///
/// Written through `summo_vault` rather than as a string literal, so the test reads the file the
/// product writes — a hand-rolled fixture would be testing the parser against my idea of the
/// format rather than against the writer.
fn vault_with(lines: &[&str]) -> (tempfile::TempDir, summo_core::paths::Paths, MeetingId) {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let id = MeetingId::new();

    let mut doc = MeetingDoc::new(
        Frontmatter::new(id.clone(), "2026-09-14"),
        "Họp ngân sách quý bốn",
    );
    for (i, text) in lines.iter().enumerate() {
        let t = i as f64 * 20.0;
        doc.transcript
            .push(Segment::new(i as u64, Lane::System, *text, t, t + 15.0));
    }

    let meetings = paths.vault().join("meetings");
    std::fs::create_dir_all(&meetings).unwrap();
    std::fs::write(
        meetings.join("2026-09-14-hop-ngan-sach.md"),
        doc.to_markdown().unwrap(),
    )
    .unwrap();

    (dir, paths, id)
}

/// Enough material to get past the "too little here to summarise" gate.
fn a_real_meeting() -> Vec<&'static str> {
    vec![
        "Hôm nay mình chốt ngân sách quý bốn cho đội kỹ thuật, có mấy khoản cần xem lại.",
        "Em đề xuất giữ nguyên mức quý ba, khoảng hai tỷ tư, vì đội chưa tuyển thêm ai.",
        "Phần hạ tầng đang vượt, tháng trước đội lên ba trăm triệu so với dự toán ban đầu.",
        "Em sẽ rà lại hợp đồng máy chủ trước thứ sáu và gửi lại con số cho cả nhóm xem.",
        "Được, chốt vậy đi. Bình lo phần báo giá của nhà cung cấp mới, xong trong tuần này.",
        "Nếu rẻ hơn hai mươi phần trăm thì mình chuyển, không thì giữ nguyên nhà cũ.",
    ]
}

fn client(addr: std::net::SocketAddr) -> LlmClient {
    LlmClient::new(Provider::custom(
        "stub",
        &format!("http://{addr}"),
        "test-model",
    ))
    .unwrap()
}

/// The reply a cooperative model gives: the template's headings, verbatim.
const ANSWER: &str = "## Tóm tắt\nChốt giữ ngân sách quý bốn ở mức quý ba.\n\n\
                      ## Quyết định\n- Giữ mức hai tỷ tư [t=00:20]\n\n\
                      ## Việc cần làm\n- [ ] @Ngọc — rà hợp đồng máy chủ — thứ sáu\n";

/// The whole path, once: file in, request out, file changed.
#[tokio::test]
async fn a_summary_reaches_the_meeting_file_it_was_asked_about() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let done = summarize::run(&paths, &client(addr), &id, None)
        .await
        .expect("a meeting with six lines of transcript is summarisable");

    assert_eq!(done.template, "standard");
    assert_eq!(done.sections, ["Tóm tắt", "Quyết định", "Việc cần làm"]);

    // On disk, not only in the returned value. `run` writes atomically and a caller that trusted
    // the return would not notice a write that never happened.
    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("## Tóm tắt"), "{after}");
    assert!(
        after.contains("Chốt giữ ngân sách quý bốn"),
        "the model's text is not in the file: {after}"
    );
    // And the transcript survived. `apply` edits sections by heading, and a bug there costs the
    // recording rather than the summary.
    assert!(
        after.contains("hai tỷ tư"),
        "the transcript was lost: {after}"
    );

    // One request, and the transcript was actually in it. A prompt built from an empty render
    // produces a plausible summary of nothing.
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one meeting, one request");
    assert!(
        seen[0].contains("hạ tầng đang vượt"),
        "the transcript did not reach the model"
    );
    assert!(
        seen[0].contains("Tóm tắt"),
        "the template's headings did not reach the model"
    );
}

/// Notes the user typed during the meeting go with the transcript.
///
/// Two inputs with separate labels is the whole point of the note editor, and the only thing that
/// proves the notes were read is what arrived on the wire.
#[tokio::test]
async fn what_the_user_typed_during_the_meeting_is_sent_too() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let mut doc = summo_vault::open(&paths.vault(), &path).unwrap();
    doc.set_section(
        summo_vault::meeting::NOTES_HEADING,
        "Khách hỏi về SLA, chưa ai trả lời.",
    );
    std::fs::write(&path, doc.to_markdown().unwrap()).unwrap();

    summarize::run(&paths, &client(addr), &id, None)
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    assert!(
        seen[0].contains("SLA"),
        "the note the user typed never reached the model"
    );
}

/// A meeting too short to be worth a request does not become one.
///
/// The gate is on transcript *plus* notes, and it matters in both directions: this is the paid
/// request nobody wants for a ten-second recording, and — see `MIN_CHARACTERS` — a short recording
/// with dense notes is still worth summarising.
#[tokio::test]
async fn a_meeting_with_nothing_in_it_costs_no_request() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&["Ừ.", "Chào."]);

    let err = summarize::run(&paths, &client(addr), &id, None)
        .await
        .expect_err("two words were summarised");
    assert!(err.to_string().contains("too little"), "{err}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "a request was sent for a meeting that was refused"
    );
}

/// A model that answers with prose rather than the headings is a failure, not an empty summary.
///
/// The dangerous shape: `apply` writes nothing, the file is left alone, and a caller that ignored
/// the result would report success over a meeting with no summary in it.
#[tokio::test]
async fn a_reply_with_no_headings_fails_rather_than_writing_nothing() {
    let (addr, _seen) = stub::stub("Cuộc họp bàn về ngân sách và mọi người đã đồng ý.").await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let err = summarize::run(&paths, &client(addr), &id, None)
        .await
        .expect_err("a reply with no headings was accepted");
    assert!(err.to_string().contains("nothing that looks like"), "{err}");

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("mọi người đã đồng ý"),
        "the unparseable reply was written into the file anyway: {after}"
    );
}

/// Only the sections the template asked for.
///
/// A model that volunteers an extra heading must not get it into the document: the template is the
/// contract, and a vault whose sections vary per response cannot be parsed back.
#[tokio::test]
async fn a_section_nobody_asked_for_is_not_written() {
    let (addr, _seen) =
        stub::stub("## Tóm tắt\nChốt ngân sách.\n\n## Đánh giá nhân sự\nNgọc làm tốt.\n").await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let done = summarize::run(&paths, &client(addr), &id, None)
        .await
        .unwrap();
    assert_eq!(done.sections, ["Tóm tắt"]);

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("Đánh giá nhân sự"),
        "a heading the template never asked for was written: {after}"
    );
}

/// The template's own language wins over the user's setting.
///
/// Three places can decide this — the template, `settings.llm.language`, and "follow the
/// transcript" — and the order is silent: nothing reports which one applied. The built-in
/// `standard` template pins `vi`, so a user whose setting says English still gets a Vietnamese
/// summary from it, which is the intended behaviour and was asserted nowhere.
#[tokio::test]
async fn a_template_that_names_a_language_overrules_the_setting() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let mut settings = summo_core::Settings::default();
    settings.llm.language = "English".into();
    settings.save(&paths.settings()).unwrap();

    summarize::run(&paths, &client(addr), &id, None)
        .await
        .unwrap();

    // The instruction line, not the bare code. `contains("vi")` passes on any Vietnamese
    // transcript — "việc" contains it — so the first version of this assertion was true whichever
    // language had won.
    let seen = seen.lock().unwrap();
    assert!(
        seen[0].contains("Write the summary in vi."),
        "the template's language did not reach the model"
    );
    assert!(
        !seen[0].contains("Write the summary in English."),
        "the setting overruled the template's own language"
    );
}

/// And with no template language, the setting decides.
///
/// The other half of the precedence, and the half a user can actually change. Driven through a
/// template written for this test, because every built-in pins `vi` — so on the shipped set there
/// is no way to observe the setting working at all.
#[tokio::test]
async fn with_no_language_on_the_template_the_setting_decides() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    std::fs::create_dir_all(paths.templates()).unwrap();
    std::fs::write(
        paths.templates().join("plain.md"),
        "---\nname: Plain\nlanguage: \nmatch: []\n---\n## Tóm tắt\nHai ba câu.\n",
    )
    .unwrap();

    let mut settings = summo_core::Settings::default();
    settings.llm.language = "English".into();
    settings.save(&paths.settings()).unwrap();

    summarize::run(&paths, &client(addr), &id, Some("plain"))
        .await
        .unwrap();

    assert!(
        seen.lock().unwrap()[0].contains("Write the summary in English."),
        "the user's language setting never reached the model"
    );
}

/// Asking for a template that does not exist is refused before a request is spent.
#[tokio::test]
async fn an_unknown_template_is_refused_without_asking_the_model() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let err = summarize::run(&paths, &client(addr), &id, Some("khong-co-mau-nay"))
        .await
        .expect_err("an unknown template was accepted");
    assert!(err.to_string().contains("khong-co-mau-nay"), "{err}");
    assert!(seen.lock().unwrap().is_empty(), "a request was still sent");
}

/// Somebody else's outage is reported as theirs, and the meeting is left as it was.
#[tokio::test]
async fn a_provider_that_fails_leaves_the_meeting_untouched() {
    let (addr, _seen) = stub::failing(
        "429 Too Many Requests",
        r#"{"error":{"message":"Insufficient balance or no resource package."}}"#,
    )
    .await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    let err = summarize::run(&paths, &client(addr), &id, None)
        .await
        .expect_err("a 429 was treated as a summary");
    assert!(
        err.to_string().contains("Insufficient balance"),
        "the provider's own message was swallowed: {err}"
    );

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "a failed summary rewrote the meeting file"
    );
}

// ---------------------------------------------------------------------------
// The draft: the path the *button* takes.
//
// `POST /meetings/{id}/summarize` is the command-line route and the one `spawn` uses after a
// recording. The app never calls it — `lib/draft.ts` posts to `/draft/generate`, `/draft/refine`
// and `/draft/chat`. Those three are the whole AI surface a user can reach from a meeting, and
// `draft.rs`'s seventeen unit tests never build a client, so not one of them had been run against
// anything that answers.
//
// This is also what `e2e/draft.mjs` works around: it writes `<!-- summo:draft -->` into the file by
// hand because nothing in the suite can produce one.
// ---------------------------------------------------------------------------

use summo_engine::draft;

/// Generating a draft marks it as a draft.
///
/// The mark is the entire safety property of the feature: unconfirmed model text sits in the user's
/// own note, and the only thing separating it from something they wrote is that comment. A
/// generate that wrote clean headings would be indistinguishable from the user's own words.
#[tokio::test]
async fn a_generated_draft_lands_in_the_note_marked_as_unconfirmed() {
    let (addr, seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    let made = draft::generate(&paths, &client(addr), &id, None)
        .await
        .expect("six lines of transcript are draftable");
    assert_eq!(made.template, "standard");
    assert!(!made.sections.is_empty());

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("## Tóm tắt <!-- summo:draft -->"),
        "the draft is in the note without its mark: {after}"
    );
    assert!(after.contains("Chốt giữ ngân sách quý bốn"), "{after}");

    assert!(
        seen.lock().unwrap()[0].contains("hạ tầng đang vượt"),
        "the transcript did not reach the model"
    );
}

/// Confirming is what removes the mark, and it must not change the words.
///
/// Driven end to end rather than from a hand-seeded marker: the pair only means anything if the
/// thing being confirmed is the thing `generate` produced.
#[tokio::test]
async fn confirming_a_generated_draft_keeps_the_text_and_drops_the_mark() {
    let (addr, _seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());

    draft::generate(&paths, &client(addr), &id, None)
        .await
        .unwrap();
    let confirmed = draft::confirm(&paths, &id).unwrap();
    assert!(!confirmed.is_empty(), "confirming accepted no sections");

    let path = summarize::find_meeting_file(&paths.vault(), &id).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("summo:draft"),
        "the mark survived confirmation: {after}"
    );
    assert!(
        after.contains("Chốt giữ ngân sách quý bốn"),
        "confirming lost the text it was confirming: {after}"
    );
}

/// Refining rewrites the selected passage and nothing else.
#[tokio::test]
async fn refining_replaces_only_the_passage_that_was_selected() {
    let (addr, _seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());
    draft::generate(&paths, &client(addr), &id, None)
        .await
        .unwrap();

    let (rewrite, _) = stub::stub("Giữ nguyên mức quý ba.").await;
    let after = draft::refine(
        &paths,
        &client(rewrite),
        &id,
        "Tóm tắt",
        "Chốt giữ ngân sách quý bốn ở mức quý ba.",
        "Ngắn hơn",
    )
    .await
    .expect("a passage that is in the draft is refinable");

    let summary = after
        .sections
        .iter()
        .find(|s| s.heading == "Tóm tắt")
        .expect("the summary section survived");
    assert!(
        summary.body.contains("Giữ nguyên mức quý ba."),
        "{summary:?}"
    );

    // The other sections are untouched. A refine that regenerates the whole draft would silently
    // discard edits the user made to sections they were not looking at.
    assert!(
        after
            .sections
            .iter()
            .any(|s| s.heading == "Việc cần làm" && s.body.contains("@Ngọc")),
        "refining one section rewrote the others: {:?}",
        after.sections
    );
}

/// A selection that has moved on is refused rather than applied to whatever is there now.
#[tokio::test]
async fn a_stale_selection_is_refused_rather_than_rewriting_the_wrong_span() {
    let (addr, _seen) = stub::stub(ANSWER).await;
    let (_dir, paths, id) = vault_with(&a_real_meeting());
    draft::generate(&paths, &client(addr), &id, None)
        .await
        .unwrap();

    let (rewrite, seen) = stub::stub("bất kỳ").await;
    let err = draft::refine(
        &paths,
        &client(rewrite),
        &id,
        "Tóm tắt",
        "một câu không còn trong bản nháp",
        "Ngắn hơn",
    )
    .await
    .expect_err("a passage that is not there was refined anyway");
    assert!(err.to_string().contains("no longer in the draft"), "{err}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "a request was spent on a selection that could not be applied"
    );
}
