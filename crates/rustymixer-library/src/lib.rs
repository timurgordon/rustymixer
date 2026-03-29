//! Music library management for RustyMixer.
//!
//! SQLite-backed track database with metadata, playlists,
//! crates, and search.

pub mod dao;
pub mod db;
pub mod error;
pub mod metadata;
pub mod models;
pub mod schema;

pub use db::Database;
pub use error::{LibraryError, Result};
pub use metadata::{CoverArt, MetadataReader, TrackMetadata};
pub use models::*;

pub use dao::{CrateDao, CueDao, DirectoryDao, LocationDao, PlaylistDao, SettingsDao, TrackDao};
