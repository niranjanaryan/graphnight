use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyList, PyModule};
use pyo3::Python;
use pyo3_asyncio::tokio::future_into_py;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use graphnight_core::models::{
    AggregationType, DataSource, Dimension, Filter, FilterOperator, Formula, Measure, Model,
    OrderBy, Query as CoreQuery, SourceSpec, TimeDimension, TimeGranularity,
};
use graphnight_sql::SqlEngine;
use graphnight_storage::{Memory, MemoryFilter, StorageBackend, YamlStorage};

/// Python wrapper for the GraphNight client
#[pyclass]
pub struct GraphNightClient {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
}

fn value_to_py(py: Python, value: Value) -> PyResult<PyObject> {
    match value {
        Value::Null => Ok(py.None()),
        Value::Bool(b) => Ok(b.into_py(py)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_py(py))
            } else if let Some(f) = n.as_f64() {
                Ok(PyFloat::new(py, f).into())
            } else if let Some(u) = n.as_u64() {
                Ok(u.into_py(py))
            } else {
                Ok(py.None())
            }
        }
        Value::String(s) => Ok(s.into_py(py)),
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.into_iter().map(|v| value_to_py(py, v)).collect();
            let list = PyList::new(py, items?);
            Ok(list.into())
        }
        Value::Object(obj) => {
            let dict = PyDict::new(py);
            for (k, v) in obj {
                dict.set_item(k, value_to_py(py, v)?)?;
            }
            Ok(dict.into())
        }
    }
}

fn storage_error_to_pyerr(e: graphnight_core::errors::StorageError) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
}

fn _anyhow_to_pyerr(e: anyhow::Error) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
}

#[pymethods]
impl GraphNightClient {
    #[new]
    #[pyo3(signature = (_url=None, storage_path=None))]
    fn new(_url: Option<String>, storage_path: Option<String>) -> PyResult<Self> {
        // For now, use local YAML storage
        let storage_path = storage_path.unwrap_or_else(|| "./graphnight_data".to_string());
        let storage = Arc::new(YamlStorage::new(&storage_path).map_err(storage_error_to_pyerr)?);

        // Load data from disk
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        rt.block_on(storage.load())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Create a default SQL engine with PostgreSQL dialect
        let dialect = graphnight_sql::dialects::get_dialect("postgres");
        let conn_manager = Arc::new(graphnight_sql::executor::ConnectionManager::new());
        let executor = Arc::new(graphnight_sql::executor::QueryExecutor::new(conn_manager));
        let sql_engine = Arc::new(
            SqlEngine::new(dialect, executor)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?,
        );

        Ok(Self {
            sql_engine,
            storage,
        })
    }

    /// Execute a query
    fn query(&self, py: Python, query: &PyDict) -> PyResult<PyObject> {
        let core_query = self
            .dict_to_query(query)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        let sql_engine = self.sql_engine.clone();
        let storage = self.storage.clone();

        future_into_py(py, async move {
            let start = std::time::Instant::now();

            // Get model to find datasource
            let model_name = core_query
                .name
                .as_ref()
                .or_else(|| core_query.source_model.as_ref().map(|s| &s.model))
                .ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                        "Query must have name or source_model",
                    )
                })?;

