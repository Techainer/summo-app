//! Translation against a real socket, because the part that matters is what goes over it.
//!
//! The unit tests in `translate.rs` cover alignment — which translated line lands on which
//! utterance — with no model involved. What they cannot see is the request itself, and the request
//! is where this feature goes wrong: a dedicated translation model handed the numbered-batch prompt
//! answers with fluent nonsense rather than an error, so nothing downstream notices. Measured
//! against MiLMMT-46-1B, a three-line batch came back as a fourth line invented in the *source*
//! language.
//!
//! So these tests assert on the bytes: how many requests, and what was in them.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use summo_core::{
    MeetingId,
    segment::{Lane, Segment},
    settings::{Settings, Translator as TranslatorSettings},
};
use summo_engine::translate::{Style, Translator, translate};
use summo_llm::{Provider, prompt::Glossary};
use summo_vault::{MeetingDoc, meeting::Frontmatter};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Every request body the stub saw, in arrival order.
type Seen = Arc<Mutex<Vec<String>>>;

/// A model that answers each request with `reply`, recording what it was asked.
///
/// Deliberately not a mock of `LlmClient`: the thing under test is the HTTP conversation, and a
/// mock at the client boundary would have passed while the wire format was wrong.
async fn stub(reply: &'static str) -> (SocketAddr, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0_u8; 8192];
                // Read until the body is complete: `Content-Length` is the only framing here, and
                // stopping at the first read would truncate a batch prompt.
                let want = loop {
                    let Ok(n) = socket.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length: ")
                                    .or(l.strip_prefix("Content-Length: "))
                            })
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if body.len() >= len {
                            break body.to_string();
                        }
                    }
                };
                recorded.lock().unwrap().push(want);

                let body = format!(
                    r#"{{"choices":[{{"message":{{"role":"assistant","content":{}}}}}]}}"#,
                    serde_json::to_string(reply).unwrap()
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (addr, seen)
}

fn provider(addr: SocketAddr) -> Provider {
    Provider::custom("stub", &format!("http://{addr}"), "test-model")
}

fn doc_with(texts: &[&str]) -> MeetingDoc {
    let mut doc = MeetingDoc::new(Frontmatter::new(MeetingId::new(), "2026-08-12"), "Họp");
    for (i, text) in texts.iter().enumerate() {
        let seq = i as u64 + 1;
        let t = i as f64 * 10.0;
        doc.transcript
            .push(Segment::new(seq, Lane::System, *text, t, t + 5.0));
    }
    doc
}

