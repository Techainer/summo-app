//! *When* a live subtitle is answered, not just what it says.
//!
//! `live.rs` grouped eight lines into one request and sent every answer together at the end. The
//! reason written in the module was context — a model that sees the neighbouring sentences keeps
//! the pronouns — and for a chat-style model that reason is true. For the backend the release
//! actually ships it is not: `summo_mt::Seq2Seq::translate` takes one line and a target language
//! and has nowhere for a neighbour to arrive, and `Translator::run_local` walks the slice
//! sequentially. So the first subtitle of a batch waited for the eighth line to finish decoding —
//! about 1.9 seconds per target at 241 ms a line, roughly 3.9 with two targets, for an answer that
//! was ready almost immediately.
//!
//! That is a fact about *timing*, so a test of the returned value cannot see it. These drive the
//! two shapes against a real socket and assert on how many times each one answers, and on what has
//! been asked by the time the first answer arrives.

mod stub;

use std::net::SocketAddr;

use summo_core::event::Event;
use summo_engine::live::{Pending, grouped, line_by_line};
use summo_engine::translate::Translator;
use summo_llm::{Provider, prompt::Glossary};

fn provider(addr: SocketAddr) -> Provider {
    Provider::custom("stub", &format!("http://{addr}"), "test-model")
}

fn lines(texts: &[&str]) -> Vec<Pending> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| Pending {
            seq: i as u64 + 1,
            text: (*text).to_string(),
            // Spoken Vietnamese, so neither target below is skipped as already-in-that-language.
            language: Some("vi".into()),
        })
        .collect()
}

/// Drain a channel into the runs that were sent, each run being one `send`.
fn runs(mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<Event>>) -> Vec<Vec<Event>> {
    let mut out = Vec::new();
    while let Ok(events) = rx.try_recv() {
        out.push(events);
    }
    out
}

fn texts(events: &[Event]) -> Vec<(u64, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Translation { seq, lang, text } => Some((*seq, lang.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

/// The defect, stated as the behaviour that replaces it.
///
/// Three lines, two targets. Six translations, and the reader must not wait for the sixth to read
/// the first.
#[tokio::test]
async fn each_line_is_answered_as_it_finishes_rather_than_with_the_last() {
    let (addr, seen) = stub::stub("translated").await;
    let translator = Translator::mt(provider(addr), Some("vi".into())).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    line_by_line(
        &translator,
        &lines(&["một", "hai", "ba"]),
        &["en".to_string(), "ja".to_string()],
        &Glossary::default(),
        &tx,
    )
    .await;

    let runs = runs(rx);
    assert_eq!(
        runs.len(),
        3,
        "one answer per line, not one for the run: {runs:#?}"
    );

    // Each run carries that one line, in both languages — languages inside, lines outside. The
    // other nesting would finish every English subtitle before starting Japanese, which makes the
    // second reader wait out the whole batch for their first line.
    for (i, run) in runs.iter().enumerate() {
        let got = texts(run);
        assert_eq!(
            got.iter().map(|(seq, ..)| *seq).collect::<Vec<_>>(),
            vec![i as u64 + 1; 2],
            "run {i} should hold only line {}",
            i + 1
        );
        assert_eq!(
            got.iter()
                .map(|(_, lang, _)| lang.clone())
                .collect::<Vec<_>>(),
            vec!["en".to_string(), "ja".to_string()],
            "both readers get line {} together",
            i + 1
        );
    }

    // Six requests on the wire, one per line per target. Pinned because the whole argument for the
    // old shape was that grouping saved requests, and for this backend it never did: `run_local`
    // and the Mt style both issue one call per line either way.
    assert_eq!(seen.lock().unwrap().len(), 6);
}

/// The other shape, unchanged, so the fix cannot have quietly cost the chat backend its context.
///
/// A chat model is handed the whole run in one prompt, so there is nothing to answer early with —
/// and one send at the end is right.
#[tokio::test]
async fn a_backend_that_gains_from_grouping_still_answers_once() {
    let (addr, seen) = stub::stub("1. one\n2. two\n3. three").await;
    let translator = Translator::chat(provider(addr)).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    grouped(
        &translator,
        &lines(&["một", "hai", "ba"]),
        &["en".to_string()],
        &Glossary::default(),
        &tx,
    )
    .await;

    assert_eq!(runs(rx).len(), 1, "one send for the whole run");
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "and one request holding all three lines"
    );
}

/// A broken target is reported once for the run, not once per line.
///
/// Splitting the request per line multiplies everything about it, including the failure. A model
/// with no token for a language fails on every line it is given, and eight identical errors is the
/// interface shouting a fault it already stated — the same restraint `grouped` has by construction.
#[tokio::test]
async fn a_target_that_cannot_be_reached_is_reported_once_not_once_per_line() {
    let (addr, _seen) = stub::failing("500 Internal Server Error", "{}").await;
    let translator = Translator::mt(provider(addr), Some("vi".into())).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    line_by_line(
        &translator,
        &lines(&["một", "hai", "ba", "bốn"]),
        &["en".to_string()],
        &Glossary::default(),
        &tx,
    )
    .await;

    let errors = runs(rx)
        .iter()
        .flatten()
        .filter(|event| matches!(event, Event::Error { .. }))
        .count();
    assert_eq!(errors, 1, "one notice for the target, not four");
}

/// Every HTTP translator still groups.
///
/// The decision is a fact about the backend, and getting it backwards would either restore the
/// delay or strip the chat model of the context its prompt is built around. The in-process arm is
/// the one that returns false; it needs a model on disk, so what proves it is the end-to-end
/// latency measurement rather than this.
#[tokio::test]
async fn grouping_is_kept_for_the_backends_it_helps() {
    let (addr, _seen) = stub::stub("x").await;
    assert!(Translator::chat(provider(addr)).unwrap().batching_helps());
    assert!(
        Translator::mt(provider(addr), None)
            .unwrap()
            .batching_helps()
    );
}
