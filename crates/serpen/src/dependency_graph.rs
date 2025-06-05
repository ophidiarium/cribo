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

/// Comprehensive analysis of circular dependencies in the module dependency graph.
///
/// This struct provides detailed information about detected circular dependencies,
/// categorizing them into resolvable and unresolvable types with specific resolution
/// strategies for each category.
#[derive(Debug, Clone)]
pub struct CircularDependencyAnalysis {
    /// Circular dependencies that can be resolved through code transformations
    pub resolvable_cycles: Vec<CircularDependencyGroup>,
    /// Circular dependencies that cannot be resolved (e.g., temporal paradox patterns)
    pub unresolvable_cycles: Vec<CircularDependencyGroup>,
    /// Total number of cycles detected across all strongly connected components
    pub total_cycles_detected: usize,
    /// Size of the largest cycle (number of modules involved)
    pub largest_cycle_size: usize,
}

#[derive(Debug, Clone)]
pub struct CircularDependencyGroup {
    pub modules: Vec<String>,
    pub cycle_type: CircularDependencyType,
    pub import_chain: Vec<ImportEdge>,
    pub suggested_resolution: ResolutionStrategy,
}

#[derive(Debug, Clone)]
pub enum CircularDependencyType {
    FunctionLevel,   // Can be resolved by moving imports inside functions
    ClassLevel,      // May be resolvable depending on usage patterns
    ModuleConstants, // Unresolvable - temporal paradox
    ImportTime,      // Depends on execution order
}

#[derive(Debug, Clone)]
pub enum ResolutionStrategy {
    LazyImport { modules: Vec<String> },
    FunctionScopedImport { import_statements: Vec<String> },
    ModuleSplit { suggestions: Vec<String> },
    Unresolvable { reason: String },
}

#[derive(Debug, Clone)]
pub struct ImportEdge {
    pub from_module: String,
    pub to_module: String,
    pub import_type: ImportType,
    pub line_number: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum ImportType {
    Direct,         // import module
    FromImport,     // from module import item
    RelativeImport, // from .module import item
    AliasedImport,  // import module as alias
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

    /// Find all strongly connected components using Tarjan's algorithm
    pub fn find_strongly_connected_components(&self) -> Vec<Vec<NodeIndex>> {
        let mut index_counter = 0;
        let mut stack = Vec::new();
        let mut indices = HashMap::new();
        let mut lowlinks = HashMap::new();
        let mut on_stack = HashMap::new();
        let mut components = Vec::new();

        for node_index in self.graph.node_indices() {
            if !indices.contains_key(&node_index) {
                self.tarjan_strongconnect(
                    node_index,
                    &mut index_counter,
                    &mut stack,
                    &mut indices,
                    &mut lowlinks,
                    &mut on_stack,
                    &mut components,
                );
            }
        }

        components
    }

    /// Tarjan's strongly connected components algorithm implementation
    fn tarjan_strongconnect(
        &self,
        v: NodeIndex,
        index_counter: &mut usize,
        stack: &mut Vec<NodeIndex>,
        indices: &mut HashMap<NodeIndex, usize>,
        lowlinks: &mut HashMap<NodeIndex, usize>,
        on_stack: &mut HashMap<NodeIndex, bool>,
        components: &mut Vec<Vec<NodeIndex>>,
    ) {
        // Set the depth index for v to the smallest unused index
        indices.insert(v, *index_counter);
        lowlinks.insert(v, *index_counter);
        *index_counter += 1;
        stack.push(v);
        on_stack.insert(v, true);

        // Consider successors of v (outgoing edges in dependency graph)
        for w in self
            .graph
            .neighbors_directed(v, petgraph::Direction::Outgoing)
        {
            if !indices.contains_key(&w) {
                // Successor w has not yet been visited; recurse on it
                self.tarjan_strongconnect(
                    w,
                    index_counter,
                    stack,
                    indices,
                    lowlinks,
                    on_stack,
                    components,
                );
                let w_lowlink = *lowlinks.get(&w).unwrap();
                let v_lowlink = *lowlinks.get(&v).unwrap();
                lowlinks.insert(v, v_lowlink.min(w_lowlink));
            } else if *on_stack.get(&w).unwrap_or(&false) {
                // Successor w is in stack S and hence in the current SCC
                let w_index = *indices.get(&w).unwrap();
                let v_lowlink = *lowlinks.get(&v).unwrap();
                lowlinks.insert(v, v_lowlink.min(w_index));
            }
        }

        // If v is a root node, pop the stack and print an SCC
        if lowlinks[&v] == indices[&v] {
            let mut component = Vec::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack.insert(w, false);
                component.push(w);
                if w == v {
                    break;
                }
            }
            // Only include components with more than 1 node (actual cycles)
            if component.len() > 1 {
                components.push(component);
            }
        }
    }

    /// Get detailed cycle information for diagnostics using three-color DFS
    pub fn find_cycle_paths(&self) -> Result<Vec<Vec<String>>> {
        let mut visited = HashMap::new();
        let mut path = Vec::new();
        let mut cycles = Vec::new();

        // Initialize all nodes as white (unvisited)
        for node_index in self.graph.node_indices() {
            visited.insert(node_index, Color::White);
        }

        // Start DFS from each unvisited node
        for node_index in self.graph.node_indices() {
            if visited[&node_index] == Color::White {
                self.dfs_find_cycles(node_index, &mut visited, &mut path, &mut cycles);
            }
        }

        Ok(cycles)
    }

