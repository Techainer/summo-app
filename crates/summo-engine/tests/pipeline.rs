//! The daemon, end to end.
//!
//! Every component is tested in isolation elsewhere. This is the only test that would catch the
//! pipeline being wired together wrongly while each piece passes its own tests: a real recording
//! goes through a real socket into a real model, and transcript events have to come back.
//!
//! Skipped unless the fixtures are set, so the suite still runs on a machine with no models.

#![cfg(feature = "models")]

use futures::{SinkExt, StreamExt};
use summo_core::{Event, audio::FRAME_LEN, paths::Paths, segment::Lane};
use summo_engine::{EngineState, Server, ServerConfig, encode_frame};
use tokio_tungstenite::tungstenite::Message;

struct Fixtures {
    home: std::path::PathBuf,
    wav: std::path::PathBuf,
    model: String,
}

fn fixtures() -> Option<Fixtures> {
    Some(Fixtures {
        home: std::env::var_os("SUMMO_TEST_HOME")?.into(),
        wav: std::env::var_os("SUMMO_TEST_WAV")?.into(),
        model: std::env::var("SUMMO_TEST_MODEL_ID").ok()?,
    })
}

fn read_wav(path: &std::path::Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("cannot open the test recording");
    let scale = 1.0 / f32::from(i16::MAX);
    reader
        .samples::<i16>()
        .map(|s| f32::from(s.expect("bad sample")) * scale)
        .collect()
}

async fn connect(
    home: &std::path::Path,
) -> (
    Server,
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) {
    let engine = EngineState::new(Paths::at(home)).unwrap();
    let server = Server::start(
        engine,
        ServerConfig {
            port: 0,
            write_token_file: false,
        },
    )
    .await
    .unwrap();

    let url = format!(
        "ws://{}/ws?token={}",
        server.addr(),
        server.token().as_str()
    );
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("the daemon should accept an authenticated connection");
    (server, socket)
}

