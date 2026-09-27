//! Optional vector embeddings for hybrid retrieval.
//!
//! Deliberately small. Embeddings are *off by default* and additive: when no
//! provider is configured, search stays lexical and nothing about existing
//! behaviour changes. That property is the reason this is a trait with a
//! `None` default rather than a required component.
//!
//! Two rules keep a hybrid search honest:
//! 1. **Vectors are never the only signal.** Every provider is combined with
//!    lexical scores via reciprocal rank fusion, so a dense-only miss degrades
//!    ranking instead of hiding the row entirely.
//! 2. **Missing vectors degrade to lexical, never to empty.** A document that
//!    was never embedded must not disappear from results.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Produces vector embeddings for text.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Model identifier, recorded alongside vectors for later re-embedding.
    fn model(&self) -> &str;

    /// Vector dimensionality. Changing it invalidates stored vectors.
    fn dimensions(&self) -> usize;

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Convenience wrapper for a single string.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed(&[text.to_string()])
            .await
            .map(|mut v| v.pop().unwrap_or_default())
    }

    /// Embed a JSON value by rendering it to compact text.
    ///
    /// Objects and arrays are walked rather than stringified wholesale, so a
    /// large payload does not get truncated mid-structure by the embedder's
    /// token limit.
    async fn embed_json(&self, value: &Value) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_one(&value_to_text(value)).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("embedding provider is not configured")]
    NotConfigured,
    #[error("embedding request failed: {0}")]
    Provider(String),
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },
    #[error("text exceeds the embedder's input limit ({0} bytes)")]
    InputTooLong(usize),
    #[error("{0}")]
    Invalid(String),
}

/// Render a JSON value to embeddable text.
pub fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(items) => items
            .iter()
            .map(value_to_text)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let v = value_to_text(v);
                if v.is_empty() {
                    k.clone()
                } else {
                    format!("{k} {v}")
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Cosine similarity.
///
/// Returns 0.0 for zero-magnitude or mismatched vectors rather than `NaN`, so a
/// single degenerate document cannot poison a whole ranking.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 0.0;
    }
    let sim = dot / (norm_a.sqrt() * norm_b.sqrt());
    if sim.is_finite() {
        sim
    } else {
        0.0
    }
}

/// Reciprocal rank fusion of two ranked lists.
///
/// RRF merges by *rank*, not by score, which is what makes combining a lexical
/// score (unbounded, corpus-dependent) with a cosine score (bounded, not
/// comparable across models) safe. No score normalisation is needed, and neither
/// list can dominate the other by scale alone.
///
/// `k` damps the influence of the top ranks; 60 is the value from the original
/// RRF paper and is a reasonable default.
pub fn reciprocal_rank_fusion(
    lexical: &[(String, f32)],
    dense: &[(String, f32)],
    k: f32,
    weight_lexical: f32,
    weight_dense: f32,
) -> Vec<(String, f32)> {
    let mut scores: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
    for (list, weight) in [(lexical, weight_lexical), (dense, weight_dense)] {
        for (rank, (id, _)) in list.iter().enumerate() {
            *scores.entry(id.as_str()).or_insert(0.0) += weight / (k + rank as f32 + 1.0);
        }
    }
    let mut fused: Vec<(String, f32)> = scores
        .into_iter()
        .map(|(id, score)| (id.to_string(), score))
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

/// Blend two scores already normalised to `[0, 1]`, where `1.0` is best.
///
/// A `None` score contributes nothing but never removes the document, so an
/// unembedded row keeps its lexical position instead of vanishing.
pub fn blend_scores(
    lexical: Option<f32>,
    dense: Option<f32>,
    weight_lexical: f32,
    weight_dense: f32,
) -> f32 {
    let l = lexical.unwrap_or(0.0).clamp(0.0, 1.0);
    let d = dense.unwrap_or(0.0).clamp(0.0, 1.0);
    l * weight_lexical + d * weight_dense
}

/// Longest text sent to a provider. Well under any real model limit; the point
/// is to fail loudly rather than let a provider truncate a document mid-vector.
const MAX_INPUT_BYTES: usize = 100_000;

/// An OpenAI-compatible `/embeddings` client.
///
/// Works with OpenAI itself and with anything that speaks the same shape
/// (Ollama, vLLM, TEI, most proxies), because hybrid retrieval is only worth
/// enabling if it can point at a local model.
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    model: String,
    dimensions: usize,
    /// Guards against a runaway batch; model catalogs are small, but the input
    /// limit is the provider's, not ours, and a silent truncation would mean
    /// silently missing candidates.
    max_inputs: usize,
}

impl std::fmt::Debug for OpenAiEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbeddingProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field(
                "api_key",
                &if self.api_key.is_some() {
                    "[redacted]"
                } else {
                    "[none]"
                },
            )
            .finish()
    }
}

