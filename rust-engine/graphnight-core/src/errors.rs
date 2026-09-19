use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Model already exists: {0}")]
    ModelExists(String),

    #[error("Datasource not found: {0}")]
    DatasourceNotFound(String),

    #[error("Datasource already exists: {0}")]
    DatasourceExists(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Invalid formula: {0}")]
    InvalidFormula(String),

    #[error("Join error: {0}")]
    JoinError(String),

    #[error("Security policy violation: {0}")]
    PolicyViolation(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Model already exists: {0}")]
    ModelExists(String),

    #[error("Datasource not found: {0}")]
    DatasourceNotFound(String),

    #[error("Datasource already exists: {0}")]
    DatasourceExists(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Storage backend error: {0}")]
    BackendError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("YAML serialization error: {0}")]
    YamlSerializationError(#[from] serde_yaml::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("SQL error: {0}")]
    SqlError(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Tantivy error: {0}")]
    TantivyError(String),

    #[error("Tantivy query parser error: {0}")]
    QueryParserError(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
