use std::hash::BuildHasherDefault;

use anyhow::Result;
use indexmap::{IndexMap, IndexSet};
use log::error;
use ruff_python_ast::ModModule;
use rustc_hash::{FxHashSet, FxHasher};

/// Type alias for IndexMap with FxHasher for better performance
type FxIndexMap<K, V> = IndexMap<K, V, BuildHasherDefault<FxHasher>>;
/// Type alias for IndexSet with FxHasher for better performance
type FxIndexSet<T> = IndexSet<T, BuildHasherDefault<FxHasher>>;

/// Handles symbol-level circular dependency analysis and resolution
#[derive(Debug, Default)]
pub struct SymbolDependencyGraph {
    /// Map from (module, symbol) to list of (module, symbol) dependencies
    pub dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
    /// Track which symbols are defined in which modules
    pub symbol_definitions: FxIndexSet<(String, String)>,
    /// Module-level dependencies (used at definition time, not inside function bodies)
    pub module_level_dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
    /// Topologically sorted symbols for circular modules (computed after analysis)
    pub sorted_symbols: Vec<(String, String)>,
}

impl SymbolDependencyGraph {
    pub fn new() -> Self {
        Self {
            dependencies: FxIndexMap::default(),
            symbol_definitions: FxIndexSet::default(),
            module_level_dependencies: FxIndexMap::default(),
            sorted_symbols: Vec::new(),
        }
    }

