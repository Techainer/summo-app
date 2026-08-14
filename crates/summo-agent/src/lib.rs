//! Summo's agent: the verbs, and the loop that runs them.
//!
//! The loop itself is **not** ours. `aion-agent` (Apache-2.0) provides the streaming agent engine,
//! tool-calling protocol, retries, context compaction, session persistence and the MCP client, and
//! this crate supplies what is specific to Summo: a handful of tools about meetings, and the
//! bookkeeping that turns an `@agent` checkbox into a run whose progress is visible in the vault.
//!
//! Writing another agent loop would have been the expensive kind of mistake — months of work to
//! arrive at a worse version of something already maintained, tested and MCP-compatible.
//!
//! ```text
//!   - [ ] @agent …   ──►  plan  ──►  step, step, step  ──►  - [x] @agent …
//!                          │            │
//!                          │            └─ each written before it runs, ticked after
//!                          └─ aion-agent drives this; the tools are ours
//! ```

pub mod delegate;
pub mod habits;
pub mod memory;
pub mod roster;
pub mod run;
pub mod steps;
pub mod tools;

pub use roster::{AgentDef, Head, Roster};
pub use tools::all as summo_tools;