            let model = storage
                .get_model(model_name, None)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?
                .ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                        "Model not found: {}",
                        model_name
                    ))
                })?;

            let datasource = storage
                .get_datasource(&model.datasource)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?
                .ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                        "Datasource not found: {}",
                        model.datasource
                    ))
                })?;

            let sql = sql_engine
                .generate_sql(&core_query)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let results = sql_engine
                .execute_sqlx(&datasource, &sql)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

            let columns: Vec<String> = if !results.is_empty() {
                results[0].keys().cloned().collect()
            } else {
                vec![]
            };

            let data: Vec<Value> = results
                .into_iter()
                .map(|m| serde_json::to_value(m).unwrap())
                .collect();

            let mut response = HashMap::new();
            response.insert("data".to_string(), Value::Array(data));
            response.insert(
                "columns".to_string(),
                Value::Array(columns.into_iter().map(Value::String).collect()),
            );
            response.insert("sql".to_string(), Value::String(sql));
            response.insert(
                "execution_time_ms".to_string(),
                Value::Number(
                    serde_json::Number::from_f64(start.elapsed().as_millis() as f64).unwrap(),
                ),
            );

            // Convert to PyObject inside the async block
            Python::with_gil(|py| value_to_py(py, Value::Object(response.into_iter().collect())))
        })
        .map(|r| r.into())
    }

    /// Execute query and return as pandas DataFrame (requires pandas feature)
    fn query_df(&self, py: Python, query: &PyDict) -> PyResult<PyObject> {
        // This would require pandas integration
        // For now, return the same as query
        self.query(py, query)
    }

    /// Generate SQL without executing
    fn generate_sql(&self, query: &PyDict) -> PyResult<String> {
        let core_query = self
            .dict_to_query(query)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        self.sql_engine
            .generate_sql(&core_query)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// List models
    fn list_models(&self, py: Python, datasource: Option<String>) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let models = storage
                .list_models(datasource.as_deref())
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let result: Vec<Value> = models
                .into_iter()
                .map(|m| {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), Value::String(m.name));
                    map.insert("datasource".to_string(), Value::String(m.datasource));
                    map.insert(
                        "description".to_string(),
                        Value::String(m.description.unwrap_or_default()),
                    );
                    map.insert(
                        "measures".to_string(),
                        Value::Array(
                            m.measures
                                .iter()
                                .map(|m| Value::String(m.formula.expression.clone()))
                                .collect(),
                        ),
                    );
                    map.insert(
                        "dimensions".to_string(),
                        Value::Array(
                            m.dimensions
                                .iter()
                                .map(|d| Value::String(d.name.clone()))
                                .collect(),
                        ),
                    );
                    map.insert(
                        "time_dimensions".to_string(),
                        Value::Array(
                            m.time_dimensions
                                .iter()
                                .map(|t| Value::String(t.dimension.clone()))
                                .collect(),
                        ),
                    );
                    Value::Object(map.into_iter().collect())
                })
                .collect();
            Python::with_gil(|py| value_to_py(py, Value::Array(result)))
        })
        .map(|r| r.into())
    }

    /// Get a model
    fn get_model(
        &self,
        py: Python,
        name: String,
        datasource: Option<String>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let model = storage
                .get_model(&name, datasource.as_deref())
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let result = model
                .map(|m| {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), Value::String(m.name));
                    map.insert("datasource".to_string(), Value::String(m.datasource));
                    map.insert(
                        "description".to_string(),
                        Value::String(m.description.unwrap_or_default()),
                    );
                    map.insert(
                        "measures".to_string(),
                        Value::Array(
                            m.measures
                                .iter()
                                .map(|m| Value::String(m.formula.expression.clone()))
                                .collect(),
                        ),
                    );
                    map.insert(
                        "dimensions".to_string(),
                        Value::Array(
                            m.dimensions
                                .iter()
                                .map(|d| Value::String(d.name.clone()))
                                .collect(),
                        ),
                    );
                    map.insert(
                        "time_dimensions".to_string(),
                        Value::Array(
                            m.time_dimensions
                                .iter()
                                .map(|t| Value::String(t.dimension.clone()))
                                .collect(),
                        ),
                    );
                    Value::Object(map.into_iter().collect())
                })
                .unwrap_or(Value::Null);
            Python::with_gil(|py| value_to_py(py, result))
        })
        .map(|r| r.into())
    }

    /// Create a model
    fn create_model(&self, py: Python, model: &PyDict) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        // Extract data from model dict before async block
        let name = model
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let datasource = model
            .get_item("datasource")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let description = model
            .get_item("description")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());

        // Extract measures
        let mut measures = Vec::new();
        if let Ok(Some(m_list)) = model.get_item("measures") {
            if let Ok(m_vec) = m_list.extract::<Vec<Py<PyDict>>>() {
                for m in m_vec {
                    Python::with_gil(|py| {
                        let obj = m.as_ref(py);
                        let formula = obj
                            .get_item("formula")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let format = obj
                            .get_item("format")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let aggregation = obj
                            .get_item("aggregation")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "sum".to_string());

                        let agg = match aggregation.to_lowercase().as_str() {
                            "sum" => AggregationType::Sum,
                            "avg" => AggregationType::Avg,
                            "count" => AggregationType::Count,
                            "min" => AggregationType::Min,
                            "max" => AggregationType::Max,
                            "count_distinct" => AggregationType::CountDistinct,
                            _ => AggregationType::Sum,
                        };

                        measures.push(Measure {
                            formula: Formula {
                                expression: formula,
                                label,
                                format,
                            },
                            aggregation: agg,
                        });
                    });
                }
            }
        }

        // Extract dimensions
        let mut dimensions = Vec::new();
        if let Ok(Some(d_list)) = model.get_item("dimensions") {
            if let Ok(d_vec) = d_list.extract::<Vec<Py<PyDict>>>() {
                for d in d_vec {
                    Python::with_gil(|py| {
                        let obj = d.as_ref(py);
                        let name = obj
                            .get_item("name")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        dimensions.push(Dimension { name, label });
                    });
                }
            }
        }

        // Extract time_dimensions
        let mut time_dimensions = Vec::new();
        if let Ok(Some(td_list)) = model.get_item("time_dimensions") {
            if let Ok(td_vec) = td_list.extract::<Vec<Py<PyDict>>>() {
                for td in td_vec {
                    Python::with_gil(|py| {
                        let obj = td.as_ref(py);
                        let dimension = obj
                            .get_item("dimension")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let granularity = obj
                            .get_item("granularity")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "day".to_string());
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());

                        let gran = match granularity.to_lowercase().as_str() {
                            "second" => TimeGranularity::Second,
                            "minute" => TimeGranularity::Minute,
                            "hour" => TimeGranularity::Hour,
                            "day" => TimeGranularity::Day,
                            "week" => TimeGranularity::Week,
                            "month" => TimeGranularity::Month,
                            "quarter" => TimeGranularity::Quarter,
                            "year" => TimeGranularity::Year,
                            _ => TimeGranularity::Day,
                        };

                        time_dimensions.push(TimeDimension {
                            dimension,
                            granularity: gran,
                            label,
                        });
                    });
                }
            }
        }

        let core_model = Model {
            name,
            datasource,
            description,
            measures,
            dimensions,
            time_dimensions,
            joins: vec![],
            sql: None,
            meta: HashMap::new(),
        };

        future_into_py(py, async move {
            let created = storage
                .create_model(core_model)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let mut map = HashMap::new();
            map.insert("name".to_string(), Value::String(created.name));
            map.insert("datasource".to_string(), Value::String(created.datasource));
            map.insert(
                "description".to_string(),
                Value::String(created.description.unwrap_or_default()),
            );
            Python::with_gil(|py| value_to_py(py, Value::Object(map.into_iter().collect())))
        })
        .map(|r| r.into())
    }

    /// List datasources
    fn list_datasources(&self, py: Python) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let datasources = storage
                .list_datasources()
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let result: Vec<Value> = datasources
                .into_iter()
                .map(|d| {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), Value::String(d.name));
                    map.insert("driver".to_string(), Value::String(d.driver));
                    map.insert(
                        "description".to_string(),
                        Value::String(d.description.unwrap_or_default()),
                    );
                    map.insert(
                        "models".to_string(),
                        Value::Array(d.models.into_iter().map(Value::String).collect()),
                    );
                    Value::Object(map.into_iter().collect())
                })
                .collect();
            Python::with_gil(|py| value_to_py(py, Value::Array(result)))
        })
        .map(|r| r.into())
    }

    /// Create a datasource
    fn create_datasource(&self, py: Python, datasource: &PyDict) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        // Extract data from datasource dict before async block
        let name = datasource
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let driver = datasource
            .get_item("driver")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_else(|| "postgres".to_string());
        let connection_string = datasource
            .get_item("connection_string")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let description = datasource
            .get_item("description")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());
        let pool_size = datasource
            .get_item("pool_size")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<u32>().ok());

        let core_ds = DataSource {
            name,
            driver,
            connection_string,
            description,
            models: vec![],
            pool_size,
            meta: HashMap::new(),
        };

        future_into_py(py, async move {
            let created = storage
                .create_datasource(core_ds)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let mut map = HashMap::new();
            map.insert("name".to_string(), Value::String(created.name));
            map.insert("driver".to_string(), Value::String(created.driver));
            map.insert(
                "description".to_string(),
                Value::String(created.description.unwrap_or_default()),
            );
            Python::with_gil(|py| value_to_py(py, Value::Object(map.into_iter().collect())))
        })
        .map(|r| r.into())
    }

    /// Search models
    fn search(&self, py: Python, q: String, limit: Option<usize>) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let results = storage
                .search(&q, limit.unwrap_or(10))
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let result: Vec<Value> = results
                .into_iter()
                .map(|r| {
                    let mut map = HashMap::new();
                    map.insert("model_name".to_string(), Value::String(r.model_name));
                    map.insert("datasource".to_string(), Value::String(r.datasource));
                    map.insert(
                        "score".to_string(),
                        Value::Number(serde_json::Number::from_f64(r.score as f64).unwrap()),
                    );
                    map.insert(
                        "matched_fields".to_string(),
                        Value::Array(r.matched_fields.into_iter().map(Value::String).collect()),
                    );
                    map.insert("snippet".to_string(), Value::String(r.snippet));
                    Value::Object(map.into_iter().collect())
                })
                .collect();
            Python::with_gil(|py| value_to_py(py, Value::Array(result)))
        })
        .map(|r| r.into())
    }

    /// Save a memory
    fn save_memory(
        &self,
        py: Python,
        learning: String,
        linked_entities: Vec<String>,
        id: Option<String>,
        description: Option<String>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let memory = Memory {
                id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                learning,
                linked_entities,
                description,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                meta: HashMap::new(),
            };
            let saved = storage
                .save_memory(memory)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let mut map = HashMap::new();
            map.insert("id".to_string(), Value::String(saved.id));
            map.insert("learning".to_string(), Value::String(saved.learning));
            map.insert(
                "linked_entities".to_string(),
                Value::Array(
                    saved
                        .linked_entities
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
            map.insert(
                "description".to_string(),
                Value::String(saved.description.unwrap_or_default()),
            );
            map.insert(
                "created_at".to_string(),
                Value::String(saved.created_at.to_rfc3339()),
            );
            map.insert(
                "updated_at".to_string(),
                Value::String(saved.updated_at.to_rfc3339()),
            );
            Python::with_gil(|py| value_to_py(py, Value::Object(map.into_iter().collect())))
        })
        .map(|r| r.into())
    }

    /// List memories
    fn list_memories(
        &self,
        py: Python,
        query: Option<String>,
        entity: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let filter = MemoryFilter {
                query,
                entity,
                limit,
                offset,
            };
            let memories = storage
                .list_memories(filter)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let result: Vec<Value> = memories
                .into_iter()
                .map(|m| {
                    let mut map = HashMap::new();
                    map.insert("id".to_string(), Value::String(m.id));
                    map.insert("learning".to_string(), Value::String(m.learning));
                    map.insert(
                        "linked_entities".to_string(),
                        Value::Array(m.linked_entities.into_iter().map(Value::String).collect()),
                    );
                    map.insert(
                        "description".to_string(),
                        Value::String(m.description.unwrap_or_default()),
                    );
                    map.insert(
                        "created_at".to_string(),
                        Value::String(m.created_at.to_rfc3339()),
                    );
                    map.insert(
                        "updated_at".to_string(),
                        Value::String(m.updated_at.to_rfc3339()),
                    );
                    Value::Object(map.into_iter().collect())
                })
                .collect();
            Python::with_gil(|py| value_to_py(py, Value::Array(result)))
        })
        .map(|r| r.into())
    }

    /// Delete a memory
    fn forget_memory(&self, py: Python, id: String) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        future_into_py(py, async move {
            let success = storage
                .delete_memory(&id)
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            let mut map = HashMap::new();
            map.insert("success".to_string(), Value::Bool(success));
            map.insert("id".to_string(), Value::String(id));
            Python::with_gil(|py| value_to_py(py, Value::Object(map.into_iter().collect())))
        })
        .map(|r| r.into())
    }
}

