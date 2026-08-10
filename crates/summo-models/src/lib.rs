//! Model distribution for Summo.
//!
//! Every model — ASR, VAD, denoiser, speaker embedder — is described by a standalone JSON manifest
//! and installed into a content-addressed blob store, the same shape Ollama uses. There are no
//! tiers and no built-in model list: the app ships knowing only how to *resolve* a manifest, so a
//! model can be added by publishing a file, and a user can point at their own registry.
//!
//! ```text
//! resolve(id) → manifest → download blobs (resumable, sha256-verified) → install → load
//! ```

pub mod credentials;
pub mod download;
pub mod hw;
pub mod manifest;
pub mod recommend;
pub mod registry;
pub mod store;

pub use credentials::Credentials;
pub use download::{DownloadProgress, Downloader};
pub use hw::{Accel, CpuFeatures, HwProfile};
pub use manifest::{FileEntry, Manifest, Mode, Profile, Task};
pub use recommend::{Recommendation, Scored, recommend};
pub use registry::{Registry, RegistrySource};
pub use store::ModelStore;
