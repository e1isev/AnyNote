//! AnyNote core.
//!
//! * [`node`]: the block model (ProseMirror-compatible node tree).
//! * [`ydoc`]: one CRDT document per page, readable and writable from Rust.
//! * [`ink`]: vector ink strokes stored in the page CRDT.
//! * [`store`]: SQLite catalogue, CRDT update log, version history, full-text
//!   search, links, tasks and tags.
//! * [`assets`]: content-addressed file storage.
//! * [`markdown`] and [`folder`]: the open, git-friendly folder format.

pub mod assets;
pub mod derive;
pub mod folder;
pub mod ids;
pub mod ink;
pub mod markdown;
pub mod node;
pub mod paths;
pub mod store;
pub mod welcome;
pub mod ydoc;

pub use node::{Mark, Node};
pub use store::Store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid CRDT update: {0}")]
    InvalidUpdate(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
