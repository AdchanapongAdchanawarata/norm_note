//! Core of norm_note: vault, oplog, CRDT reconciliation, index and history.
//!
//! # Invariants
//!
//! These are load-bearing. Breaking one breaks a guarantee we make to users,
//! so each is enforced by a test or by the build, not by discipline alone.
//!
//! **G1 — Zero Egress.** This crate opens no sockets and has no
//! network-capable dependency. Networking, when it exists, lives only in
//! `norm-net`. Enforced by `deny.toml` (`cargo deny check bans`).
//!
//! **G3 — It's just files.** The `.md` files on disk are the user's source of
//! truth. Everything under `.norm/` is derived and may be deleted at any time
//! without losing a note. Enforced by `tests/rebuild_from_scratch.rs`.
//!
//! **Determinism.** No sync or merge decision may depend on the wall clock.
//! Two devices with badly skewed clocks must converge identically. Timestamps
//! are recorded for humans to read, never for machines to resolve conflicts.

pub mod config;
pub mod doc;
pub mod oplog;
pub mod replica;
pub mod trash;
pub mod vault;
pub mod workspace;

/// Name of the derived-state directory inside a vault.
///
/// Everything under here is reconstructible from the `.md` files, with the
/// single exception of `history/`, which holds past versions that no longer
/// exist on disk. Deleting the whole directory is a supported operation.
pub const NORM_DIR: &str = ".norm";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed frontmatter in {path}: {reason}")]
    Frontmatter { path: String, reason: String },

    #[error("malformed oplog chunk {name}: {reason}")]
    Chunk { name: String, reason: String },

    #[error("decryption failed for chunk {name} (wrong key, or the chunk was tampered with)")]
    Decrypt { name: String },

    #[error("crdt: {0}")]
    Crdt(String),
}

impl From<automerge::AutomergeError> for Error {
    fn from(e: automerge::AutomergeError) -> Self {
        Error::Crdt(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