    /// Get symbols for a specific module in dependency order
    pub fn get_module_symbols_ordered(&self, module_name: &str) -> Vec<String> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
            visit::EdgeRef,
        };
        use rustc_hash::FxHashMap;

        // Build a directed graph of symbol dependencies ONLY for this module
        let mut graph = DiGraph::new();
        let mut node_map: FxHashMap<String, NodeIndex> = FxHashMap::default();
        let mut symbols_in_module = Vec::new();

        // Add nodes for all symbols in this specific module
        for (module, symbol) in &self.symbol_definitions {
            if module == module_name {
                let node = graph.add_node(symbol.clone());
                node_map.insert(symbol.clone(), node);
                symbols_in_module.push(symbol.clone());
            }
        }

        // Add edges for dependencies within this module
        for ((module, symbol), deps) in &self.module_level_dependencies {
            if module == module_name
                && let Some(&from_node) = node_map.get(symbol)
            {
                for (dep_module, dep_symbol) in deps {
                    // Only add edges for dependencies within the same module
                    if dep_module == module_name
                        && let Some(&to_node) = node_map.get(dep_symbol)
                    {
                        // Edge from dependency to dependent
                        graph.add_edge(to_node, from_node, ());
                    }
                }
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted_nodes) => {
                // Return symbols in topological order (dependencies first)
                sorted_nodes
                    .into_iter()
                    .map(|node_idx| graph[node_idx].clone())
                    .collect()
            }
            Err(cycle) => {
                // If topological sort fails, there's a symbol-level circular dependency
                // This is a fatal error - we cannot generate correct code
                let cycle_info = cycle.node_id();
                let symbol = &graph[cycle_info];
                log::error!(
                    "Fatal: Circular dependency detected in module '{module_name}' involving \
                     symbol '{symbol}'"
                );

                // Find all symbols involved in the cycle
                let mut cycle_symbols = vec![symbol.clone()];
                let current = cycle_info;

                // Walk through edges to find the cycle
                for edge in graph.edges(current) {
                    let target = edge.target();
                    if target != cycle_info {
                        cycle_symbols.push(graph[target].clone());
                    } else {
                        break;
                    }
                }

                panic!(
                    "Cannot bundle due to circular symbol dependency in module '{module_name}': \
                     {cycle_symbols:?}"
                );
            }
        }
    }

    /// Perform topological sort on symbols within circular modules
    /// Stores symbols in reverse topological order (dependencies first)
    pub fn topological_sort_symbols(
        &mut self,
        circular_modules: &FxIndexSet<String>,
    ) -> Result<()> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
            visit::EdgeRef,
        };
        use rustc_hash::FxHashMap;

        // Build a directed graph of symbol dependencies
        let mut graph = DiGraph::new();
        let mut node_map: FxHashMap<(String, String), NodeIndex> = FxHashMap::default();

        // Add nodes for all symbols in circular modules
        for module_symbol in &self.symbol_definitions {
            if circular_modules.contains(&module_symbol.0) {
                let node = graph.add_node(module_symbol.clone());
                node_map.insert(module_symbol.clone(), node);
            }
        }

        // Add edges for module-level dependencies only
        // (dependencies within function bodies are not relevant for sorting)
        for (symbol, deps) in &self.module_level_dependencies {
            if let Some(&from_node) = node_map.get(symbol) {
                for dep in deps {
                    if let Some(&to_node) = node_map.get(dep) {
                        graph.add_edge(from_node, to_node, ());
                    }
                }
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted) => {
                // Store in reverse order (dependencies first)
                self.sorted_symbols = sorted
                    .into_iter()
                    .rev()
                    .map(|idx| graph[idx].clone())
                    .collect();
                Ok(())
            }
            Err(cycle_info) => {
                // Get the module name from the cycle
                let module_name = &graph[cycle_info.node_id()].0;

                // Find a cycle for better error reporting
                let cycle_start = cycle_info.node_id();
                let mut cycle_symbols = vec![graph[cycle_start].clone()];

                // Try to reconstruct the cycle
                let mut current = cycle_start;
                let mut visited = FxHashSet::default();
                visited.insert(current);

                // Follow edges to find the cycle
                'outer: loop {
                    let mut found_next = false;
                    for edge in graph.edges(current) {
                        let target = edge.target();
                        if target == cycle_start {
                            // Found complete cycle
                            break 'outer;
                        }
                        if !visited.contains(&target) {
                            visited.insert(target);
                            cycle_symbols.push(graph[target].clone());
                            current = target;
                            found_next = true;
                            break;
                        }
                    }
                    if !found_next {
                        // No unvisited neighbors, might be a more complex cycle
                        break;
                    }
                }

                error!("Cannot bundle due to circular symbol dependency in module '{module_name}'");
                error!("Circular dependency involves symbols: {cycle_symbols:?}");
                error!("This is an unresolvable circular dependency at the symbol level.");
                error!("Consider refactoring to break the circular dependency:");
                error!("  - Move shared base classes to a separate module");
                error!("  - Use protocols or abstract base classes");
                error!("  - Restructure class inheritance hierarchy");

                anyhow::bail!(
                    "Unresolvable circular dependency detected in module '{}'. Symbols involved: \
                     {:?}",
                    module_name,
                    cycle_symbols
                );
            }
        }
    }

    /// Collect symbol dependencies for a module
    pub fn collect_dependencies(
        &mut self,
        module_name: &str,
        _ast: &ModModule,
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        // Only analyze modules that are part of circular dependencies
        if !circular_modules.contains(module_name) {
            return;
        }

        log::debug!("Building symbol dependency graph for circular module: {module_name}");

        // Get the module from the graph
        if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
            // For each item in the module
            for item_data in module_dep_graph.items.values() {
                match &item_data.item_type {
                    crate::cribo_graph::ItemType::FunctionDef { name } => {
                        self.analyze_function_dependencies(
                            module_name,
                            name,
                            item_data,
                            module_dep_graph,
                            graph,
                            circular_modules,
                        );
                    }
                    crate::cribo_graph::ItemType::ClassDef { name } => {
                        self.analyze_class_dependencies(
                            module_name,
                            name,
                            item_data,
                            module_dep_graph,
                            graph,
                            circular_modules,
                        );
                    }
                    crate::cribo_graph::ItemType::Assignment { targets } => {
                        for target in targets {
                            self.analyze_assignment_dependencies(
                                module_name,
                                target,
                                item_data,
                                module_dep_graph,
                                graph,
                                circular_modules,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Analyze dependencies for a class definition
    fn analyze_class_dependencies(
        &mut self,
        module_name: &str,
        class_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        let key = (module_name.to_string(), class_name.to_string());
        let mut all_dependencies = Vec::new();
        let mut module_level_deps = Vec::new();

        // For classes, check both immediate reads (base classes) and eventual reads (methods)
        for var in &item_data.read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
            {
                let dep = (dep_module, var.clone());
                all_dependencies.push(dep.clone());
                // Base classes need to exist at definition time, even within same module
                module_level_deps.push(dep);
            }
        }

        self.dependencies.insert(key.clone(), all_dependencies);
        self.module_level_dependencies
            .insert(key.clone(), module_level_deps);
        self.symbol_definitions.insert(key);
    }

    /// Analyze dependencies for a function definition
    fn analyze_function_dependencies(
        &mut self,
        module_name: &str,
        function_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        let key = (module_name.to_string(), function_name.to_string());
        let mut all_dependencies = Vec::new();

        // For functions, we only care about reads that happen during execution
        for var in &item_data.read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
            {
                let dep = (dep_module, var.clone());
                all_dependencies.push(dep);
            }
        }

        self.dependencies.insert(key.clone(), all_dependencies);
        // Functions have no module-level dependencies (they're executed later)
        self.module_level_dependencies
            .insert(key.clone(), Vec::new());
        self.symbol_definitions.insert(key);
    }

    /// Analyze dependencies for an assignment
    fn analyze_assignment_dependencies(
        &mut self,
        module_name: &str,
        target_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        _module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        let key = (module_name.to_string(), target_name.to_string());
        let mut all_dependencies = Vec::new();
        let mut module_level_deps = Vec::new();

        // For assignments, all reads are module-level dependencies
        for var in &item_data.read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
            {
                let dep = (dep_module, var.clone());
                all_dependencies.push(dep.clone());
                module_level_deps.push(dep);
            }
        }

        self.dependencies.insert(key.clone(), all_dependencies);
        self.module_level_dependencies
            .insert(key.clone(), module_level_deps);
        self.symbol_definitions.insert(key);
    }

    /// Find which module defines a symbol
    fn find_symbol_module(
        &self,
        symbol: &str,
        _current_module: &str,
        graph: &crate::cribo_graph::CriboGraph,
    ) -> Option<String> {
        // Search through all modules in the graph
        for module_graph in graph.modules.values() {
            for item_data in module_graph.items.values() {
                match &item_data.item_type {
                    crate::cribo_graph::ItemType::FunctionDef { name }
                    | crate::cribo_graph::ItemType::ClassDef { name } => {
                        if name == symbol {
                            return Some(module_graph.module_name.clone());
                        }
                    }
                    crate::cribo_graph::ItemType::Assignment { targets } => {
                        if targets.contains(&symbol.to_string()) {
                            return Some(module_graph.module_name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Check if we should sort symbols for the given modules
    pub fn should_sort_symbols(&self, circular_modules: &FxIndexSet<String>) -> bool {
        // Check if any circular module has symbol definitions
        circular_modules
            .iter()
            .any(|module| self.symbol_definitions.iter().any(|(m, _)| m == module))
    }

    /// Get sorted symbols for circular modules
    pub fn get_sorted_symbols(&self) -> &[(String, String)] {
        &self.sorted_symbols
    }

    /// Add a hard dependency (for tracking only, not used in sorting)
    pub fn add_hard_dependency(&mut self, from: (String, String), to: (String, String)) {
        self.dependencies.entry(from).or_default().push(to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topological_sort_simple() {
        let mut graph = SymbolDependencyGraph::default();

        // Add some test symbols
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "ClassA".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "ClassB".to_string()));

        // ClassB depends on ClassA
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "ClassB".to_string()),
            vec![("mod1".to_string(), "ClassA".to_string())],
        );

        let mut circular_modules = FxIndexSet::default();
        circular_modules.insert("mod1".to_string());

        assert!(graph.topological_sort_symbols(&circular_modules).is_ok());

        // ClassA should come before ClassB (dependencies first)
        assert_eq!(graph.sorted_symbols[0].1, "ClassA");
        assert_eq!(graph.sorted_symbols[1].1, "ClassB");
    }
}