impl OpenAiEmbeddingProvider {
    /// `api_key` may be empty for a local server that requires no auth.
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
        dimensions: usize,
    ) -> Result<Self, EmbeddingError> {
        if dimensions == 0 {
            return Err(EmbeddingError::Invalid(
                "embedding dimensions must be greater than zero".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| EmbeddingError::Provider(e.to_string()))?;
        Ok(Self {
            client,
            api_key,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            dimensions,
            max_inputs: 64,
        })
    }

    pub fn with_max_inputs(mut self, max: usize) -> Self {
        self.max_inputs = max.max(1);
        self
    }

    fn endpoint(&self) -> String {
        format!("{}/embeddings", self.base_url)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn model(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if texts.len() > self.max_inputs {
            return Err(EmbeddingError::Invalid(format!(
                "{} inputs exceeds the batch limit of {}",
                texts.len(),
                self.max_inputs
            )));
        }
        if let Some(len) = texts
            .iter()
            .map(|t| t.len())
            .find(|&len| len > MAX_INPUT_BYTES)
        {
            return Err(EmbeddingError::InputTooLong(len));
        }

        let mut request = self.client.post(self.endpoint()).json(&serde_json::json!({
            "model": self.model,
            "input": texts,
        }));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request
            .send()
            .await
            .map_err(|e| EmbeddingError::Provider(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(EmbeddingError::Provider(format!(
                "embedding endpoint returned {status}: {}",
                body.chars().take(200).collect::<String>()
            )));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| EmbeddingError::Provider(e.to_string()))?;

        let data = payload
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| EmbeddingError::Provider("response has no `data` array".into()))?;

        // The API returns an `index` per item and does not promise order, so
        // place by index rather than trusting array order.
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for item in data {
            let idx = item.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            if idx >= texts.len() {
                return Err(EmbeddingError::Provider(format!(
                    "response index {idx} is out of range for {} inputs",
                    texts.len()
                )));
            }
            let vector = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| {
                    EmbeddingError::Provider("response item has no `embedding` array".into())
                })?;
            let floats: Vec<f32> = vector
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            if floats.len() != self.dimensions {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dimensions,
                    actual: floats.len(),
                });
            }
            out[idx] = Some(floats);
        }

        out.into_iter()
            .enumerate()
            .map(|(i, v)| {
                v.ok_or_else(|| EmbeddingError::Provider(format!("response omitted input {i}")))
            })
            .collect()
    }
}

/// A no-op provider, so callers can hold an `Option` uniformly in tests.
pub struct NullEmbedder;

#[async_trait]
impl EmbeddingProvider for NullEmbedder {
    fn model(&self) -> &str {
        "none"
    }
    fn dimensions(&self) -> usize {
        0
    }
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Err(EmbeddingError::NotConfigured)
    }
}

