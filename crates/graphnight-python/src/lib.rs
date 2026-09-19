use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyList, PyModule};
use pyo3::Python;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use graphnight_core::models::{
    AggregationType, DataSource, Dimension, Filter, FilterOperator, Formula, Measure, Model,
    OrderBy, Query as CoreQuery, SourceSpec, TimeDimension, TimeGranularity,
};
use graphnight_sql::SqlEngine;
use graphnight_storage::{Memory, MemoryFilter, StorageBackend, YamlStorage};

/// Local YAML-backed GraphNight client for embedding in Python.
#[pyclass]
pub struct GraphNightClient {
    sql_engine: Mutex<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
    runtime: tokio::runtime::Runtime,
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

fn runtime_error(msg: impl ToString) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(msg.to_string())
}

fn parse_aggregation(s: &str) -> AggregationType {
    match s.to_lowercase().as_str() {
        "sum" => AggregationType::Sum,
        "avg" => AggregationType::Avg,
        "count" => AggregationType::Count,
        "min" => AggregationType::Min,
        "max" => AggregationType::Max,
        "count_distinct" => AggregationType::CountDistinct,
        _ => AggregationType::Sum,
    }
}

fn parse_granularity(s: &str) -> TimeGranularity {
    match s.to_lowercase().as_str() {
        "second" => TimeGranularity::Second,
        "minute" => TimeGranularity::Minute,
        "hour" => TimeGranularity::Hour,
        "day" => TimeGranularity::Day,
        "week" => TimeGranularity::Week,
        "month" => TimeGranularity::Month,
        "quarter" => TimeGranularity::Quarter,
        "year" => TimeGranularity::Year,
        _ => TimeGranularity::Day,
    }
}

fn parse_filter_operator(s: &str) -> FilterOperator {
    match s.to_lowercase().as_str() {
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
    }
}

/// Accept formula as a string or `{"expression", "label?", "format?"}`.
fn extract_formula(obj: &PyDict) -> Formula {
    if let Ok(Some(f)) = obj.get_item("formula") {
        if let Ok(s) = f.extract::<String>() {
            return Formula::new(s);
        }
        if let Ok(fd) = f.downcast::<PyDict>() {
            let expression = fd
                .get_item("expression")
                .ok()
                .flatten()
                .and_then(|v| v.extract::<String>().ok())
                .unwrap_or_default();
            let label = fd
                .get_item("label")
                .ok()
                .flatten()
                .and_then(|v| v.extract::<String>().ok());
            let format = fd
                .get_item("format")
                .ok()
                .flatten()
                .and_then(|v| v.extract::<String>().ok());
            return Formula {
                expression,
                label,
                format,
            };
        }
    }
    Formula::new("")
}

fn extract_measure(obj: &PyDict) -> Measure {
    let formula = extract_formula(obj);
    let aggregation = obj
        .get_item("aggregation")
        .ok()
        .flatten()
        .and_then(|v| v.extract::<String>().ok())
        .unwrap_or_else(|| "sum".to_string());
    Measure {
        formula,
        aggregation: parse_aggregation(&aggregation),
    }
}

fn extract_dimension_from_obj(obj: &PyAny) -> Option<Dimension> {
    if let Ok(name) = obj.extract::<String>() {
        return Some(Dimension::new(name));
    }
    if let Ok(d) = obj.downcast::<PyDict>() {
        let name = d
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let label = d
            .get_item("label")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());
        return Some(Dimension { name, label });
    }
    None
}

fn extract_time_dimension(obj: &PyDict) -> TimeDimension {
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
    TimeDimension {
        dimension,
        granularity: parse_granularity(&granularity),
        label,
    }
}

fn extract_py_value(v: &PyAny) -> Value {
    if let Ok(s) = v.extract::<String>() {
        return serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s));
    }
    if let Ok(n) = v.extract::<i64>() {
        return Value::Number(serde_json::Number::from(n));
    }
    if let Ok(n) = v.extract::<f64>() {
        return serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Ok(b) = v.extract::<bool>() {
        return Value::Bool(b);
    }
    Value::Null
}

