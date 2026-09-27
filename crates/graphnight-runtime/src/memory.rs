//! Agent memory v2, layered on the v1 storage row.
//!
//! The persisted record (`graphnight_storage::Memory`) has a fixed shape:
//! `id`, `learning`, `linked_entities`, `description`, timestamps and a free-form
//! `meta` map. Rather than migrate every existing row — which would break
//! already-indexed memories and every external reader of the table — v2 is a
//! *view* over that row: typed, ranked fields live in `meta`, and anything not
//! present falls back to a sensible default.
//!
//! The one behaviour change that matters is ranking. `MemoryFilter` in storage
//! only filters. `ranked` here reorders a result set by relevance, recency and
//! importance so an agent surfaces the strongest memories first instead of the
//! most recently written ones.

use crate::governance::{CallContext, ServiceError};
use graphnight_storage::Memory;
use serde_json::{json, Value};

/// Typed classification of a memory, controlling how it is surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemoryKind {
    /// A distilled lesson ("revenue is net of refunds").
    #[default]
    Learning,
    /// A stable fact about the data or the business.
    Fact,
    /// Something that happened, tied to a point in time.
    Episode,
    /// A choice that was made, ideally with its rationale.
    Decision,
    /// A failure worth not repeating.
    Error,
}

impl MemoryKind {
    pub fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("").to_ascii_lowercase().as_str() {
            "fact" => MemoryKind::Fact,
            "episode" => MemoryKind::Episode,
            "decision" => MemoryKind::Decision,
            "error" => MemoryKind::Error,
            _ => MemoryKind::Learning,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Learning => "learning",
            MemoryKind::Fact => "fact",
            MemoryKind::Episode => "episode",
            MemoryKind::Decision => "decision",
            MemoryKind::Error => "error",
        }
    }
}

/// Who a memory belongs to.
///
/// Scoping matters for multi-tenant deployments: without it, one tenant's stored
/// "decision" leaks into another tenant's agent context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    Global,
    Tenant(String),
    User(String),
}

impl MemoryScope {
    pub fn as_str(&self) -> &str {
        match self {
            MemoryScope::Global => "global",
            MemoryScope::Tenant(_) => "tenant",
            MemoryScope::User(_) => "user",
        }
    }

    /// Whether a caller may read a memory in this scope.
    pub fn visible_to(&self, ctx: &CallContext) -> bool {
        match self {
            MemoryScope::Global => true,
            MemoryScope::Tenant(t) => ctx.tenant_id.as_deref() == Some(t.as_str()),
            MemoryScope::User(u) => ctx.user_id.as_deref() == Some(u.as_str()),
        }
    }
}

/// A v2 view over a stored memory row.
#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub id: String,
    pub text: String,
    pub linked_entities: Vec<String>,
    pub kind: MemoryKind,
    /// 0.0–1.0.
    pub importance: f32,
    pub scope: MemoryScope,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

const META_KIND: &str = "gn_kind";
const META_IMPORTANCE: &str = "gn_importance";
const META_SCOPE: &str = "gn_scope";
const META_SCOPE_ID: &str = "gn_scope_id";

impl MemoryRecord {
    /// Read a v2 record out of a v1 row, defaulting anything absent.
    pub fn from_storage(m: &Memory) -> Self {
        let scope = match m.meta.get(META_SCOPE).and_then(|v| v.as_str()) {
            Some("tenant") => MemoryScope::Tenant(
                m.meta
                    .get(META_SCOPE_ID)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            ),
            Some("user") => MemoryScope::User(
                m.meta
                    .get(META_SCOPE_ID)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            ),
            // Absent scope means a pre-v2 row: global, as it always was.
            _ => MemoryScope::Global,
        };
        Self {
            id: m.id.clone(),
            text: m.learning.clone(),
            linked_entities: m.linked_entities.clone(),
            kind: MemoryKind::parse(m.meta.get(META_KIND).and_then(|v| v.as_str())),
            importance: m
                .meta
                .get(META_IMPORTANCE)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5) as f32,
            scope,
            description: m.description.clone(),
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }

    /// Project back down to the v1 row shape for persistence.
    pub fn to_storage(&self) -> Memory {
        let mut meta = std::collections::HashMap::new();
        meta.insert(META_KIND.to_string(), json!(self.kind.as_str()));
        meta.insert(META_IMPORTANCE.to_string(), json!(self.importance));
        meta.insert(META_SCOPE.to_string(), json!(self.scope.as_str()));
        let scope_id = match &self.scope {
            MemoryScope::Global => None,
            MemoryScope::Tenant(t) | MemoryScope::User(t) => Some(t.clone()),
        };
        if let Some(id) = scope_id {
            meta.insert(META_SCOPE_ID.to_string(), json!(id));
        }
        Memory {
            id: self.id.clone(),
            learning: self.text.clone(),
            linked_entities: self.linked_entities.clone(),
            description: self.description.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            meta,
        }
    }