impl GraphNightClient {
    fn dict_to_query(&self, dict: &PyDict) -> anyhow::Result<CoreQuery> {
        let mut query = CoreQuery::new();

        if let Ok(Some(name)) = dict.get_item("name") {
            if let Ok(name_str) = name.extract::<String>() {
                query.name = Some(name_str);
            }
        }

        if let Ok(Some(source_model)) = dict.get_item("source_model") {
            if let Ok(source_dict) = source_model.downcast::<PyDict>() {
                let model = source_dict
                    .get_item("model")
                    .ok()
                    .flatten()
                    .and_then(|v| v.extract::<String>().ok())
                    .unwrap_or_default();
                let datasource = source_dict
                    .get_item("datasource")
                    .ok()
                    .flatten()
                    .and_then(|v| v.extract::<String>().ok());
                let alias = source_dict
                    .get_item("alias")
                    .ok()
                    .flatten()
                    .and_then(|v| v.extract::<String>().ok());
                query.source_model = Some(SourceSpec {
                    model,
                    datasource,
                    alias,
                });
            }
        }

        if let Ok(Some(measures)) = dict.get_item("measures") {
            if let Ok(measures_list) = measures.extract::<Vec<Py<PyDict>>>() {
                for m in measures_list {
                    Python::with_gil(|py| {
                        let obj = m.as_ref(py);
                        let formula = obj
                            .get_item("formula")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let format = obj
                            .get_item("format")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let aggregation = obj
                            .get_item("aggregation")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "sum".to_string());

                        let agg = match aggregation.to_lowercase().as_str() {
                            "sum" => AggregationType::Sum,
                            "avg" => AggregationType::Avg,
                            "count" => AggregationType::Count,
                            "min" => AggregationType::Min,
                            "max" => AggregationType::Max,
                            "count_distinct" => AggregationType::CountDistinct,
                            _ => AggregationType::Sum,
                        };

                        query.measures.push(Measure {
                            formula: Formula {
                                expression: formula,
                                label,
                                format,
                            },
                            aggregation: agg,
                        });
                    });
                }
            }
        }

