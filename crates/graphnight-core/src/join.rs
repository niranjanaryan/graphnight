use crate::models::{Join, JoinType, Model};
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};

/// Join graph for resolving join paths
pub struct JoinGraph {
    models: HashMap<String, Model>,
    adjacency: HashMap<String, Vec<JoinEdge>>,
}

#[derive(Debug, Clone)]
pub struct JoinEdge {
    pub target_model: String,
    pub join: Join,
    pub reverse: bool,
}

impl JoinGraph {
    pub fn new(models: Vec<Model>) -> Self {
        let mut graph = Self {
            models: HashMap::new(),
            adjacency: HashMap::new(),
        };

        for model in models {
            graph.add_model(model);
        }

        graph
    }

    fn add_model(&mut self, model: Model) {
        let name = model.name.clone();

        // Add edges for each join
        for join in &model.joins {
            let edge = JoinEdge {
                target_model: join.model.clone(),
                join: join.clone(),
                reverse: false,
            };
            self.adjacency.entry(name.clone()).or_default().push(edge);

            // Add reverse edge
            let reverse_edge = JoinEdge {
                target_model: name.clone(),
                join: Join {
                    name: format!("{}_reverse", join.name),
                    model: name.clone(),
                    join_type: join.join_type.clone(),
                    on: join
                        .on
                        .iter()
                        .map(|(l, r)| (r.clone(), l.clone()))
                        .collect(),
                    alias: None,
                },
                reverse: true,
            };
            self.adjacency
                .entry(join.model.clone())
                .or_default()
                .push(reverse_edge);
        }

        self.models.insert(name, model);
    }

    /// Find join path between two models
    pub fn find_path(&self, from: &str, to: &str) -> Result<Vec<JoinEdge>> {
        if from == to {
            return Ok(vec![]);
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((from.to_string(), vec![]));
        visited.insert(from.to_string());

        while let Some((current, path)) = queue.pop_front() {
            if current == to {
                return Ok(path);
            }

            if let Some(edges) = self.adjacency.get(&current) {
                for edge in edges {
                    if !visited.contains(&edge.target_model) {
                        visited.insert(edge.target_model.clone());
                        let mut new_path = path.clone();
                        new_path.push(edge.clone());
                        queue.push_back((edge.target_model.clone(), new_path));
                    }
                }
            }
        }

        Err(anyhow!("No join path found from {} to {}", from, to))
    }

    /// Resolve all joins needed for a query
    pub fn resolve_joins(
        &self,
        base_model: &str,
        required_models: &[String],
    ) -> Result<Vec<JoinEdge>> {
        let mut all_edges = Vec::new();

        for model in required_models {
            if model != base_model {
                let path = self.find_path(base_model, model)?;
                all_edges.extend(path);
            }
        }

        // Deduplicate by target model
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for edge in all_edges {
            if seen.insert(edge.target_model.clone()) {
                result.push(edge);
            }
        }

        Ok(result)
    }
}

/// Join walker for building join SQL
pub struct JoinWalker {
    graph: JoinGraph,
}

impl JoinWalker {
    pub fn new(models: Vec<Model>) -> Self {
        Self {
            graph: JoinGraph::new(models),
        }
    }

    /// Walk joins and build SQL join clauses
    pub fn build_join_clauses(
        &self,
        base_model: &str,
        required_models: &[String],
        base_alias: &str,
    ) -> Result<Vec<JoinClause>> {
        let edges = self.graph.resolve_joins(base_model, required_models)?;
        let mut clauses = Vec::new();

        for edge in edges {
            let join = &edge.join;
            let target_alias = join.alias.as_deref().unwrap_or(&join.model);
            let join_type = match join.join_type {
                JoinType::Inner => "INNER JOIN",
                JoinType::Left => "LEFT JOIN",
                JoinType::Right => "RIGHT JOIN",
                JoinType::Full => "FULL JOIN",
            };

            let mut on_conditions = Vec::new();
            for (left, right) in &join.on {
                let left_col = if edge.reverse {
                    format!("{}.{}", target_alias, left)
                } else {
                    format!("{}.{}", base_alias, left)
                };
                let right_col = if edge.reverse {
                    format!("{}.{}", base_alias, right)
                } else {
                    format!("{}.{}", target_alias, right)
                };
                on_conditions.push(format!("{} = {}", left_col, right_col));
            }

            clauses.push(JoinClause {
                join_type: join_type.to_string(),
                table: join.model.clone(),
                alias: target_alias.to_string(),
                on: on_conditions.join(" AND "),
            });
        }

        Ok(clauses)
    }
}

#[derive(Debug, Clone)]
pub struct JoinClause {
    pub join_type: String,
    pub table: String,
    pub alias: String,
    pub on: String,
}
