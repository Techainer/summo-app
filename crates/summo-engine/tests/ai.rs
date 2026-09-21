//! The rest of the AI surface against a socket: questions, the draft conversation, and dreaming.
//!
//! `tests/summarize.rs` covers the summary and the draft's generate and refine. This is what was
//! left: every other place in the engine that hands a prompt to a language model.
//!
//! The full list of `client.complete` call sites outside `summo-llm` is seven — `summarize::run`,
//! `draft::generate`, `draft::refine`, `draft::chat`, `ask::ask`, `dream::dream_one`, and the two
//! in `translate`. Translation had its own suite. The summary and two thirds of the draft now do.
//! These are the remaining three, and none of them had ever been run against something that
//! answers: `ask.rs` has five unit tests and `dream.rs` has its own, and not one of them builds a
//! client.
//!
//! What makes these worth driving rather than unit-testing is that the interesting behaviour is
//! *around* the request — which excerpts were selected and cited, whether a request is spent at
//! all, and, in `dream`'s case, a safety rule that throws away the model's answer and keeps a
//! month of memory instead.

mod stub;

use summo_core::{
    MeetingId,
    segment::{Lane, Segment},
};
use summo_engine::{ask, draft, dream};
use summo_llm::{LlmClient, Provider};
use summo_vault::{MeetingDoc, meeting::Frontmatter};

fn client(addr: std::net::SocketAddr) -> LlmClient {
    LlmClient::new(Provider::custom(
        "stub",
        &format!("http://{addr}"),
        "test-model",
    ))
    .unwrap()
}

/// A vault with one meeting in it, under `meetings/`.
fn vault(
    dir: &tempfile::TempDir,
    title: &str,
    lines: &[&str],
) -> (summo_core::paths::Paths, MeetingId) {
    let paths = summo_core::paths::Paths::at(dir.path());
    let id = MeetingId::new();
    let mut doc = MeetingDoc::new(Frontmatter::new(id.clone(), "2026-09-14"), title);
    for (i, text) in lines.iter().enumerate() {
        let t = i as f64 * 60.0 + 63.0;
        doc.transcript
            .push(Segment::new(i as u64, Lane::System, *text, t, t + 20.0));
    }
    let meetings = paths.vault().join("meetings");
    std::fs::create_dir_all(&meetings).unwrap();
    std::fs::write(
        meetings.join("2026-09-14-ngan-sach.md"),
        doc.to_markdown().unwrap(),
    )
    .unwrap();
    (paths, id)
}

// ---------------------------------------------------------------------------
// Asking the vault
// ---------------------------------------------------------------------------

/// The excerpts the answer is based on reach the model, and come back as checkable citations.
#[tokio::test]
async fn a_question_sends_the_matching_excerpts_and_cites_where_they_came_from() {
    let (addr, seen) = stub::stub("Ngân sách giữ ở mức quý ba. [t=01:03]").await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, id) = vault(
        &dir,
        "Họp ngân sách quý bốn",
        &[
            "Mình giữ ngân sách quý bốn ở mức quý ba, khoảng hai tỷ tư.",
            "Phần hạ tầng vượt ba trăm triệu so với dự toán.",
        ],
    );

    let answer = ask::ask(&paths, &client(addr), "ngân sách quý bốn")
        .await
        .expect("a vault with a matching meeting answers");

    assert!(!answer.text.is_empty());
    assert_eq!(answer.sources.len(), 1, "{:?}", answer.sources);
    assert_eq!(answer.sources[0].meeting, id.to_string());
    // `kind` is what the interface routes on. It was absent once, so every citation — including
    // the ones that were notes — opened `/meetings/<id>`, and a note cited as a source opened a
    // page that does not exist.
    assert_eq!(answer.sources[0].kind, "meeting");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].contains("hai tỷ tư"),
        "the excerpt never reached the model"
    );
    // The timestamp travels with the text, because that is what makes a citation checkable.
    assert!(
        seen[0].contains("t=01:03"),
        "the excerpt was sent without its timestamp: {}",
        &seen[0][..seen[0].len().min(600)]
    );
}

