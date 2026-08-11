//! The local daemon.
//!
//! Everything that touches audio or a model runs here, in its own process, and the app talks to it
//! over a loopback socket. That split buys three things:
//!
//! * **Crash isolation.** A bad allocation inside an ONNX kernel takes down the daemon, not the
//!   meeting. The app reconnects and keeps the transcript it already has.
//! * **One implementation.** The desktop app, the CLI and — later — mobile all drive the same
//!   engine, so there is no second copy of the pipeline to keep in step.
//! * **A boundary worth defending.** Recognition state stays behind an authenticated socket rather
//!   than inside a webview.
//!
//! The security of that socket is not incidental; see [`auth`] for why a loopback port needs a
//! token and an origin check.

pub mod archive;
pub mod ask;
pub mod audio_stream;
pub mod auth;
pub mod board;
pub mod collaborate;
pub mod draft;
pub mod imports;
pub mod install;
pub mod locales;
pub mod nudge;
pub mod onboarding;
pub mod people;
pub mod protocol;
pub mod recorder;

#[cfg(feature = "models")]
pub mod offline;
#[cfg(feature = "models")]
pub mod runner;
pub mod server;
pub mod state;
pub mod summarize;
pub mod translate;
pub mod voicebook;

pub use auth::SessionToken;
pub use protocol::{Command, SessionSpec, decode_frame, encode_frame};
pub use recorder::Recorder;
pub use server::{Server, ServerConfig};
pub use state::{EngineState, SessionStatus};