/// The whole reason [`Style::Mt`] exists. One request per line, in the template the model was
/// trained to continue — no system turn, no numbering, no instructions.
#[tokio::test]
async fn a_translation_model_is_asked_one_line_at_a_time_in_its_own_template() {
    let (addr, seen) = stub("Settle the API spec.").await;
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let doc = doc_with(&["Chốt spec API.", "Gửi cho khách.", "Xong thứ Sáu."]);

    let translator = Translator::mt(provider(addr), Some("vi".into())).unwrap();
    let outcome = translate(
        &paths,
        &translator,
        &MeetingId::new(),
        &doc,
        "en",
        &Glossary::default(),
        false,
    )
    .await
    .unwrap();

    assert_eq!(outcome.translated, 3);
    assert_eq!(
        outcome.requests, 3,
        "one request per line, not one per batch"
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    for body in seen.iter() {
        let sent: serde_json::Value = serde_json::from_str(body).unwrap();
        let messages = sent["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "a system turn moves it off the template");
        assert_eq!(messages[0]["role"], "user");
        let content = messages[0]["content"].as_str().unwrap();
        assert!(
            content.starts_with("Translate this from Vietnamese to English:\nVietnamese: "),
            "got: {content}"
        );
        assert!(content.ends_with("\nEnglish:"), "got: {content}");
    }
}

/// The other half of the same decision: a general model still gets numbered batches, because it can
/// follow the instruction and one request for twenty-five lines is far cheaper than twenty-five.
#[tokio::test]
async fn a_general_model_still_gets_one_numbered_batch() {
    let (addr, seen) = stub("1. one\n2. two\n3. three").await;
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let doc = doc_with(&["một", "hai", "ba"]);

    let outcome = translate(
        &paths,
        &Translator::chat(provider(addr)).unwrap(),
        &MeetingId::new(),
        &doc,
        "en",
        &Glossary::default(),
        false,
    )
    .await
    .unwrap();

    assert_eq!(outcome.translated, 3);
    assert_eq!(outcome.requests, 1);

    let seen = seen.lock().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&seen[0]).unwrap();
    let messages = sent["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[1]["content"].as_str().unwrap().contains("1. một"));
}

/// A translation model that returns nothing for a line must cost that line, not shift every later
/// one onto the wrong utterance.
#[tokio::test]
async fn a_line_the_model_did_not_answer_keeps_its_original_text() {
    let (addr, _seen) = stub("   ").await;
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let id = MeetingId::new();
    let doc = doc_with(&["một", "hai"]);

    let outcome = translate(
        &paths,
        &Translator::mt(provider(addr), None).unwrap(),
        &id,
        &doc,
        "en",
        &Glossary::default(),
        false,
    )
    .await
    .unwrap();

    assert_eq!(outcome.translated, 0);
    assert_eq!(outcome.missing, 2);
    assert!(!outcome.complete());

    let saved = summo_vault::translation::load(&paths, &id, "en")
        .unwrap()
        .expect("a file, so the UI can tell `translated, empty` from `never translated`");
    assert!(saved.get(1).is_none(), "an absent line must stay absent");
}

/// Everything the model returns is echoed straight into a Markdown file in the user's vault, so a
/// model that keeps talking past the sentence must not get its continuation written there.
#[tokio::test]
async fn a_model_that_kept_talking_contributes_only_its_first_line() {
    let (addr, _seen) = stub("Settle the spec.\nVietnamese: Chốt spec.\nEnglish: Settle it.").await;
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let id = MeetingId::new();

    translate(
        &paths,
        &Translator::mt(provider(addr), Some("vi".into())).unwrap(),
        &id,
        &doc_with(&["Chốt spec."]),
        "en",
        &Glossary::default(),
        false,
    )
    .await
    .unwrap();

    let saved = summo_vault::translation::load(&paths, &id, "en")
        .unwrap()
        .unwrap();
    assert_eq!(saved.get(1), Some("Settle the spec."));
}

/// End to end, the defence that matters: a wrong-language reply must never reach the file.
#[tokio::test]
async fn a_reply_in_another_language_is_dropped_rather_than_written() {
    // The exact string MiLMMT-46-1B returned when asked for Japanese.
    let (addr, _seen) = stub("โอเคค่ะ ฉันจะเลื่อนกำหนดไปเป็นวันศุกร์สัปดาห์หน้า").await;
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let id = MeetingId::new();

    let outcome = translate(
        &paths,
        &Translator::mt(provider(addr), Some("vi".into())).unwrap(),
        &id,
        &doc_with(&["Ok vậy mình dời mốc ra thứ Sáu."]),
        "ja",
        &Glossary::default(),
        false,
    )
    .await
    .unwrap();

    assert_eq!(outcome.translated, 0);
    assert_eq!(
        outcome.missing, 1,
        "counted as missing, so the UI can say so"
    );

    let saved = summo_vault::translation::load(&paths, &id, "ja")
        .unwrap()
        .unwrap();
    assert!(
        saved.get(1).is_none(),
        "Thai must not be written into a file labelled `ja` — the original is readable, this is not"
    );
}

/// Found by running it: translating a three-line Vietnamese meeting into Japanese at the shared
/// default temperature of 0.2, MiLMMT-46-1B returned the third line in **Thai** — fluent Thai, in a
/// file labelled `ja`. No parser can catch that, because the wrong language is not a malformed
/// response; the only defence is not to sample.
#[test]
fn a_translation_model_never_samples() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let mut settings = Settings::default();
    settings.llm.translator = Some(TranslatorSettings {
        provider: "llama-cpp".into(),
        model: None,
    });

    let translator = Translator::from_settings(&paths, &settings).unwrap();
    assert_eq!(translator.temperature(), 0.0);
    assert_eq!(
        Translator::mt(Provider::custom("x", "http://127.0.0.1:1", "m"), None)
            .unwrap()
            .temperature(),
        0.0,
        "the guarantee is the constructor's, not the caller's"
    );
}

/// Which model translation goes to is a setting, and getting it wrong is silent — so the mapping
/// from settings to style is worth pinning down on its own.
#[test]
fn settings_decide_the_style_and_the_endpoint_together() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());

    let mut settings = Settings::default();
    settings.llm.provider = "ollama".into();
    let general = Translator::from_settings(&paths, &settings).unwrap();
    assert_eq!(general.style(), Style::Chat);

    settings.llm.translator = Some(TranslatorSettings {
        provider: "llama-cpp".into(),
        model: Some("milmmt-46-1b".into()),
    });
    let dedicated = Translator::from_settings(&paths, &settings).unwrap();
    assert_eq!(dedicated.style(), Style::Mt);
    assert_eq!(dedicated.model(), "milmmt-46-1b");
}

/// `local` is not an endpoint and must never be resolved as one. If it were, it would fall through
/// to the "some other OpenAI-compatible server" branch and the daemon would spend every translation
/// connecting to `http://local`.
#[test]
fn local_is_not_treated_as_an_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let paths = summo_core::paths::Paths::at(dir.path());
    let mut settings = Settings::default();
    settings.llm.translator = Some(TranslatorSettings {
        provider: "local".into(),
        model: Some("milmmt-46-1b".into()),
    });

    // Nothing is installed in this temp home, so the local path must fail for that reason — not by
    // quietly building an HTTP client for a host called `local`.
    let err = Translator::from_settings(&paths, &settings)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        !err.contains("http"),
        "`local` was resolved as an endpoint: {err}"
    );
}

/// The default is the one that needs nothing installed. A user who turns translation on and is then
/// told to install a model server has not been given a feature that costs nothing.
#[test]
fn the_default_translator_runs_in_this_process() {
    assert!(TranslatorSettings::default().is_local());
}
