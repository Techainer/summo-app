//! Summaries, translation and question answering.
//!
//! The only part of Summo that leaves the machine, and only ever with *text*. Audio and the
//! recognition that turns it into text stay local by design; what a user opts into here is sending
//! a transcript to a model they chose — which may itself be running on their own laptop.
//!
//! Two consequences shape the code. [`Provider::is_local`] exists so the UI can state plainly
//! whether the configured endpoint keeps data on the machine. And every prompt in [`prompt`] is a
//! pure function returning messages, so what gets sent can be inspected and tested without a
//! network.

pub mod prompt;
pub mod provider;

pub use prompt::{Glossary, SummaryStyle};
pub use provider::{LlmClient, Message, Provider, Role, Wire};