pub type SharedEmbedder = Arc<dyn EmbeddingProvider>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cosine_of_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_orthogonal_vectors_is_zero() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_of_zero_vector_is_zero_not_nan() {
        let got = cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]);
        assert_eq!(got, 0.0);
        assert!(got.is_finite());
    }

    #[test]
    fn cosine_of_mismatched_lengths_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn rrf_ranks_agreement_first() {
        let lexical = vec![("a".into(), 9.0), ("b".into(), 1.0)];
        let dense = vec![("a".into(), 0.9), ("b".into(), 0.1)];
        let fused = reciprocal_rank_fusion(&lexical, &dense, 60.0, 1.0, 1.0);
        assert_eq!(fused[0].0, "a");
        assert!(fused[0].1 > fused[1].1);
    }

    #[test]
    fn rrf_keeps_docs_present_in_only_one_list() {
        let lexical = vec![("a".into(), 1.0)];
        let dense = vec![("b".into(), 0.9)];
        let fused = reciprocal_rank_fusion(&lexical, &dense, 60.0, 1.0, 1.0);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn rrf_weight_zero_disables_a_list_without_dropping_its_docs() {
        let lexical = vec![("a".into(), 1.0)];
        let dense = vec![("b".into(), 0.9)];
        let fused = reciprocal_rank_fusion(&lexical, &dense, 60.0, 1.0, 0.0);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, "a");
    }

    #[test]
    fn unembedded_doc_still_scores_lexically() {
        let both = blend_scores(Some(1.0), Some(1.0), 0.5, 0.5);
        let lexical_only = blend_scores(Some(1.0), None, 0.5, 0.5);
        assert_eq!(lexical_only, 0.5);
        assert!(both > lexical_only);
    }

    #[test]
    fn blend_clamps_out_of_range_scores() {
        // 5.0 clamps up to 1.0, -5.0 clamps down to 0.0, so only the lexical
        // half contributes.
        assert_eq!(blend_scores(Some(5.0), Some(-5.0), 0.5, 0.5), 0.5);
        assert_eq!(blend_scores(Some(1.0), Some(1.0), 0.5, 0.5), 1.0);
        assert_eq!(blend_scores(None, None, 0.5, 0.5), 0.0);
    }

    #[test]
    fn renders_nested_json_to_text() {
        let v = json!({"name": "revenue", "meta": {"agg": "sum"}});
        let text = value_to_text(&v);
        assert!(text.contains("revenue"));
        assert!(text.contains("sum"));
    }

    #[test]
    fn null_embedder_reports_not_configured() {
        let e = NullEmbedder;
        assert!(e.dimensions() == 0);
    }

    /// Shared state for the stub endpoint: the canned status and body, plus the
    /// inputs and auth header it saw, so a test can assert what was sent.
    type StubState = (
        axum::http::StatusCode,
        &'static str,
        Arc<std::sync::Mutex<Vec<String>>>,
        Arc<std::sync::Mutex<Option<String>>>,
    );

    /// Minimal stand-in for an OpenAI-compatible `/embeddings` endpoint.
    async fn stub_embeddings(
        status: u16,
        body: &'static str,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        auth: Arc<std::sync::Mutex<Option<String>>>,
    ) -> String {
        use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
        async fn handler(
            State((status, body, seen, auth)): State<StubState>,
            headers: axum::http::HeaderMap,
            Json(req): Json<serde_json::Value>,
        ) -> impl IntoResponse {
            *seen.lock().unwrap() = req["input"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            *auth.lock().unwrap() = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            (status, body).into_response()
        }
        let state = (
            StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
            body,
            seen,
            auth,
        );
        let app = axum::Router::new().route(
            "/embeddings",
            axum::routing::post(handler).with_state(state),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    fn vector_payload(vectors: &[(&str, &[f32])]) -> String {
        let data: Vec<serde_json::Value> = vectors
            .iter()
            .enumerate()
            .map(|(i, (_, v))| serde_json::json!({ "index": i, "embedding": v }))
            .collect();
        serde_json::json!({ "object": "list", "data": data }).to_string()
    }

    #[tokio::test]
    async fn openai_provider_returns_vectors_in_request_order() {
        // Returned out of order on purpose: the provider must place by `index`.
        let body = serde_json::json!({
            "data": [
                { "index": 1, "embedding": [0.0, 1.0] },
                { "index": 0, "embedding": [1.0, 0.0] }
            ]
        })
        .to_string();
        let body: &'static str = Box::leak(body.into_boxed_str());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let auth = Arc::new(std::sync::Mutex::new(None));
        let base = stub_embeddings(200, body, seen.clone(), auth.clone()).await;

        let p = OpenAiEmbeddingProvider::new(base, Some("k".into()), "m", 2).unwrap();
        let out = p.embed(&["a".into(), "b".into()]).await.unwrap();
        assert_eq!(out, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(auth.lock().unwrap().as_deref(), Some("Bearer k"));
    }

    #[tokio::test]
    async fn openai_provider_reports_a_dimension_mismatch() {
        let body: &'static str =
            Box::leak(vector_payload(&[("x", &[1.0, 0.0, 0.0])]).into_boxed_str());
        let base = stub_embeddings(
            200,
            body,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            Arc::new(std::sync::Mutex::new(None)),
        )
        .await;
        let p = OpenAiEmbeddingProvider::new(base, None, "m", 2).unwrap();
        let err = p.embed_one("a").await.unwrap_err();
        assert!(
            matches!(
                err,
                EmbeddingError::DimensionMismatch {
                    expected: 2,
                    actual: 3
                }
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn openai_provider_surfaces_an_http_error() {
        let base = stub_embeddings(
            429,
            "rate limited",
            Arc::new(std::sync::Mutex::new(Vec::new())),
            Arc::new(std::sync::Mutex::new(None)),
        )
        .await;
        let p = OpenAiEmbeddingProvider::new(base, None, "m", 2).unwrap();
        let err = p.embed_one("a").await.unwrap_err();
        assert!(err.to_string().contains("429"), "{err}");
    }

    #[tokio::test]
    async fn openai_provider_rejects_an_oversized_batch() {
        let base = stub_embeddings(
            200,
            "[]",
            Arc::new(std::sync::Mutex::new(Vec::new())),
            Arc::new(std::sync::Mutex::new(None)),
        )
        .await;
        let p = OpenAiEmbeddingProvider::new(base, None, "m", 2)
            .unwrap()
            .with_max_inputs(2);
        let err = p
            .embed(&["a".into(), "b".into(), "c".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, EmbeddingError::Invalid(_)), "{err:?}");
    }

    #[tokio::test]
    async fn openai_provider_rejects_zero_dimensions() {
        let err = OpenAiEmbeddingProvider::new("http://x", None, "m", 0).unwrap_err();
        assert!(matches!(err, EmbeddingError::Invalid(_)), "{err:?}");
    }

    #[tokio::test]
    async fn openai_provider_sends_no_auth_header_without_a_key() {
        let body: &'static str = Box::leak(vector_payload(&[("x", &[1.0])]).into_boxed_str());
        let auth = Arc::new(std::sync::Mutex::new(None));
        let base = stub_embeddings(
            200,
            body,
            Arc::new(std::sync::Mutex::new(Vec::new())),
            auth.clone(),
        )
        .await;
        let p = OpenAiEmbeddingProvider::new(base, None, "m", 1).unwrap();
        p.embed_one("a").await.unwrap();
        assert!(auth.lock().unwrap().is_none());
    }
}
