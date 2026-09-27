//! RDF/SPARQL knowledge-graph storage for GraphNight (Oxigraph-backed).
//!
//! Workspace scaffold. The Oxigraph-backed `StorageBackend` implementation is
//! in progress; this stub keeps the workspace buildable while it lands.

/// Namespace for RDF/SPARQL storage primitives.
pub mod sparql {
    /// Query log entry placeholder (SPARQL SELECT executed against the graph).
    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    pub struct SparqlQueryLog {
        pub statement: String,
    }
}
