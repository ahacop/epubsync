pub mod config;
pub mod cover;
pub mod device;
pub mod kepub;
pub mod kobo;
pub mod library;
pub mod query;
pub mod sort_name;
pub mod stats;
pub mod sync;

/// The metadata record types, which the EPUB crate defines because the
/// file and the library both hold them.
pub use epubsync_epub::metadata;

/// The crate version, shown by the CLI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