/// A vault with nothing on the subject says so, and does not pay for a model to say it.
#[tokio::test]
async fn a_question_the_vault_cannot_answer_costs_no_request() {
    let (addr, seen) = stub::stub("bất kỳ").await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, _) = vault(&dir, "Họp ngân sách", &["Mình chốt ngân sách quý bốn."]);

    let answer = ask::ask(&paths, &client(addr), "zzzzqqq không có trong kho")
        .await
        .unwrap();

    assert!(answer.sources.is_empty());
    assert!(answer.text.contains("Không tìm thấy"), "{}", answer.text);
    assert!(
        seen.lock().unwrap().is_empty(),
        "a request was spent on a search that found nothing"
    );
}

/// An empty question is refused before anything is searched or sent.
#[tokio::test]
async fn an_empty_question_is_refused_without_a_request() {
    let (addr, seen) = stub::stub("bất kỳ").await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, _) = vault(&dir, "Họp", &["Chốt ngân sách quý bốn."]);

    assert!(ask::ask(&paths, &client(addr), "   ").await.is_err());
    assert!(seen.lock().unwrap().is_empty());
}

/// The answer is written in the language the user chose, not the one the meeting was in.
#[tokio::test]
async fn the_answer_language_follows_the_setting() {
    let (addr, seen) = stub::stub("The budget stays at Q3 levels.").await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, _) = vault(
        &dir,
        "Họp ngân sách",
        &["Mình chốt ngân sách quý bốn ở mức cũ."],
    );

    let mut settings = summo_core::Settings::default();
    settings.llm.language = "English".into();
    settings.save(&paths.settings()).unwrap();

    ask::ask(&paths, &client(addr), "ngân sách").await.unwrap();

    assert!(
        seen.lock().unwrap()[0].contains("English"),
        "the language setting never reached the model"
    );
}

// ---------------------------------------------------------------------------
// The draft conversation
// ---------------------------------------------------------------------------

/// Enough transcript to get past the "too little here" gate.
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

const DRAFT: &str = "## Tóm tắt\nChốt giữ ngân sách quý bốn ở mức quý ba.\n\n\
                     ## Việc cần làm\n- [ ] @Ngọc — rà hợp đồng máy chủ — thứ sáu\n";

/// Asking the draft to change carries the draft *and* the transcript, and rewrites the draft.
///
/// Both halves matter. Without the transcript the model is editing prose it cannot check; without
/// the current draft it is writing a new one, and the user's edits are gone.
#[tokio::test]
async fn a_chat_turn_carries_the_draft_and_the_transcript_and_rewrites_the_draft() {
    let (first, _) = stub::stub(DRAFT).await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, id) = vault(&dir, "Họp ngân sách quý bốn", &a_real_meeting());
    draft::generate(&paths, &client(first), &id, None)
        .await
        .unwrap();

    let (second, seen) = stub::stub(
        "## Tóm tắt\nGiữ nguyên ngân sách.\n\n## Việc cần làm\n- [ ] @Ngọc — rà hợp đồng — thứ sáu\n",
    )
    .await;
    let after = draft::chat(&paths, &client(second), &id, "ngắn gọn hơn")
        .await
        .expect("a meeting with a draft can be talked to");

    let seen = seen.lock().unwrap();
    assert!(
        seen[0].contains("Chốt giữ ngân sách quý bốn ở mức quý ba."),
        "the current draft did not reach the model"
    );
    assert!(
        seen[0].contains("hợp đồng máy chủ"),
        "the transcript did not reach the model"
    );
    assert!(seen[0].contains("ngắn gọn hơn"), "the instruction was lost");

    assert!(
        after
            .sections
            .iter()
            .any(|s| s.body.contains("Giữ nguyên ngân sách.")),
        "the reply did not become the draft: {:?}",
        after.sections
    );
    // The turn is remembered, or the next message has no conversation behind it.
    assert!(!after.turns.is_empty(), "the chat turn was not recorded");
}

/// Talking to a meeting that has no draft is refused rather than silently starting one.
#[tokio::test]
async fn chatting_without_a_draft_is_refused_without_a_request() {
    let (addr, seen) = stub::stub(DRAFT).await;
    let dir = tempfile::tempdir().unwrap();
    let (paths, id) = vault(&dir, "Họp", &a_real_meeting());

    assert!(
        draft::chat(&paths, &client(addr), &id, "ngắn hơn")
            .await
            .is_err()
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "a request was spent before checking there was a draft"
    );
}

