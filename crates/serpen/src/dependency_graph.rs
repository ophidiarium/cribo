use anyhow::{Result, anyhow};
use indexmap::IndexSet;
use log::debug;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ModuleNode {
    pub name: String,
    pub path: PathBuf,
    pub imports: Vec<String>,
}

#[derive(Debug)]
pub struct DependencyGraph {
    graph: DiGraph<ModuleNode, ()>,
    node_indices: HashMap<String, NodeIndex>,
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
        }
    }

    /// Add a module to the graph
    pub fn add_module(&mut self, module: ModuleNode) -> NodeIndex {
        let module_name = module.name.clone();
        let module_path = module.path.clone();

        // 1) If name exists, update payload
        if let Some(&existing_index) = self.node_indices.get(&module_name) {
            self.graph[existing_index] = module;
            return existing_index;
        }
        // 2) Detect rename by matching path
        if let Some(old_key) = self.node_indices.iter().find_map(|(key, &idx)| {
            if self.graph[idx].path == module_path {
                Some(key.clone())
            } else {
                None
            }
        }) {
            // Remove old entry and insert new name
            let existing_index = self.node_indices.remove(&old_key).unwrap();
            self.node_indices
                .insert(module_name.clone(), existing_index);
            self.graph[existing_index] = module;
            return existing_index;
        }
        // 3) New module, add node
        let index = self.graph.add_node(module);
        self.node_indices.insert(module_name, index);
        index
    }

    /// Add a dependency edge between two modules
    pub fn add_dependency(&mut self, from_module: &str, to_module: &str) -> Result<()> {
        let from_index = self
            .node_indices
            .get(from_module)
            .ok_or_else(|| anyhow!("Module not found: {}", from_module))?;
        let to_index = self
            .node_indices
            .get(to_module)
            .ok_or_else(|| anyhow!("Module not found: {}", to_module))?;

        // Add edge if it doesn't already exist
        if !self.graph.contains_edge(*from_index, *to_index) {
            self.graph.add_edge(*from_index, *to_index, ());
        }

        Ok(())
    }

    /// Get topologically sorted modules (dependencies first)
    pub fn topological_sort(&self) -> Result<Vec<&ModuleNode>> {
        let sorted_indices = toposort(&self.graph, None).map_err(|cycle| {
            anyhow!(
                "Circular dependency detected involving module: {}",
                self.graph[cycle.node_id()].name
            )
        })?;

        // toposort already returns dependencies before dependents
        let sorted_modules: Vec<&ModuleNode> = sorted_indices
            .iter()
            .map(|&index| &self.graph[index])
            .collect();

        Ok(sorted_modules)
    }

    /// Get all modules in the graph
    pub fn get_modules(&self) -> Vec<&ModuleNode> {
        self.graph.node_weights().collect()
    }

    /// Get a module by name
    pub fn get_module(&self, name: &str) -> Option<&ModuleNode> {
        self.node_indices.get(name).map(|&index| &self.graph[index])
    }

    /// Get the dependencies of a module (modules that the given module imports)
    pub fn get_dependencies(&self, module_name: &str) -> Option<Vec<&str>> {
        let module_index = self.node_indices.get(module_name)?;

        // Incoming edges: from dependency -> dependent, so incoming neighbors are modules that this module depends on
        let dependencies: Vec<&str> = self
            .graph
            .neighbors_directed(*module_index, petgraph::Direction::Incoming)
            .map(|neighbor_index| self.graph[neighbor_index].name.as_str())
            .collect();

        Some(dependencies)
    }

    /// Check if the graph has cycles
    pub fn has_cycles(&self) -> bool {
        toposort(&self.graph, None).is_err()
    }

    /// Get modules that have no dependencies (entry points)
    pub fn get_entry_modules(&self) -> Vec<&ModuleNode> {
        self.graph
            .node_indices()
            .filter(|&index| {
                // Zero incoming edges means no dependencies
                self.graph
                    .neighbors_directed(index, petgraph::Direction::Incoming)
                    .count()
                    == 0
            })
            .map(|index| &self.graph[index])
            .collect()
    }

    /// Filter to only include modules reachable from the given entry module
    pub fn filter_reachable_from(&self, entry_module: &str) -> Result<DependencyGraph> {
        let entry_index = self.find_entry_module_index(entry_module)?;

        debug!("Filtering from entry module: {}", entry_module);

        // Use DFS to find all reachable modules
        let visited = self.find_reachable_modules_dfs(entry_index);

        debug!("Visited {} modules total", visited.len());
        self.log_visited_modules(&visited);

        // Create new graph with only reachable modules
        self.create_filtered_graph(visited)
    }

    /// Find the node index for the entry module
    fn find_entry_module_index(&self, entry_module: &str) -> Result<NodeIndex> {
        self.node_indices
            .get(entry_module)
            .copied()
            .ok_or_else(|| anyhow!("Entry module not found: {}", entry_module))
    }

    /// Find all modules reachable from the entry using DFS
    fn find_reachable_modules_dfs(&self, entry_index: NodeIndex) -> IndexSet<NodeIndex> {
        let mut visited = IndexSet::new();
        let mut stack = vec![entry_index];

        while let Some(current_index) = stack.pop() {
            if visited.insert(current_index) {
                self.process_current_module_for_dfs(current_index, &mut stack, &visited);
            }
        }

        visited
    }

    /// Process the current module during DFS traversal
    fn process_current_module_for_dfs(
        &self,
        current_index: NodeIndex,
        stack: &mut Vec<NodeIndex>,
        visited: &IndexSet<NodeIndex>,
    ) {
        let current_module = &self.graph[current_index].name;
        debug!("Visiting module: {}", current_module);

        // Add all dependencies to the stack
        let incoming_count = self
            .graph
            .neighbors_directed(current_index, petgraph::Direction::Incoming)
            .count();
        println!(
            "DEBUG: Module {} has {} incoming edges (dependencies)",
            current_module, incoming_count
        );

        for neighbor_index in self
            .graph
            .neighbors_directed(current_index, petgraph::Direction::Incoming)
        {
            self.process_neighbor_for_topological_sort(neighbor_index, stack, visited);
        }
    }

    /// Log all visited modules for debugging
    fn log_visited_modules(&self, visited: &IndexSet<NodeIndex>) {
        for &index in visited {
            debug!("Visited module: {}", self.graph[index].name);
        }
    }

    /// Create a new filtered graph containing only the visited modules
    fn create_filtered_graph(&self, visited: IndexSet<NodeIndex>) -> Result<DependencyGraph> {
        let mut filtered_graph = DependencyGraph::new();
        let mut _index_mapping = HashMap::new();

        // Add all reachable nodes
        self.add_reachable_nodes_to_filtered_graph(
            &visited,
            &mut filtered_graph,
            &mut _index_mapping,
        );

        // Add all edges between reachable nodes
        self.add_edges_to_filtered_graph(&visited, &mut filtered_graph)?;

        Ok(filtered_graph)
    }

    /// Add all reachable nodes to the filtered graph
    fn add_reachable_nodes_to_filtered_graph(
        &self,
        visited: &IndexSet<NodeIndex>,
        filtered_graph: &mut DependencyGraph,
        _index_mapping: &mut HashMap<NodeIndex, NodeIndex>,
    ) {
        for &old_index in visited {
            let module = self.graph[old_index].clone();
            let new_index = filtered_graph.add_module(module);
            _index_mapping.insert(old_index, new_index);
        }
    }

    /// Add all edges between reachable nodes to the filtered graph
    fn add_edges_to_filtered_graph(
        &self,
        visited: &IndexSet<NodeIndex>,
        filtered_graph: &mut DependencyGraph,
    ) -> Result<()> {
        for &from_index in visited {
            self.add_edges_for_module(from_index, visited, filtered_graph)?;
        }
        Ok(())
    }

    /// Add edges for a specific module to the filtered graph
    fn add_edges_for_module(
        &self,
        from_index: NodeIndex,
        visited: &IndexSet<NodeIndex>,
        filtered_graph: &mut DependencyGraph,
    ) -> Result<()> {
        for to_index in self
            .graph
            .neighbors_directed(from_index, petgraph::Direction::Incoming)
        {
            if visited.contains(&to_index) {
                let from_module = &self.graph[to_index].name; // dependency
                let to_module = &self.graph[from_index].name; // dependent
                filtered_graph.add_dependency(from_module, to_module)?;
            }
        }
        Ok(())
    }

    /// Process a neighbor node during topological sort
    fn process_neighbor_for_topological_sort(
        &self,
        neighbor_index: petgraph::graph::NodeIndex,
        stack: &mut Vec<petgraph::graph::NodeIndex>,
        visited: &indexmap::IndexSet<petgraph::graph::NodeIndex>,
    ) {
        let neighbor_module = &self.graph[neighbor_index].name;
        let current_module = &self.graph[stack.last().copied().unwrap_or(neighbor_index)].name;
        println!(
            "DEBUG: Found dependency: {} -> {}",
            neighbor_module, current_module
        );
        if !visited.contains(&neighbor_index) {
            debug!("Adding {} to stack", neighbor_module);
            stack.push(neighbor_index);
        } else {
            debug!("{} already visited", neighbor_module);
        }
    }
}
