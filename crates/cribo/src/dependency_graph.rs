use anyhow::{Result, anyhow};
use indexmap::{IndexMap, IndexSet};
use log::debug;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
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

/// State for Tarjan's strongly connected components algorithm
struct TarjanState {
    index_counter: usize,
    stack: Vec<NodeIndex>,
    indices: IndexMap<NodeIndex, usize>,
    lowlinks: IndexMap<NodeIndex, usize>,
    on_stack: IndexMap<NodeIndex, bool>,
    components: Vec<Vec<NodeIndex>>,
}

#[derive(Debug)]
pub struct DependencyGraph {
    graph: DiGraph<ModuleNode, ()>,
    node_indices: IndexMap<String, NodeIndex>,
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
            node_indices: IndexMap::new(),
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
            let existing_index = self
                .node_indices
                .shift_remove(&old_key)
                .expect("old key should exist in node_indices");
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
        debug!(
            "Module {} has {} incoming edges (dependencies)",
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
        let mut index_mapping = IndexMap::new();

        // Add all reachable nodes
        self.add_reachable_nodes_to_filtered_graph(
            &visited,
            &mut filtered_graph,
            &mut index_mapping,
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
        index_mapping: &mut IndexMap<NodeIndex, NodeIndex>,
    ) {
        for &old_index in visited {
            let module = self.graph[old_index].clone();
            let new_index = filtered_graph.add_module(module);
            index_mapping.insert(old_index, new_index);
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
        debug!(
            "Found dependency: {} -> {}",
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
        let mut state = TarjanState {
            index_counter: 0,
            stack: Vec::new(),
            indices: IndexMap::new(),
            lowlinks: IndexMap::new(),
            on_stack: IndexMap::new(),
            components: Vec::new(),
        };

        for node_index in self.graph.node_indices() {
            if !state.indices.contains_key(&node_index) {
                self.tarjan_strongconnect(node_index, &mut state);
            }
        }

        state.components
    }

    fn pop_scc_component(
        &self,
        stack: &mut Vec<NodeIndex>,
        on_stack: &mut IndexMap<NodeIndex, bool>,
        v: NodeIndex,
    ) -> Vec<NodeIndex> {
        let mut component = Vec::new();
        while let Some(w) = stack.pop() {
            on_stack.insert(w, false);
            component.push(w);
            if w == v {
                break;
            }
        }
        component
    }

    fn tarjan_strongconnect(&self, v: NodeIndex, state: &mut TarjanState) {
        state.indices.insert(v, state.index_counter);
        state.lowlinks.insert(v, state.index_counter);
        state.index_counter += 1;
        state.stack.push(v);
        state.on_stack.insert(v, true);

        for w in self
            .graph
            .neighbors_directed(v, petgraph::Direction::Outgoing)
        {
            if !state.indices.contains_key(&w) {
                self.tarjan_strongconnect(w, state);
                let w_lowlink = *state.lowlinks.get(&w).expect("w should exist in lowlinks");
                let v_lowlink = *state.lowlinks.get(&v).expect("v should exist in lowlinks");
                state.lowlinks.insert(v, v_lowlink.min(w_lowlink));
            } else if *state.on_stack.get(&w).unwrap_or(&false) {
                let w_index = *state.indices.get(&w).expect("w should exist in indices");
                let v_lowlink = *state.lowlinks.get(&v).expect("v should exist in lowlinks");
                state.lowlinks.insert(v, v_lowlink.min(w_index));
            }
        }

        if state.lowlinks[&v] == state.indices[&v] {
            let component = self.pop_scc_component(&mut state.stack, &mut state.on_stack, v);
            if component.len() > 1 {
                state.components.push(component);
            }
        }
    }

    pub fn find_cycle_paths(&self) -> Result<Vec<Vec<String>>> {
        let mut visited = IndexMap::new();
        let mut path = Vec::new();
        let mut cycles = Vec::new();

        for node_index in self.graph.node_indices() {
            visited.insert(node_index, Color::White);
        }

        let mut ctx = DfsCycleContext {
            visited: &mut visited,
            path: &mut path,
            cycles: &mut cycles,
        };

        for node_index in self.graph.node_indices() {
            if ctx.visited[&node_index] == Color::White {
                self.dfs_find_cycles_internal(node_index, &mut ctx);
            }
        }

        Ok(ctx.cycles.clone())
    }

    fn dfs_find_cycles_internal(&self, node: NodeIndex, ctx: &mut DfsCycleContext) {
        ctx.visited.insert(node, Color::Gray);
        ctx.path.push(node);

        for neighbor in self
            .graph
            .neighbors_directed(node, petgraph::Direction::Outgoing)
        {
            match ctx.visited[&neighbor] {
                Color::White => {
                    self.dfs_find_cycles_internal(neighbor, ctx);
                }
                Color::Gray => {
                    self.handle_cycle_detection(neighbor, ctx);
                }
                Color::Black => {}
            }
        }

        ctx.path.pop();
        ctx.visited.insert(node, Color::Black);
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

            let scc_neighbors: Vec<_> = self
                .graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .filter(|neighbor| scc.contains(neighbor))
                .collect();

            for neighbor in scc_neighbors {
                let to_module = &self.graph[neighbor].name;
                import_chain.push(ImportEdge {
                    from_module: from_module.clone(),
                    to_module: to_module.clone(),
                    import_type: ImportType::Direct, // Simplified - could be enhanced
                    line_number: None,               // Would need AST analysis to determine
                });
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
                        "Consider extracting common dependencies to a separate module".to_owned(),
                        "Use dependency injection to break circular references".to_owned(),
                    ],
                }
            }
            CircularDependencyType::ModuleConstants => {
                ResolutionStrategy::Unresolvable {
                    reason: "Module-level constant dependencies create temporal paradox - cannot be resolved through bundling".to_owned(),
                }
            }
        }
    }

    /// Helper method to handle cycle detection when encountering a gray node
    fn handle_cycle_detection(&self, neighbor: NodeIndex, ctx: &mut DfsCycleContext) {
        if let Some(cycle_start) = ctx.path.iter().position(|&n| n == neighbor) {
            let cycle_path: Vec<String> = ctx.path[cycle_start..]
                .iter()
                .map(|&idx| self.graph[idx].name.clone())
                .collect();
            ctx.cycles.push(cycle_path);
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

struct DfsCycleContext<'a> {
    visited: &'a mut IndexMap<NodeIndex, Color>,
    path: &'a mut Vec<NodeIndex>,
    cycles: &'a mut Vec<Vec<String>>,
}