#[tokio::test]
async fn a_recording_pushed_through_the_socket_comes_back_as_transcript() {
    let Some(fx) = fixtures() else {
        eprintln!("skipping: set SUMMO_TEST_HOME, SUMMO_TEST_WAV and SUMMO_TEST_MODEL_ID");
        return;
    };
    let (server, mut socket) = connect(&fx.home).await;

    socket
        .send(Message::Text(
            serde_json::json!({"cmd": "session_start", "live_model": fx.model})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    // Acknowledged before any audio: a failure here means the models did not load.
    let ack = socket.next().await.unwrap().unwrap();
    let ack: Event = serde_json::from_str(ack.to_text().unwrap()).unwrap();
    assert!(
        matches!(ack, Event::Info { .. }),
        "session did not start: {ack:?}"
    );

    // Feed the recording exactly as live capture would: one 100 ms frame at a time.
    let pcm = read_wav(&fx.wav);
    let mut finals: Vec<summo_core::Segment> = Vec::new();
    let mut partials = 0;

    for frame in pcm.chunks(FRAME_LEN) {
        socket
            .send(Message::Binary(encode_frame(Lane::Mic, frame).into()))
            .await
            .unwrap();

        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(std::time::Duration::from_millis(2), socket.next()).await
        {
            let Ok(text) = message.to_text() else {
                continue;
            };
            match serde_json::from_str::<Event>(text) {
                Ok(Event::Final(segment)) => finals.push(segment),
                Ok(Event::Partial(_)) => partials += 1,
                _ => {}
            }
        }
    }

    socket
        .send(Message::Text(r#"{"cmd":"session_stop"}"#.into()))
        .await
        .unwrap();

    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(std::time::Duration::from_secs(30), socket.next()).await
    {
        let Ok(text) = message.to_text() else {
            continue;
        };
        match serde_json::from_str::<Event>(text) {
            Ok(Event::Final(segment)) => finals.push(segment),
            Ok(Event::Info { text }) if text.contains("stopped") => break,
            _ => {}
        }
    }

    assert!(
        !finals.is_empty(),
        "audio reached the daemon but no transcript came back"
    );
    assert!(
        finals.iter().any(|s| !s.text.trim().is_empty()),
        "every segment was empty: {finals:?}"
    );
    assert!(
        partials > 0,
        "no partial text was produced, so the pseudo-streaming loop is not running"
    );
    assert!(
        finals.windows(2).all(|w| w[0].t0 <= w[1].t0),
        "segments arrived out of order"
    );

    eprintln!(
        "transcribed {} segment(s), {partials} partial(s):",
        finals.len()
    );
    for s in &finals {
        eprintln!("  [{:>6.2}s] {}", s.t0, s.text);
    }

    server.shutdown();
}

/// The promise this guards: your data is a folder of files you own. A recording that only ever
/// existed in the app's memory does not keep it.
#[tokio::test]
async fn a_finished_recording_is_written_into_the_vault() {
    let Some(fx) = fixtures() else {
        eprintln!("skipping: set the pipeline fixtures");
        return;
    };
    let meetings = fx.home.join("vault/meetings");
    let before: Vec<_> = std::fs::read_dir(&meetings)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();

    let (server, mut socket) = connect(&fx.home).await;

    socket
        .send(Message::Text(
            serde_json::json!({"cmd": "session_start", "live_model": fx.model})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ack = socket.next().await.unwrap().unwrap();
    let ack: Event = serde_json::from_str(ack.to_text().unwrap()).unwrap();
    assert!(
        matches!(ack, Event::Info { .. }),
        "session did not start: {ack:?}"
    );

    let pcm = read_wav(&fx.wav);
    for frame in pcm.chunks(FRAME_LEN) {
        socket
            .send(Message::Binary(encode_frame(Lane::Mic, frame).into()))
            .await
            .unwrap();
        // Drain one pending event if there is one; this test cares about the file, not the stream.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(1), socket.next()).await;
    }

    socket
        .send(Message::Text(r#"{"cmd":"session_stop"}"#.into()))
        .await
        .unwrap();

    let mut saved_path = None;
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(std::time::Duration::from_secs(30), socket.next()).await
    {
        let Ok(text) = message.to_text() else {
            continue;
        };
        match serde_json::from_str::<Event>(text) {
            Ok(Event::Info { text }) if text.starts_with("saved to ") => {
                saved_path = Some(std::path::PathBuf::from(
                    text.trim_start_matches("saved to ").to_string(),
                ));
            }
            Ok(Event::Info { text }) if text.contains("stopped") => break,
            _ => {}
        }
    }

    let path = saved_path.expect("stopping a session should report where it was saved");
    assert!(
        path.exists(),
        "the daemon reported a path it did not write: {path:?}"
    );
    assert!(
        !before.contains(&path),
        "a new file should have been created"
    );

    let body = std::fs::read_to_string(&path).unwrap();
    assert!(
        body.starts_with("---"),
        "the saved file should lead with frontmatter"
    );
    assert!(
        body.contains("## Transcript"),
        "no transcript section: {body}"
    );
    assert!(
        body.lines().any(|l| l.starts_with("**[")),
        "the transcript section is empty:\n{body}"
    );
    assert!(
        body.contains(&fx.model),
        "the model used should be recorded"
    );

    server.shutdown();
}

#[tokio::test]
async fn audio_before_the_session_starts_is_reported_not_swallowed() {
    let Some(fx) = fixtures() else {
        eprintln!("skipping: set the pipeline fixtures");
        return;
    };
    let (server, mut socket) = connect(&fx.home).await;

    socket
        .send(Message::Binary(
            encode_frame(Lane::Mic, &[0.0; FRAME_LEN]).into(),
        ))
        .await
        .unwrap();

    let reply = socket.next().await.unwrap().unwrap();
    let event: Event = serde_json::from_str(reply.to_text().unwrap()).unwrap();

    // Discarding it silently looks exactly like a microphone that is not working.
    assert!(
        matches!(
            event,
            Event::Error {
                transient: true,
                ..
            }
        ),
        "expected a transient error, got {event:?}"
    );

    server.shutdown();
}

#[tokio::test]
async fn a_session_naming_a_model_that_is_not_installed_fails_and_stays_idle() {
    let Some(fx) = fixtures() else {
        eprintln!("skipping: set the pipeline fixtures");
        return;
    };
    let (server, mut socket) = connect(&fx.home).await;

    socket
        .send(Message::Text(
            r#"{"cmd":"session_start","live_model":"not-installed"}"#.into(),
        ))
        .await
        .unwrap();

    let reply = socket.next().await.unwrap().unwrap();
    let event: Event = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert!(matches!(event, Event::Error { .. }), "got {event:?}");

    // A failed load must not leave the daemon believing it is recording, or the next attempt with a
    // model that *is* installed would be refused as "already in progress".
    socket
        .send(Message::Text(
            serde_json::json!({"cmd": "session_start", "live_model": fx.model})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let reply = socket.next().await.unwrap().unwrap();
    let event: Event = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert!(
        matches!(event, Event::Info { .. }),
        "a second attempt should succeed after a failed load, got {event:?}"
    );

    server.shutdown();
}
