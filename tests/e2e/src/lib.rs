//! GraphNight end-to-end integration tests (see `tests/`).
//!
//! - `pipeline_sqlite`: examples YAML → SQL dry-run → temp SQLite execute
//! - `pipeline_testcontainers`: Postgres (CI) + optional MySQL; skip with
//!   `GRAPHNIGHT_SKIP_TESTCONTAINERS=1` or when Docker is unavailable
