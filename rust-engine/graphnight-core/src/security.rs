use crate::models::Filter;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Session policy for security and governance
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    /// Forced filters that are always applied
    pub forced_filters: Vec<Filter>,
    /// Allowed models (whitelist)
    pub allowed_models: Option<Vec<String>>,
    /// Denied models (blacklist)
    pub denied_models: Option<Vec<String>>,
    /// Column masks for PII
    pub column_masks: HashMap<String, MaskFn>,
    /// Row-level security filter
    pub row_filter: Option<Filter>,
    /// Maximum rows to return
    pub max_rows: Option<usize>,
    /// Allowed datasources
    pub allowed_datasources: Option<Vec<String>>,
    /// Query timeout in seconds
    pub query_timeout_secs: Option<u64>,
}

/// Mask function type
pub type MaskFn = fn(&str) -> String;

impl SessionPolicy {
    pub fn new() -> Self {
        Self {
            forced_filters: Vec::new(),
            allowed_models: None,
            denied_models: None,
            column_masks: HashMap::new(),
            row_filter: None,
            max_rows: Some(10000),
            allowed_datasources: None,
            query_timeout_secs: Some(300),
        }
    }

    pub fn with_forced_filter(mut self, filter: Filter) -> Self {
        self.forced_filters.push(filter);
        self
    }

    pub fn with_allowed_models(mut self, models: Vec<String>) -> Self {
        self.allowed_models = Some(models);
        self
    }

    pub fn with_denied_models(mut self, models: Vec<String>) -> Self {
        self.denied_models = Some(models);
        self
    }

    pub fn with_column_mask(mut self, column: String, mask_fn: MaskFn) -> Self {
        self.column_masks.insert(column, mask_fn);
        self
    }

    pub fn with_row_filter(mut self, filter: Filter) -> Self {
        self.row_filter = Some(filter);
        self
    }

    pub fn with_max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = Some(max_rows);
        self
    }
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in mask functions
pub mod masks {
    pub fn email_mask(value: &str) -> String {
        if let Some(at_pos) = value.find('@') {
            let local = &value[..at_pos];
            let domain = &value[at_pos..];
            if local.len() > 2 {
                format!("{}***{}", &local[..1], &local[local.len() - 1..]) + domain
            } else {
                format!("***{}", domain)
            }
        } else {
            "***".to_string()
        }
    }

    pub fn phone_mask(value: &str) -> String {
        let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 10 {
            format!("(***) ***-{:0>4}", &digits[digits.len() - 4..])
        } else {
            "***-***-****".to_string()
        }
    }

    pub fn ssn_mask(_value: &str) -> String {
        "***-**-****".to_string()
    }

    pub fn credit_card_mask(value: &str) -> String {
        let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 4 {
            format!("**** **** **** {:>4}", &digits[digits.len() - 4..])
        } else {
            "**** **** **** ****".to_string()
        }
    }

    pub fn hash_mask(value: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        format!("hash_{:x}", hasher.finish())
    }

    pub fn truncate_mask(len: usize) -> impl Fn(&str) -> String {
        move |value: &str| {
            if value.len() > len {
                format!("{}...", &value[..len])
            } else {
                value.to_string()
            }
        }
    }
}

/// Policy enforcement
pub struct PolicyEnforcer {
    policy: SessionPolicy,
}

impl PolicyEnforcer {
    pub fn new(policy: SessionPolicy) -> Self {
        Self { policy }
    }

    /// Check if model access is allowed
    pub fn check_model_access(&self, model_name: &str) -> Result<()> {
        if let Some(allowed) = &self.policy.allowed_models {
            if !allowed.contains(&model_name.to_string()) {
                return Err(anyhow!("Model '{}' not in allowed list", model_name));
            }
        }

        if let Some(denied) = &self.policy.denied_models {
            if denied.contains(&model_name.to_string()) {
                return Err(anyhow!("Model '{}' is denied", model_name));
            }
        }

        Ok(())
    }

    /// Check if datasource access is allowed
    pub fn check_datasource_access(&self, datasource: &str) -> Result<()> {
        if let Some(allowed) = &self.policy.allowed_datasources {
            if !allowed.contains(&datasource.to_string()) {
                return Err(anyhow!("Datasource '{}' not in allowed list", datasource));
            }
        }
        Ok(())
    }

    /// Apply forced filters to query
    pub fn apply_forced_filters(&self, query: &mut crate::models::Query) {
        for filter in &self.policy.forced_filters {
            query.filters.push(filter.clone());
        }
    }

    /// Apply row-level security
    pub fn apply_rls(&self, query: &mut crate::models::Query) {
        if let Some(rls_filter) = &self.policy.row_filter {
            query.filters.push(rls_filter.clone());
        }
    }

    /// Enforce row limit
    pub fn enforce_row_limit(&self, query: &mut crate::models::Query) {
        let max_rows = self.policy.max_rows.unwrap_or(10000);
        match query.limit {
            Some(limit) if limit > max_rows => query.limit = Some(max_rows),
            None => query.limit = Some(max_rows),
            _ => {}
        }
    }

    /// Get query timeout
    pub fn query_timeout(&self) -> u64 {
        self.policy.query_timeout_secs.unwrap_or(300)
    }
}

/// Audit log entry
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub action: String,
    pub model: Option<String>,
    pub query_hash: Option<String>,
    pub row_count: Option<usize>,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Audit logger
pub struct AuditLogger {
    entries: Vec<AuditEntry>,
    max_entries: usize,
}

impl AuditLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries,
        }
    }

    pub fn log(&mut self, entry: AuditEntry) {
        self.entries.push(entry);
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
    }

    pub fn get_entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
