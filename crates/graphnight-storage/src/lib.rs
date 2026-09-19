pub mod backend;
pub mod sqlite;
pub mod tantivy;
pub mod yaml;

// Re-export
pub use backend::{Memory, MemoryFilter, SearchResult, StorageBackend};
pub use sqlite::SqliteStorage;
pub use tantivy::TantivyStorage;
pub use yaml::YamlStorage;