// ---------------------------------------------------------------------------
// Dreaming: consolidating an agent's memory
// ---------------------------------------------------------------------------

/// Write `count` facts into an agent's memory and return the roster slug holding them.
fn agent_with_memory(paths: &summo_core::paths::Paths, count: usize) -> String {
    let roster = summo_agent::roster::Roster::load_or_seed(&paths.agents()).unwrap();
    let agent = roster
        .all()
        .next()
        .expect("the seed roster has an agent")
        .clone();
    let facts: Vec<String> = (0..count)
        .map(|i| format!("Ngọc thích họp buổi sáng, lần thứ {i}."))
        .collect();
    summo_agent::memory::replace(&agent.memory_path(), &facts, "2026-09-14").unwrap();
    agent.slug.clone()
}

/// A night that consolidates keeps the shorter memory, and keeps a copy of the old one.
#[tokio::test]
async fn a_night_that_merges_facts_replaces_the_memory_and_archives_what_it_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let slug = agent_with_memory(&paths, 8);

    // Six of eight: a real consolidation, comfortably above the half it is allowed to forget.
    let merged = (0..6)
        .map(|i| format!("Ngọc thích họp buổi sáng ({i})."))
        .collect::<Vec<_>>()
        .join("\n");
    let reply: &'static str = Box::leak(merged.into_boxed_str());
    let (addr, seen) = stub::stub(reply).await;

    let done = dream::run(&paths, &client(addr), Some(&slug), "2026-09-15")
        .await
        .unwrap();

    assert_eq!(done.len(), 1);
    assert_eq!(done[0].before, 8);
    assert_eq!(
        done[0].after, 6,
        "the merged memory was not kept: {:?}",
        done[0]
    );
    assert!(done[0].refused.is_none(), "{:?}", done[0].refused);
    assert!(
        seen.lock().unwrap()[0].contains("Ngọc thích họp buổi sáng"),
        "the memory never reached the model"
    );
}

/// A night that would forget most of the memory is thrown away instead.
///
/// The rule this exists for: merging three ways of saying one thing is the job, and coming back
/// with two lines out of forty is a model that ignored the instruction. Applying that would cost a
/// month of what an agent knows in order to save one prompt — so the answer is discarded and the
/// old memory kept, which is a decision no unit test could observe because it depends on what came
/// back over the wire.
#[tokio::test]
async fn a_night_that_would_forget_most_of_the_memory_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let slug = agent_with_memory(&paths, 20);

    let roster = summo_agent::roster::Roster::load_or_seed(&paths.agents()).unwrap();
    let agent = roster.get(&slug).unwrap().clone();
    let before = std::fs::read_to_string(agent.memory_path()).unwrap();

    // Two lines back from twenty.
    let (addr, _seen) = stub::stub("Ngọc thích họp sáng.\nBình hay quên hạn.").await;
    let done = dream::run(&paths, &client(addr), Some(&slug), "2026-09-15")
        .await
        .unwrap();

    assert!(
        done[0].refused.is_some(),
        "a night that forgot 18 of 20 facts was applied: {:?}",
        done[0]
    );
    assert_eq!(done[0].after, 20, "the memory shrank anyway");
    assert_eq!(
        std::fs::read_to_string(agent.memory_path()).unwrap(),
        before,
        "a refused night rewrote the memory file"
    );
}

/// An agent with almost nothing in memory is not worth a request.
#[tokio::test]
async fn an_agent_with_nothing_to_think_about_costs_no_request() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let slug = agent_with_memory(&paths, 2);

    let (addr, seen) = stub::stub("bất kỳ").await;
    let done = dream::run(&paths, &client(addr), Some(&slug), "2026-09-15")
        .await
        .unwrap();

    assert!(done[0].refused.is_some());
    assert!(
        seen.lock().unwrap().is_empty(),
        "a request was spent on an agent with two facts"
    );
}

/// Naming an agent that is not in the roster fails before a request.
#[tokio::test]
async fn an_unknown_agent_is_refused_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let (addr, seen) = stub::stub("bất kỳ").await;

    assert!(
        dream::run(
            &paths,
            &client(addr),
            Some("khong-co-agent-nay"),
            "2026-09-15"
        )
        .await
        .is_err()
    );
    assert!(seen.lock().unwrap().is_empty());
}
