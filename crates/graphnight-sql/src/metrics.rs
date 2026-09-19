use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Process-wide query metrics for Prometheus text exposition.
#[derive(Debug, Default)]
pub struct QueryMetrics {
    pub queries_total: AtomicU64,
    pub query_errors: AtomicU64,
    pub plan_cache_hits: AtomicU64,
    pub plan_cache_misses: AtomicU64,
    pub result_cache_hits: AtomicU64,
    pub result_cache_misses: AtomicU64,
    pub rows_returned: AtomicU64,
}

impl QueryMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn inc_queries(&self) {
        self.queries_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_errors(&self) {
        self.query_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_rows(&self, n: u64) {
        self.rows_returned.fetch_add(n, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        format!(
            "# HELP graphnight_queries_total Total semantic queries executed\n\
             # TYPE graphnight_queries_total counter\n\
             graphnight_queries_total {}\n\
             # HELP graphnight_query_errors_total Query execution errors\n\
             # TYPE graphnight_query_errors_total counter\n\
             graphnight_query_errors_total {}\n\
             # HELP graphnight_plan_cache_hits_total SQL plan cache hits\n\
             # TYPE graphnight_plan_cache_hits_total counter\n\
             graphnight_plan_cache_hits_total {}\n\
             # HELP graphnight_plan_cache_misses_total SQL plan cache misses\n\
             # TYPE graphnight_plan_cache_misses_total counter\n\
             graphnight_plan_cache_misses_total {}\n\
             # HELP graphnight_result_cache_hits_total Result cache hits\n\
             # TYPE graphnight_result_cache_hits_total counter\n\
             graphnight_result_cache_hits_total {}\n\
             # HELP graphnight_result_cache_misses_total Result cache misses\n\
             # TYPE graphnight_result_cache_misses_total counter\n\
             graphnight_result_cache_misses_total {}\n\
             # HELP graphnight_rows_returned_total Rows returned from executions\n\
             # TYPE graphnight_rows_returned_total counter\n\
             graphnight_rows_returned_total {}\n",
            self.queries_total.load(Ordering::Relaxed),
            self.query_errors.load(Ordering::Relaxed),
            self.plan_cache_hits.load(Ordering::Relaxed),
            self.plan_cache_misses.load(Ordering::Relaxed),
            self.result_cache_hits.load(Ordering::Relaxed),
            self.result_cache_misses.load(Ordering::Relaxed),
            self.rows_returned.load(Ordering::Relaxed),
        )
    }
}