    /// Build a record from an agent's `remember` tool input.
    pub fn from_agent_input(
        text: &str,
        linked_entities: &[String],
        kind: Option<&str>,
        importance: Option<f32>,
        description: Option<String>,
        ctx: &CallContext,
    ) -> Result<Self, ServiceError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ServiceError::Invalid(
                "memory text must not be empty".to_string(),
            ));
        }
        // Clamp rather than reject: an LLM confidently emitting 7.5 should not
        // fail the write, it should just be capped.
        let importance = importance.unwrap_or(0.5).clamp(0.0, 1.0);
        // Default to the caller's own user scope so a memory is never broadcast
        // to every tenant by omission.
        let scope = match (&ctx.tenant_id, &ctx.user_id) {
            (_, Some(u)) => MemoryScope::User(u.clone()),
            (Some(t), None) => MemoryScope::Tenant(t.clone()),
            (None, None) => MemoryScope::Global,
        };
        let now = chrono::Utc::now();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.to_string(),
            linked_entities: linked_entities
                .iter()
                .filter(|e| !e.trim().is_empty())
                .cloned()
                .collect(),
            kind: MemoryKind::parse(kind),
            importance,
            scope,
            description,
            created_at: now,
            updated_at: now,
        })
    }

    /// Ranking score: importance dominates, with a mild recency tiebreak.
    ///
    /// Deliberately simple and inspectable. A learned weighting would score
    /// marginally better on a benchmark but is far harder to explain when an
    /// agent behaves oddly, and this ranking is shown to users.
    pub fn rank_score(&self, query_terms: &[String]) -> f32 {
        let mut score = self.importance;
        if !query_terms.is_empty() {
            let haystack = format!("{} {}", self.text, self.linked_entities.join(" "));
            let haystack = haystack.to_ascii_lowercase();
            let hits = query_terms
                .iter()
                .filter(|t| haystack.contains(&t.to_ascii_lowercase()))
                .count();
            score += (hits as f32 / query_terms.len() as f32) * 2.0;
        }
        let age_days = (chrono::Utc::now() - self.created_at).num_days().max(0) as f32;
        score += 1.0 / (1.0 + age_days);
        score
    }

    /// Rank records, dropping any not visible to `ctx`.
    pub fn ranked(
        records: Vec<MemoryRecord>,
        ctx: &CallContext,
        query: Option<&str>,
    ) -> Vec<MemoryRecord> {
        let terms: Vec<String> = query
            .map(|q| {
                q.to_ascii_lowercase()
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut visible: Vec<MemoryRecord> = records
            .into_iter()
            .filter(|r| r.scope.visible_to(ctx))
            .collect();
        visible.sort_by(|a, b| {
            b.rank_score(&terms)
                .partial_cmp(&a.rank_score(&terms))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        visible
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "text": self.text,
            "kind": self.kind.as_str(),
            "linked_entities": self.linked_entities,
            "importance": self.importance,
            "scope": self.scope.as_str(),
            "description": self.description,
            "created_at": self.created_at.to_rfc3339(),
            "updated_at": self.updated_at.to_rfc3339(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(user: &str, tenant: &str) -> CallContext {
        CallContext {
            user_id: Some(user.to_string()),
            tenant_id: Some(tenant.to_string()),
            ..Default::default()
        }
    }

    fn row(learning: &str, meta: std::collections::HashMap<String, Value>) -> Memory {
        let now = chrono::Utc::now();
        Memory {
            id: "m1".to_string(),
            learning: learning.to_string(),
            linked_entities: vec![],
            description: None,
            created_at: now,
            updated_at: now,
            meta,
        }
    }

    #[test]
    fn v1_row_reads_as_global_learning() {
        let record =
            MemoryRecord::from_storage(&row("revenue is net of refunds", Default::default()));
        assert_eq!(record.kind, MemoryKind::Learning);
        assert_eq!(record.scope, MemoryScope::Global);
        assert!((record.importance - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn round_trips_through_v1_shape() {
        let record = MemoryRecord::from_agent_input(
            "fiscal year starts in April",
            &["orders".to_string()],
            Some("fact"),
            Some(0.9),
            None,
            &ctx("u1", "t1"),
        )
        .unwrap();
        let back = MemoryRecord::from_storage(&record.to_storage());
        assert_eq!(back.text, record.text);
        assert_eq!(back.kind, MemoryKind::Fact);
        assert_eq!(back.scope, MemoryScope::User("u1".to_string()));
        assert!((back.importance - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn clamps_out_of_range_importance() {
        let record =
            MemoryRecord::from_agent_input("x", &[], None, Some(7.5), None, &ctx("u1", "t1"))
                .unwrap();
        assert!((record.importance - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_empty_text() {
        assert!(
            MemoryRecord::from_agent_input("   ", &[], None, None, None, &ctx("u1", "t1")).is_err()
        );
    }

    #[test]
    fn user_scoped_memory_is_hidden_from_others() {
        let a =
            MemoryRecord::from_agent_input("a", &[], None, None, None, &ctx("u1", "t1")).unwrap();
        let b =
            MemoryRecord::from_agent_input("b", &[], None, None, None, &ctx("u2", "t1")).unwrap();
        let visible = MemoryRecord::ranked(vec![a.clone(), b.clone()], &ctx("u1", "t1"), None);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, a.id);
    }

    #[test]
    fn ranking_prefers_term_over_importance() {
        let now = chrono::Utc::now();
        let low = MemoryRecord {
            id: "low".into(),
            text: "unrelated note".into(),
            linked_entities: vec![],
            kind: MemoryKind::Learning,
            importance: 0.9,
            scope: MemoryScope::Global,
            description: None,
            created_at: now,
            updated_at: now,
        };
        let high = MemoryRecord {
            id: "high".into(),
            text: "refunds are subtracted from revenue".into(),
            linked_entities: vec![],
            kind: MemoryKind::Learning,
            importance: 0.2,
            scope: MemoryScope::Global,
            description: None,
            created_at: now,
            updated_at: now,
        };
        let ranked = MemoryRecord::ranked(vec![low, high], &ctx("u1", "t1"), Some("revenue"));
        assert_eq!(ranked[0].id, "high");
    }
}
