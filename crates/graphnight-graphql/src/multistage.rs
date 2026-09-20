use anyhow::{anyhow, Result};
use graphnight_core::models::Query;
use graphnight_core::models::{Filter, FilterOperator};
use graphnight_sql::SqlEngine;
use graphnight_storage::StorageBackend;
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct StageResult {
    pub stage_name: String,
    pub query: Query,
    pub data: Vec<Map<String, Value>>,
    pub columns: Vec<String>,
    pub sql: String,
}

#[derive(Debug, Clone)]
pub struct StageInput {
    pub query: Query,
    pub stage_name: String,
    pub depends_on: Vec<String>,
}

pub struct MultiStageExecutor {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
    stage_results: Arc<RwLock<HashMap<String, StageResult>>>,
}

impl MultiStageExecutor {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
            stage_results: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn execute_dag(
        &self,
        stages: Vec<StageInput>,
    ) -> Result<Vec<StageResult>> {
        let mut sorted = self.topological_sort(&stages)?;
        let mut results = Vec::new();

        for stage in sorted.drain(..) {
            let result = self.execute_stage(&stage).await?;
            let stage_name = stage.stage_name.clone();

            self.stage_results
                .write()
                .await
                .insert(stage_name.clone(), result.clone());
            results.push(result);
        }

        Ok(results)
    }

    fn topological_sort(&self, stages: &[StageInput]) -> Result<Vec<StageInput>> {
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut stage_map: HashMap<String, StageInput> = HashMap::new();

        for stage in stages {
            graph.insert(stage.stage_name.clone(), stage.depends_on.clone());
            in_degree.insert(stage.stage_name.clone(), stage.depends_on.len());
            stage_map.insert(stage.stage_name.clone(), stage.clone());
        }

        let mut queue = VecDeque::new();
        for (name, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(name.clone());
            }
        }

        let mut sorted = Vec::new();
        while let Some(name) = queue.pop_front() {
            if let Some(stage) = stage_map.remove(&name) {
                sorted.push(stage);
            }

            for (other_name, deps) in &graph {
                if deps.contains(&name) {
                    let degree = in_degree.get_mut(other_name).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(other_name.clone());
                    }
                }
            }
        }

        if sorted.len() != stages.len() {
            let remaining: Vec<String> = stage_map.keys().cloned().collect();
            return Err(anyhow!(
                "Cycle detected in multi-stage query DAG. Remaining stages: {:?}",
                remaining
            ));
        }

        Ok(sorted)
    }

    async fn execute_stage(&self, stage: &StageInput) -> Result<StageResult> {
        let mut query = stage.query.clone();

        if let Some(ref stage_ref) = query.stage_ref {
            let stage_results = self.stage_results.read().await;
            if let Some(ref_result) = stage_results.get(stage_ref) {
                query = self.apply_stage_ref(query, ref_result)?;
            } else {
                return Err(anyhow!(
                    "Stage reference '{}' not found in previous results",
                    stage_ref
                ));
            }
        }

        let model_name = query
            .name
            .as_ref()
            .or_else(|| query.source_model.as_ref().map(|s| &s.model))
            .ok_or_else(|| anyhow!("Query must have a name or source_model"))?
            .clone();

        let model = self
            .storage
            .get_model(&model_name, None)
            .await?
            .ok_or_else(|| anyhow!("Model not found: {}", model_name))?;

        let datasource = self
            .storage
            .get_datasource(&model.datasource)
            .await?
            .ok_or_else(|| anyhow!("Datasource not found: {}", model.datasource))?;

        let sql = self.sql_engine.generate_sql(&query)?;
        let data = self.sql_engine.execute_sqlx(&datasource, &sql).await?;
        let columns = if !data.is_empty() {
            data[0].keys().cloned().collect()
        } else {
            vec![]
        };

        Ok(StageResult {
            stage_name: stage.stage_name.clone(),
            query,
            data,
            columns,
            sql,
        })
    }

    fn apply_stage_ref(&self, mut query: Query, ref_result: &StageResult) -> Result<Query> {
        if let Some(ref_data) = ref_result.data.first() {
            for (key, value) in ref_data {
                let filter = Filter::new(
                    key.clone(),
                    FilterOperator::Eq,
                    value.clone(),
                );
                query.filters.push(filter);
            }
        }
        Ok(query)
    }
}