        if let Ok(Some(dimensions)) = dict.get_item("dimensions") {
            if let Ok(dim_list) = dimensions.extract::<Vec<Py<PyDict>>>() {
                for d in dim_list {
                    Python::with_gil(|py| {
                        let obj = d.as_ref(py);
                        let name = obj
                            .get_item("name")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        query.dimensions.push(Dimension { name, label });
                    });
                }
            }
        }

        if let Ok(Some(time_dimensions)) = dict.get_item("time_dimensions") {
            if let Ok(td_list) = time_dimensions.extract::<Vec<Py<PyDict>>>() {
                for td in td_list {
                    Python::with_gil(|py| {
                        let obj = td.as_ref(py);
                        let dimension = obj
                            .get_item("dimension")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let granularity = obj
                            .get_item("granularity")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "day".to_string());
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());

                        let gran = match granularity.to_lowercase().as_str() {
                            "second" => TimeGranularity::Second,
                            "minute" => TimeGranularity::Minute,
                            "hour" => TimeGranularity::Hour,
                            "day" => TimeGranularity::Day,
                            "week" => TimeGranularity::Week,
                            "month" => TimeGranularity::Month,
                            "quarter" => TimeGranularity::Quarter,
                            "year" => TimeGranularity::Year,
                            _ => TimeGranularity::Day,
                        };

                        query.time_dimensions.push(TimeDimension {
                            dimension,
                            granularity: gran,
                            label,
                        });
                    });
                }
            }
        }

        if let Ok(Some(filters)) = dict.get_item("filters") {
            if let Ok(filter_list) = filters.extract::<Vec<Py<PyDict>>>() {
                for f in filter_list {
                    Python::with_gil(|py| {
                        let obj = f.as_ref(py);
                        let field = obj
                            .get_item("field")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let operator = obj
                            .get_item("operator")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "eq".to_string());
                        let value = obj
                            .get_item("value")
                            .ok()
                            .flatten()
                            .and_then(|v| {
                                // Try to extract as String first (JSON string)
                                if let Ok(s) = v.extract::<String>() {
                                    // Try to parse as JSON
                                    serde_json::from_str::<Value>(&s)
                                        .ok()
                                        .or(Some(Value::String(s)))
                                } else if let Ok(n) = v.extract::<i64>() {
                                    Some(Value::Number(serde_json::Number::from(n)))
                                } else if let Ok(n) = v.extract::<f64>() {
                                    serde_json::Number::from_f64(n).map(Value::Number)
                                } else if let Ok(b) = v.extract::<bool>() {
                                    Some(Value::Bool(b))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(Value::Null);
                        let or_condition = obj
                            .get_item("or_condition")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<bool>().ok())
                            .unwrap_or(false);

                        let op = match operator.to_lowercase().as_str() {
                            "eq" => FilterOperator::Eq,
                            "neq" => FilterOperator::Neq,
                            "gt" => FilterOperator::Gt,
                            "gte" => FilterOperator::Gte,
                            "lt" => FilterOperator::Lt,
                            "lte" => FilterOperator::Lte,
                            "like" => FilterOperator::Like,
                            "ilike" => FilterOperator::ILike,
                            "in" => FilterOperator::In,
                            "not_in" => FilterOperator::NotIn,
                            "is_null" => FilterOperator::IsNull,
                            "is_not_null" => FilterOperator::IsNotNull,
                            "between" => FilterOperator::Between,
                            "not_between" => FilterOperator::NotBetween,
                            _ => FilterOperator::Eq,
                        };

                        query.filters.push(Filter {
                            field,
                            operator: op,
                            value,
                            or_condition,
                        });
                    });
                }
            }
        }

        if let Ok(Some(order)) = dict.get_item("order") {
            if let Ok(order_list) = order.extract::<Vec<Py<PyDict>>>() {
                for o in order_list {
                    Python::with_gil(|py| {
                        let obj = o.as_ref(py);
                        let field = obj
                            .get_item("field")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let descending = obj
                            .get_item("descending")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<bool>().ok())
                            .unwrap_or(false);
                        query.order.push(OrderBy { field, descending });
                    });
                }
            }
        }

        if let Ok(Some(limit)) = dict.get_item("limit") {
            if let Ok(limit_val) = limit.extract::<usize>() {
                query.limit = Some(limit_val);
            }
        }

        if let Ok(Some(offset)) = dict.get_item("offset") {
            if let Ok(offset_val) = offset.extract::<usize>() {
                query.offset = Some(offset_val);
            }
        }

        if let Ok(Some(whole_periods_only)) = dict.get_item("whole_periods_only") {
            if let Ok(val) = whole_periods_only.extract::<bool>() {
                query.whole_periods_only = Some(val);
            }
        }

        if let Ok(Some(distinct_dimension_values)) = dict.get_item("distinct_dimension_values") {
            if let Ok(val) = distinct_dimension_values.extract::<bool>() {
                query.distinct_dimension_values = Some(val);
            }
        }

        Ok(query)
    }

    fn _dict_to_model(dict: &PyDict) -> anyhow::Result<Model> {
        let name = dict
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let datasource = dict
            .get_item("datasource")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let description = dict
            .get_item("description")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());

        let mut measures = Vec::new();
        if let Ok(Some(m_list)) = dict.get_item("measures") {
            if let Ok(m_vec) = m_list.extract::<Vec<Py<PyDict>>>() {
                for m in m_vec {
                    Python::with_gil(|py| {
                        let obj = m.as_ref(py);
                        let formula = obj
                            .get_item("formula")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let format = obj
                            .get_item("format")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        let aggregation = obj
                            .get_item("aggregation")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "sum".to_string());

                        let agg = match aggregation.to_lowercase().as_str() {
                            "sum" => AggregationType::Sum,
                            "avg" => AggregationType::Avg,
                            "count" => AggregationType::Count,
                            "min" => AggregationType::Min,
                            "max" => AggregationType::Max,
                            "count_distinct" => AggregationType::CountDistinct,
                            _ => AggregationType::Sum,
                        };

                        measures.push(Measure {
                            formula: Formula {
                                expression: formula,
                                label,
                                format,
                            },
                            aggregation: agg,
                        });
                    });
                }
            }
        }

        let mut dimensions = Vec::new();
        if let Ok(Some(d_list)) = dict.get_item("dimensions") {
            if let Ok(d_vec) = d_list.extract::<Vec<Py<PyDict>>>() {
                for d in d_vec {
                    Python::with_gil(|py| {
                        let obj = d.as_ref(py);
                        let name = obj
                            .get_item("name")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());
                        dimensions.push(Dimension { name, label });
                    });
                }
            }
        }

        let mut time_dimensions = Vec::new();
        if let Ok(Some(td_list)) = dict.get_item("time_dimensions") {
            if let Ok(td_vec) = td_list.extract::<Vec<Py<PyDict>>>() {
                for td in td_vec {
                    Python::with_gil(|py| {
                        let obj = td.as_ref(py);
                        let dimension = obj
                            .get_item("dimension")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_default();
                        let granularity = obj
                            .get_item("granularity")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok())
                            .unwrap_or_else(|| "day".to_string());
                        let label = obj
                            .get_item("label")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<String>().ok());

                        let gran = match granularity.to_lowercase().as_str() {
                            "second" => TimeGranularity::Second,
                            "minute" => TimeGranularity::Minute,
                            "hour" => TimeGranularity::Hour,
                            "day" => TimeGranularity::Day,
                            "week" => TimeGranularity::Week,
                            "month" => TimeGranularity::Month,
                            "quarter" => TimeGranularity::Quarter,
                            "year" => TimeGranularity::Year,
                            _ => TimeGranularity::Day,
                        };

                        time_dimensions.push(TimeDimension {
                            dimension,
                            granularity: gran,
                            label,
                        });
                    });
                }
            }
        }

        Ok(Model {
            name,
            datasource,
            description,
            measures,
            dimensions,
            time_dimensions,
            joins: vec![],
            sql: None,
            meta: HashMap::new(),
        })
    }

    fn _dict_to_datasource(dict: &PyDict) -> anyhow::Result<DataSource> {
        let name = dict
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let driver = dict
            .get_item("driver")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_else(|| "postgres".to_string());
        let connection_string = dict
            .get_item("connection_string")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let description = dict
            .get_item("description")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());
        let pool_size = dict
            .get_item("pool_size")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<u32>().ok());

        Ok(DataSource {
            name,
            driver,
            connection_string,
            description,
            models: vec![],
            pool_size,
            meta: HashMap::new(),
        })
    }
}

/// Module definition
#[pymodule]
fn graphnight(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<GraphNightClient>()?;
    Ok(())
}
