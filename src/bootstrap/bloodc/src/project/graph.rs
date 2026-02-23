//! Dependency graph for Blood modules.
//!
//! This module provides:
//! - Dependency tracking between modules
//! - Topological sorting for compilation order
//! - Cycle detection for circular dependency errors
//! - Invalidation propagation for incremental compilation

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::Path;
use thiserror::Error;
use serde::{Deserialize, Serialize};

use super::resolve::ModuleId;

/// Errors that can occur during dependency analysis.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("circular dependency detected: {cycle}")]
    CircularDependency { cycle: String },

    #[error("unknown module: {id:?}")]
    UnknownModule { id: ModuleId },

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for GraphError {
    fn from(e: serde_json::Error) -> Self {
        GraphError::Serialization(e.to_string())
    }
}

/// Dependency graph for modules.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// All nodes (modules) in the graph.
    nodes: HashSet<ModuleId>,

    /// Outgoing edges: module -> modules it depends on.
    dependencies: HashMap<ModuleId, HashSet<ModuleId>>,

    /// Incoming edges: module -> modules that depend on it.
    dependents: HashMap<ModuleId, HashSet<ModuleId>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        DependencyGraph::default()
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, id: ModuleId) {
        self.nodes.insert(id);
        self.dependencies.entry(id).or_default();
        self.dependents.entry(id).or_default();
    }

    /// Add a dependency: `from` depends on `to`.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) {
        self.nodes.insert(from);
        self.nodes.insert(to);
        self.dependencies.entry(from).or_default().insert(to);
        self.dependents.entry(to).or_default().insert(from);
    }

    /// Remove a dependency.
    pub fn remove_dependency(&mut self, from: ModuleId, to: ModuleId) {
        if let Some(deps) = self.dependencies.get_mut(&from) {
            deps.remove(&to);
        }
        if let Some(deps) = self.dependents.get_mut(&to) {
            deps.remove(&from);
        }
    }

    /// Get all modules that `id` depends on.
    pub fn dependencies_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.dependencies.get(&id).into_iter().flatten().copied()
    }

    /// Get all modules that depend on `id`.
    pub fn dependents_of(&self, id: ModuleId) -> impl Iterator<Item = ModuleId> + '_ {
        self.dependents.get(&id).into_iter().flatten().copied()
    }

    /// Check if `from` depends on `to` (directly or transitively).
    pub fn depends_on(&self, from: ModuleId, to: ModuleId) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(from);

        while let Some(current) = queue.pop_front() {
            if current == to {
                return true;
            }

            if visited.insert(current) {
                if let Some(deps) = self.dependencies.get(&current) {
                    queue.extend(deps.iter().copied());
                }
            }
        }

        false
    }

    /// Get all modules in the graph.
    pub fn modules(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.nodes.iter().copied()
    }

    /// Get the number of modules in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Detect cycles in the graph.
    ///
    /// Returns the first cycle found as a list of module IDs, or None if no cycle exists.
    pub fn find_cycle(&self) -> Option<Vec<ModuleId>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for &node in &self.nodes {
            if let Some(cycle) = self.dfs_find_cycle(node, &mut visited, &mut rec_stack, &mut path) {
                return Some(cycle);
            }
        }

        None
    }

    /// DFS helper for cycle detection.
    fn dfs_find_cycle(
        &self,
        node: ModuleId,
        visited: &mut HashSet<ModuleId>,
        rec_stack: &mut HashSet<ModuleId>,
        path: &mut Vec<ModuleId>,
    ) -> Option<Vec<ModuleId>> {
        if rec_stack.contains(&node) {
            // Found a cycle - extract the cycle from the path
            let cycle_start = path.iter().position(|&n| n == node).unwrap_or(0);
            let mut cycle: Vec<_> = path[cycle_start..].to_vec();
            cycle.push(node);
            return Some(cycle);
        }

        if visited.contains(&node) {
            return None;
        }

        visited.insert(node);
        rec_stack.insert(node);
        path.push(node);

        if let Some(deps) = self.dependencies.get(&node) {
            for &dep in deps {
                if let Some(cycle) = self.dfs_find_cycle(dep, visited, rec_stack, path) {
                    return Some(cycle);
                }
            }
        }

        path.pop();
        rec_stack.remove(&node);
        None
    }

    /// Compute a topological sort of the modules.
    ///
    /// Returns modules in order such that dependencies come before dependents.
    /// Returns an error if a cycle is detected.
    pub fn topological_sort(&self) -> Result<Vec<ModuleId>, GraphError> {
        // Check for cycles first
        if let Some(cycle) = self.find_cycle() {
            let cycle_str = cycle
                .iter()
                .map(|id| format!("{}", id.raw()))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(GraphError::CircularDependency { cycle: cycle_str });
        }

        // Kahn's algorithm
        let mut in_degree: HashMap<ModuleId, usize> = HashMap::new();
        for &node in &self.nodes {
            in_degree.insert(node, 0);
        }

        for deps in self.dependencies.values() {
            for &dep in deps {
                *in_degree.entry(dep).or_default() += 1;
            }
        }

        // Wait, that's backwards. The in_degree should count incoming edges.
        // An incoming edge to X means "something depends on X".
        // Let me fix this.

        let mut in_degree: HashMap<ModuleId, usize> = HashMap::new();
        for &node in &self.nodes {
            in_degree.insert(node, 0);
        }

        // For each node, count how many dependencies it has (how many it depends on)
        // Actually, for topological sort, we want nodes with no dependencies first.
        // So in_degree[node] = number of things node depends on.

        for (&node, deps) in &self.dependencies {
            in_degree.insert(node, deps.len());
        }

        // Start with nodes that have no dependencies
        let mut queue: VecDeque<_> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut result = Vec::new();

        while let Some(node) = queue.pop_front() {
            result.push(node);

            // For each module that depends on this node, decrement its in-degree
            if let Some(dependents) = self.dependents.get(&node) {
                for &dependent in dependents {
                    if let Some(deg) = in_degree.get_mut(&dependent) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }

        // If we didn't visit all nodes, there's a cycle (shouldn't happen since we checked)
        if result.len() != self.nodes.len() {
            // This shouldn't happen since we checked for cycles above
            return Err(GraphError::CircularDependency {
                cycle: "unknown cycle".to_string(),
            });
        }

        Ok(result)
    }

    /// Get modules that need recompilation when `changed` modules are modified.
    ///
    /// Returns all modules that directly or transitively depend on any changed module,
    /// plus the changed modules themselves.
    pub fn invalidation_set(&self, changed: &[ModuleId]) -> HashSet<ModuleId> {
        let mut result: HashSet<ModuleId> = changed.iter().copied().collect();
        let mut queue: VecDeque<_> = changed.iter().copied().collect();

        while let Some(current) = queue.pop_front() {
            if let Some(dependents) = self.dependents.get(&current) {
                for &dependent in dependents {
                    if result.insert(dependent) {
                        queue.push_back(dependent);
                    }
                }
            }
        }

        result
    }

    /// Find modules that can be compiled in parallel.
    ///
    /// Returns groups of modules where each group can be compiled concurrently
    /// (none of the modules in a group depend on each other).
    pub fn parallel_groups(&self) -> Result<Vec<Vec<ModuleId>>, GraphError> {
        let sorted = self.topological_sort()?;

        // Group modules by their "level" (distance from roots)
        let mut levels: HashMap<ModuleId, usize> = HashMap::new();

        for &node in &sorted {
            let max_dep_level = self.dependencies_of(node)
                .filter_map(|dep| levels.get(&dep))
                .max()
                .copied()
                .unwrap_or(0);

            let node_level = if self.dependencies.get(&node).map_or(true, |d| d.is_empty()) {
                0
            } else {
                max_dep_level + 1
            };

            levels.insert(node, node_level);
        }

        // Group by level
        let max_level = levels.values().max().copied().unwrap_or(0);
        let mut groups: Vec<Vec<ModuleId>> = vec![Vec::new(); max_level + 1];

        for (&node, &level) in &levels {
            groups[level].push(node);
        }

        // Remove empty groups
        groups.retain(|g| !g.is_empty());

        Ok(groups)
    }

    /// Save the dependency graph to a file.
    pub fn save(&self, path: &Path) -> Result<(), GraphError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Load a dependency graph from a file.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        let json = fs::read_to_string(path)?;
        let graph: DependencyGraph = serde_json::from_str(&json)?;
        Ok(graph)
    }

    /// Try to load a dependency graph from a file, returning a new empty graph if the file doesn't exist.
    pub fn load_or_new(path: &Path) -> Self {
        if path.exists() {
            Self::load(path).unwrap_or_else(|e| {
                eprintln!("warning: failed to load dependency graph from {}: {}; starting with empty graph", path.display(), e);
                Self::new()
            })
        } else {
            Self::new()
        }
    }

    /// Clear the graph.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.dependencies.clear();
        self.dependents.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_dependency() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);

        graph.add_dependency(a, b);

        assert!(graph.nodes.contains(&a));
        assert!(graph.nodes.contains(&b));
        assert!(graph.dependencies_of(a).any(|d| d == b));
        assert!(graph.dependents_of(b).any(|d| d == a));
    }

    #[test]
    fn test_depends_on_transitive() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);

        // a -> b -> c
        graph.add_dependency(a, b);
        graph.add_dependency(b, c);

        assert!(graph.depends_on(a, b));
        assert!(graph.depends_on(a, c)); // Transitive
        assert!(graph.depends_on(b, c));
        assert!(!graph.depends_on(c, a)); // No reverse dependency
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);

        // a -> b -> c -> a (cycle)
        graph.add_dependency(a, b);
        graph.add_dependency(b, c);
        graph.add_dependency(c, a);

        let cycle = graph.find_cycle();
        assert!(cycle.is_some());

        let cycle = cycle.unwrap();
        assert!(cycle.len() >= 3);
    }

    #[test]
    fn test_no_cycle() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);

        // a -> b, a -> c (no cycle)
        graph.add_dependency(a, b);
        graph.add_dependency(a, c);

        assert!(graph.find_cycle().is_none());
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);
        let d = ModuleId::new(3);

        // d depends on nothing
        // c depends on d
        // b depends on c
        // a depends on b
        graph.add_module(d);
        graph.add_dependency(c, d);
        graph.add_dependency(b, c);
        graph.add_dependency(a, b);

        let sorted = graph.topological_sort().unwrap();

        // d should come before c, c before b, b before a
        let pos_d = sorted.iter().position(|&x| x == d).unwrap();
        let pos_c = sorted.iter().position(|&x| x == c).unwrap();
        let pos_b = sorted.iter().position(|&x| x == b).unwrap();
        let pos_a = sorted.iter().position(|&x| x == a).unwrap();

        assert!(pos_d < pos_c);
        assert!(pos_c < pos_b);
        assert!(pos_b < pos_a);
    }

    #[test]
    fn test_topological_sort_cycle_error() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);

        // a -> b -> a (cycle)
        graph.add_dependency(a, b);
        graph.add_dependency(b, a);

        let result = graph.topological_sort();
        assert!(matches!(result, Err(GraphError::CircularDependency { .. })));
    }

    #[test]
    fn test_invalidation_set() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);
        let d = ModuleId::new(3);

        // a depends on b
        // c depends on b
        // d depends on c
        graph.add_dependency(a, b);
        graph.add_dependency(c, b);
        graph.add_dependency(d, c);

        // If b changes, a, c, and d need recompilation
        let invalid = graph.invalidation_set(&[b]);

        assert!(invalid.contains(&b)); // The changed module
        assert!(invalid.contains(&a)); // Depends on b
        assert!(invalid.contains(&c)); // Depends on b
        assert!(invalid.contains(&d)); // Depends on c (transitive)
    }

    #[test]
    fn test_parallel_groups() {
        let mut graph = DependencyGraph::new();
        let a = ModuleId::new(0);
        let b = ModuleId::new(1);
        let c = ModuleId::new(2);
        let d = ModuleId::new(3);

        // d and c have no dependencies (can compile in parallel)
        // b depends on d
        // a depends on b and c
        graph.add_module(c);
        graph.add_module(d);
        graph.add_dependency(b, d);
        graph.add_dependency(a, b);
        graph.add_dependency(a, c);

        let groups = graph.parallel_groups().unwrap();

        // Group 0: c, d (no dependencies)
        // Group 1: b (depends on d)
        // Group 2: a (depends on b, c)
        assert!(groups.len() >= 2);

        // d and c should be in the first group
        assert!(groups[0].contains(&d) || groups[0].contains(&c));
    }
}
