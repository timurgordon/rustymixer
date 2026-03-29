//! Music library management for RustyMixer.
//!
//! SQLite-backed track database with metadata, playlists,
//! crates, and search.

pub mod dao;
pub mod db;
pub mod error;
pub mod import;
pub mod manager;
pub mod metadata;
pub mod models;
pub mod scanner;
pub mod schema;

pub use db::Database;
pub use error::{LibraryError, Result};
pub use manager::LibraryManager;
pub use metadata::{CoverArt, MetadataReader, TrackMetadata};
pub use models::*;
pub use scanner::{spawn_scan, LibraryScanner, ScanHandle, ScanPhase, ScanProgress, ScanResult};

pub use dao::{CrateDao, CueDao, DirectoryDao, LocationDao, PlaylistDao, SettingsDao, TrackDao};