fn model_summary(m: &Model) -> Value {
    let mut map = HashMap::new();
    map.insert("name".to_string(), Value::String(m.name.clone()));
    map.insert(
        "datasource".to_string(),
        Value::String(m.datasource.clone()),
    );
    map.insert(
        "description".to_string(),
        Value::String(m.description.clone().unwrap_or_default()),
    );
    map.insert(
        "measures".to_string(),
        Value::Array(
            m.measures
                .iter()
                .map(|meas| Value::String(meas.formula.expression.clone()))
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
}

fn memory_to_value(m: &Memory) -> Value {
    let mut map = HashMap::new();
    map.insert("id".to_string(), Value::String(m.id.clone()));
    map.insert("learning".to_string(), Value::String(m.learning.clone()));
    map.insert(
        "linked_entities".to_string(),
        Value::Array(
            m.linked_entities
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    map.insert(
        "description".to_string(),
        Value::String(m.description.clone().unwrap_or_default()),
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
}

#[pymethods]
impl GraphNightClient {
    /// Create a client backed by local YAML storage.
    ///
    /// `url` is reserved for a future remote mode and is currently ignored.
    /// `storage_path` defaults to `./graphnight_data`.
    #[new]
    #[pyo3(signature = (_url=None, storage_path=None))]
    fn new(_url: Option<String>, storage_path: Option<String>) -> PyResult<Self> {
        let storage_path = storage_path.unwrap_or_else(|| "./graphnight_data".to_string());
        let storage = Arc::new(YamlStorage::new(&storage_path).map_err(storage_error_to_pyerr)?);

        let runtime = tokio::runtime::Runtime::new().map_err(|e| runtime_error(e))?;
        runtime
            .block_on(storage.load())
            .map_err(|e| runtime_error(e))?;

        let models = runtime
            .block_on(storage.list_models(None))
            .map_err(|e| runtime_error(e))?;

        let dialect = graphnight_sql::dialects::get_dialect("postgres");
        let conn_manager = Arc::new(graphnight_sql::executor::ConnectionManager::new());
        let executor = Arc::new(graphnight_sql::executor::QueryExecutor::new(conn_manager));
        let sql_engine = SqlEngine::new(dialect, executor)
            .map_err(|e| runtime_error(e))?
            .with_models(models);

        Ok(Self {
            sql_engine: Mutex::new(sql_engine),
            storage,
            runtime,
        })
    }

    /// Execute a query against the model's datasource and return data + SQL.
    fn query(&self, py: Python, query: &PyDict) -> PyResult<PyObject> {
        let core_query = self.dict_to_query(query).map_err(|e| runtime_error(e))?;
        let storage = self.storage.clone();
        let start = std::time::Instant::now();

        let model_name = core_query
            .name
            .as_ref()
            .or_else(|| core_query.source_model.as_ref().map(|s| &s.model))
            .ok_or_else(|| runtime_error("Query must have name or source_model"))?
            .clone();

        let model = self
            .runtime
            .block_on(storage.get_model(&model_name, None))
            .map_err(|e| runtime_error(e))?
            .ok_or_else(|| runtime_error(format!("Model not found: {}", model_name)))?;

        let datasource = self
            .runtime
            .block_on(storage.get_datasource(&model.datasource))
            .map_err(|e| runtime_error(e))?
            .ok_or_else(|| {
                runtime_error(format!("Datasource not found: {}", model.datasource))
            })?;

        let engine = self
            .sql_engine
            .lock()
            .map_err(|e| runtime_error(e.to_string()))?;
        let sql = engine
            .generate_sql(&core_query)
            .map_err(|e| runtime_error(e))?;
        let results = self
            .runtime
            .block_on(engine.execute_sqlx(&datasource, &sql))
            .map_err(|e| runtime_error(e))?;
        drop(engine);

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

        value_to_py(py, Value::Object(response.into_iter().collect()))
    }

    /// Same as `query` today; pandas DataFrame conversion is planned for the pandas extra.
    fn query_df(&self, py: Python, query: &PyDict) -> PyResult<PyObject> {
        self.query(py, query)
    }

    /// Generate SQL for a query without executing it.
    fn generate_sql(&self, query: &PyDict) -> PyResult<String> {
        let core_query = self.dict_to_query(query).map_err(|e| runtime_error(e))?;
        let engine = self
            .sql_engine
            .lock()
            .map_err(|e| runtime_error(e.to_string()))?;
        engine
            .generate_sql(&core_query)
            .map_err(|e| runtime_error(e))
    }

    /// Dry-run a query: return generated SQL (and echo of the model name) without hitting a DB.
    fn dry_run_query(&self, py: Python, query: &PyDict) -> PyResult<PyObject> {
        let core_query = self.dict_to_query(query).map_err(|e| runtime_error(e))?;
        let engine = self
            .sql_engine
            .lock()
            .map_err(|e| runtime_error(e.to_string()))?;
        let sql = engine
            .generate_sql(&core_query)
            .map_err(|e| runtime_error(e))?;

        let mut map = HashMap::new();
        map.insert("sql".to_string(), Value::String(sql));
        if let Some(name) = core_query.name {
            map.insert("name".to_string(), Value::String(name));
        }
        value_to_py(py, Value::Object(map.into_iter().collect()))
    }

    /// List models, optionally filtered by datasource name.
    #[pyo3(signature = (datasource=None))]
    fn list_models(&self, py: Python, datasource: Option<String>) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let models = self
            .runtime
            .block_on(async move { storage.list_models(datasource.as_deref()).await })
            .map_err(|e| runtime_error(e))?;
        let result: Vec<Value> = models.iter().map(model_summary).collect();
        value_to_py(py, Value::Array(result))
    }

    /// Fetch one model by name.
    #[pyo3(signature = (name, datasource=None))]
    fn get_model(
        &self,
        py: Python,
        name: String,
        datasource: Option<String>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let model = self
            .runtime
            .block_on(async move { storage.get_model(&name, datasource.as_deref()).await })
            .map_err(|e| runtime_error(e))?;
        let result = model.map(|m| model_summary(&m)).unwrap_or(Value::Null);
        value_to_py(py, result)
    }

    /// Create (persist) a semantic model from a Python dict.
    fn create_model(&self, py: Python, model: &PyDict) -> PyResult<PyObject> {
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

        let mut measures = Vec::new();
        if let Ok(Some(m_list)) = model.get_item("measures") {
            if let Ok(list) = m_list.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(d) = item.downcast::<PyDict>() {
                        measures.push(extract_measure(d));
                    }
                }
            }
        }

        let mut dimensions = Vec::new();
        if let Ok(Some(d_list)) = model.get_item("dimensions") {
            if let Ok(list) = d_list.downcast::<PyList>() {
                for item in list.iter() {
                    if let Some(dim) = extract_dimension_from_obj(item) {
                        dimensions.push(dim);
                    }
                }
            }
        }

        let mut time_dimensions = Vec::new();
        if let Ok(Some(td_list)) = model.get_item("time_dimensions") {
            if let Ok(list) = td_list.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(d) = item.downcast::<PyDict>() {
                        time_dimensions.push(extract_time_dimension(d));
                    }
                }
            }
        }

        let sql = model
            .get_item("sql")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok());

        let core_model = Model {
            name,
            datasource,
            description,
            measures,
            dimensions,
            time_dimensions,
            joins: vec![],
            sql,
            meta: HashMap::new(),
        };

        let storage = self.storage.clone();
        let created = self
            .runtime
            .block_on(async move { storage.create_model(core_model).await })
            .map_err(|e| runtime_error(e))?;

        {
            let mut engine = self
                .sql_engine
                .lock()
                .map_err(|e| runtime_error(e.to_string()))?;
            engine.register_model(created.clone());
        }

        let mut map = HashMap::new();
        map.insert("name".to_string(), Value::String(created.name));
        map.insert("datasource".to_string(), Value::String(created.datasource));
        map.insert(
            "description".to_string(),
            Value::String(created.description.unwrap_or_default()),
        );
        value_to_py(py, Value::Object(map.into_iter().collect()))
    }

    /// List datasources.
    fn list_datasources(&self, py: Python) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let datasources = self
            .runtime
            .block_on(async move { storage.list_datasources().await })
            .map_err(|e| runtime_error(e))?;
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
        value_to_py(py, Value::Array(result))
    }

    /// Create (persist) a datasource.
    fn create_datasource(&self, py: Python, datasource: &PyDict) -> PyResult<PyObject> {
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

        let storage = self.storage.clone();
        let created = self
            .runtime
            .block_on(async move { storage.create_datasource(core_ds).await })
            .map_err(|e| runtime_error(e))?;

        let mut map = HashMap::new();
        map.insert("name".to_string(), Value::String(created.name));
        map.insert("driver".to_string(), Value::String(created.driver));
        map.insert(
            "description".to_string(),
            Value::String(created.description.unwrap_or_default()),
        );
        value_to_py(py, Value::Object(map.into_iter().collect()))
    }

    /// Full-text / substring search over models (backend-dependent).
    #[pyo3(signature = (q, limit=None))]
    fn search(&self, py: Python, q: String, limit: Option<usize>) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let results = self
            .runtime
            .block_on(async move { storage.search(&q, limit.unwrap_or(10)).await })
            .map_err(|e| runtime_error(e))?;
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
        value_to_py(py, Value::Array(result))
    }

    /// Persist an agent/human learning linked to semantic entities.
    #[pyo3(signature = (learning, linked_entities, id=None, description=None))]
    fn save_memory(
        &self,
        py: Python,
        learning: String,
        linked_entities: Vec<String>,
        id: Option<String>,
        description: Option<String>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let memory = Memory {
            id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            learning,
            linked_entities,
            description,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: HashMap::new(),
        };
        let saved = self
            .runtime
            .block_on(async move { storage.save_memory(memory).await })
            .map_err(|e| runtime_error(e))?;
        value_to_py(py, memory_to_value(&saved))
    }

    /// List memories with optional query/entity filters.
    #[pyo3(signature = (query=None, entity=None, limit=None, offset=None))]
    fn list_memories(
        &self,
        py: Python,
        query: Option<String>,
        entity: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let filter = MemoryFilter {
            query,
            entity,
            limit,
            offset,
        };
        let memories = self
            .runtime
            .block_on(async move { storage.list_memories(filter).await })
            .map_err(|e| runtime_error(e))?;
        let result: Vec<Value> = memories.iter().map(memory_to_value).collect();
        value_to_py(py, Value::Array(result))
    }

    /// Delete a memory by id.
    fn forget_memory(&self, py: Python, id: String) -> PyResult<PyObject> {
        let storage = self.storage.clone();
        let id_for_delete = id.clone();
        let success = self
            .runtime
            .block_on(async move { storage.delete_memory(&id_for_delete).await })
            .map_err(|e| runtime_error(e))?;
        let mut map = HashMap::new();
        map.insert("success".to_string(), Value::Bool(success));
        map.insert("id".to_string(), Value::String(id));
        value_to_py(py, Value::Object(map.into_iter().collect()))
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
            if let Ok(list) = measures.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(obj) = item.downcast::<PyDict>() {
                        query.measures.push(extract_measure(obj));
                    }
                }
            }
        }

        if let Ok(Some(dimensions)) = dict.get_item("dimensions") {
            if let Ok(list) = dimensions.downcast::<PyList>() {
                for item in list.iter() {
                    if let Some(dim) = extract_dimension_from_obj(item) {
                        query.dimensions.push(dim);
                    }
                }
            }
        }

        if let Ok(Some(time_dimensions)) = dict.get_item("time_dimensions") {
            if let Ok(list) = time_dimensions.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(obj) = item.downcast::<PyDict>() {
                        query.time_dimensions.push(extract_time_dimension(obj));
                    }
                }
            }
        }

        if let Ok(Some(filters)) = dict.get_item("filters") {
            if let Ok(list) = filters.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(obj) = item.downcast::<PyDict>() {
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
                            .map(extract_py_value)
                            .unwrap_or(Value::Null);
                        let or_condition = obj
                            .get_item("or_condition")
                            .ok()
                            .flatten()
                            .and_then(|v| v.extract::<bool>().ok())
                            .unwrap_or(false);

                        query.filters.push(Filter {
                            field,
                            operator: parse_filter_operator(&operator),
                            value,
                            or_condition,
                        });
                    }
                }
            }
        }

        if let Ok(Some(order)) = dict.get_item("order") {
            if let Ok(list) = order.downcast::<PyList>() {
                for item in list.iter() {
                    if let Ok(obj) = item.downcast::<PyDict>() {
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
                    }
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
}

#[pymodule]
fn graphnight(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<GraphNightClient>()?;
    m.add(
        "__doc__",
        "GraphNight: embeddable semantic layer (local YAML client).",
    )?;
    Ok(())
}