    /// Three-color DFS to find cycles with exact paths
    fn dfs_find_cycles(
        &self,
        node: NodeIndex,
        visited: &mut HashMap<NodeIndex, Color>,
        path: &mut Vec<NodeIndex>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node, Color::Gray);
        path.push(node);

        // Visit all outgoing neighbors
        for neighbor in self
            .graph
            .neighbors_directed(node, petgraph::Direction::Outgoing)
        {
            match visited[&neighbor] {
                Color::White => {
                    // Unvisited node, continue DFS
                    self.dfs_find_cycles(neighbor, visited, path, cycles);
                }
                Color::Gray => {
                    // Back edge found - we have a cycle!
                    if let Some(cycle_start) = path.iter().position(|&n| n == neighbor) {
                        let cycle_path: Vec<String> = path[cycle_start..]
                            .iter()
                            .map(|&idx| self.graph[idx].name.clone())
                            .collect();
                        cycles.push(cycle_path);
                    }
                }
                Color::Black => {
                    // Already processed, skip
                }
            }
        }

        path.pop();
        visited.insert(node, Color::Black);
    }

    /// Classify circular dependencies by type for resolution strategy
    pub fn classify_circular_dependencies(&self) -> CircularDependencyAnalysis {
        let sccs = self.find_strongly_connected_components();
        let _cycle_paths = self.find_cycle_paths().unwrap_or_default();

        let mut resolvable_cycles = Vec::new();
        let mut unresolvable_cycles = Vec::new();
        let mut largest_cycle_size = 0;

        for scc in &sccs {
            if scc.len() > largest_cycle_size {
                largest_cycle_size = scc.len();
            }

            let module_names: Vec<String> = scc
                .iter()
                .map(|&idx| self.graph[idx].name.clone())
                .collect();

            // Create import chain from the modules in the SCC
            let import_chain = self.build_import_chain_for_scc(scc);

            // Classify the cycle type based on heuristics
            let cycle_type = self.classify_cycle_type(&module_names);

            let suggested_resolution =
                self.suggest_resolution_for_cycle(&cycle_type, &module_names);

            let group = CircularDependencyGroup {
                modules: module_names,
                cycle_type: cycle_type.clone(),
                import_chain,
                suggested_resolution,
            };

            match cycle_type {
                CircularDependencyType::ModuleConstants => {
                    unresolvable_cycles.push(group);
                }
                _ => {
                    resolvable_cycles.push(group);
                }
            }
        }

        CircularDependencyAnalysis {
            resolvable_cycles,
            unresolvable_cycles,
            total_cycles_detected: sccs.len(),
            largest_cycle_size,
        }
    }

    /// Build import chain for a strongly connected component
    fn build_import_chain_for_scc(&self, scc: &[NodeIndex]) -> Vec<ImportEdge> {
        let mut import_chain = Vec::new();

        for &node_idx in scc {
            let from_module = &self.graph[node_idx].name;

            // Find outgoing edges to other nodes in the same SCC
            for neighbor in self
                .graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            {
                if scc.contains(&neighbor) {
                    let to_module = &self.graph[neighbor].name;

                    import_chain.push(ImportEdge {
                        from_module: from_module.clone(),
                        to_module: to_module.clone(),
                        import_type: ImportType::Direct, // Simplified - could be enhanced
                        line_number: None,               // Would need AST analysis to determine
                    });
                }
            }
        }

        import_chain
    }

    /// Classify the type of circular dependency based on module names and patterns
    fn classify_cycle_type(&self, _module_names: &[String]) -> CircularDependencyType {
        // Simple heuristic-based classification
        // In a real implementation, this would analyze the actual import statements
        // and module content to determine the cycle type

        // Check for specific patterns in module names
        for module_name in _module_names {
            // Modules with "constants" are unresolvable
            if module_name.contains("constants") {
                return CircularDependencyType::ModuleConstants;
            }
            // Modules with "class" suggest class-level dependencies
            if module_name.contains("class") {
                return CircularDependencyType::ClassLevel;
            }
            // Modules with "import" or "loader" suggest import-time dependencies
            if module_name.contains("import") || module_name.contains("loader") {
                return CircularDependencyType::ImportTime;
            }
        }

        // Default to function-level (most resolvable)
        CircularDependencyType::FunctionLevel
    }

    /// Suggest resolution strategy based on cycle type
    fn suggest_resolution_for_cycle(
        &self,
        cycle_type: &CircularDependencyType,
        module_names: &[String],
    ) -> ResolutionStrategy {
        match cycle_type {
            CircularDependencyType::FunctionLevel => {
                ResolutionStrategy::LazyImport {
                    modules: module_names.to_vec(),
                }
            }
            CircularDependencyType::ClassLevel => {
                ResolutionStrategy::FunctionScopedImport {
                    import_statements: module_names
                        .iter()
                        .map(|name| format!("# Move 'from {} import ...' inside functions", name))
                        .collect(),
                }
            }
            CircularDependencyType::ImportTime => {
                ResolutionStrategy::ModuleSplit {
                    suggestions: vec![
                        "Consider extracting common dependencies to a separate module".to_string(),
                        "Use dependency injection to break circular references".to_string(),
                    ],
                }
            }
            CircularDependencyType::ModuleConstants => {
                ResolutionStrategy::Unresolvable {
                    reason: "Module-level constant dependencies create temporal paradox - cannot be resolved through bundling".to_string(),
                }
            }
        }
    }
}

/// Color enum for three-color DFS algorithm
#[derive(Debug, Clone, Copy, PartialEq)]
enum Color {
    White, // Unvisited
    Gray,  // Currently being processed
    Black, // Completely processed
}
