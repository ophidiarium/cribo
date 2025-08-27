#![allow(clippy::excessive_nesting)]

use anyhow::Result;

use crate::types::{FxIndexMap, FxIndexSet};

/// Handles symbol-level circular dependency analysis and resolution
#[derive(Debug, Default, Clone)]
pub struct SymbolDependencyGraph {
    /// Track which symbols are defined in which modules
    pub symbol_definitions: FxIndexSet<(String, String)>,
    /// Module-level dependencies (used at definition time, not inside function bodies)
    pub module_level_dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
    /// Topologically sorted symbols for circular modules (computed after analysis)
    pub sorted_symbols: Vec<(String, String)>,
}

impl SymbolDependencyGraph {
    /// Find all symbols in the strongly connected component containing the given node
    /// Uses Tarjan's SCC algorithm for robust cycle detection
    fn find_cycle_symbols_with_scc(
        graph: &petgraph::Graph<String, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<String> {
        Self::find_cycle_symbols_generic(graph, cycle_node)
    }

    /// Find all symbols in the strongly connected component containing the given node
    /// For (module, symbol) pairs
    fn find_cycle_symbols_with_scc_pairs(
        graph: &petgraph::Graph<(String, String), ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<(String, String)> {
        Self::find_cycle_symbols_generic(graph, cycle_node)
    }

    /// Generic implementation of Tarjan's strongly connected components algorithm
    /// Works with any graph node type that implements Clone
    fn find_cycle_symbols_generic<T>(
        graph: &petgraph::Graph<T, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<T>
    where
        T: Clone,
    {
        use petgraph::visit::EdgeRef;
        use rustc_hash::FxHashMap;

        /// State for Tarjan's SCC algorithm
        struct TarjanState {
            index_counter: usize,
            stack: Vec<petgraph::graph::NodeIndex>,
            indices: FxHashMap<petgraph::graph::NodeIndex, usize>,
            lowlinks: FxHashMap<petgraph::graph::NodeIndex, usize>,
            on_stack: FxHashMap<petgraph::graph::NodeIndex, bool>,
            components: Vec<Vec<petgraph::graph::NodeIndex>>,
        }

        impl TarjanState {
            fn new() -> Self {
                Self {
                    index_counter: 0,
                    stack: Vec::new(),
                    indices: FxHashMap::default(),
                    lowlinks: FxHashMap::default(),
                    on_stack: FxHashMap::default(),
                    components: Vec::new(),
                }
            }
        }

        fn tarjan_strongconnect<T>(
            graph: &petgraph::Graph<T, ()>,
            v: petgraph::graph::NodeIndex,
            state: &mut TarjanState,
        ) {
            state.indices.insert(v, state.index_counter);
            state.lowlinks.insert(v, state.index_counter);
            state.index_counter += 1;
            state.stack.push(v);
            state.on_stack.insert(v, true);

            for edge in graph.edges(v) {
                let w = edge.target();
                if !state.indices.contains_key(&w) {
                    tarjan_strongconnect(graph, w, state);
                    let w_lowlink = state.lowlinks[&w];
                    let v_lowlink = state.lowlinks[&v];
                    state.lowlinks.insert(v, v_lowlink.min(w_lowlink));
                } else if state.on_stack.get(&w).copied().unwrap_or(false) {
                    let w_index = state.indices[&w];
                    let v_lowlink = state.lowlinks[&v];
                    state.lowlinks.insert(v, v_lowlink.min(w_index));
                }
            }

            if state.lowlinks[&v] == state.indices[&v] {
                let mut component = Vec::new();
                while let Some(w) = state.stack.pop() {
                    state.on_stack.insert(w, false);
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                // Only store components with more than one node (actual cycles)
                if component.len() > 1 {
                    state.components.push(component);
                }
            }
        }

        let mut state = TarjanState::new();

        // Run Tarjan's algorithm on all unvisited nodes
        for node_index in graph.node_indices() {
            if !state.indices.contains_key(&node_index) {
                tarjan_strongconnect(graph, node_index, &mut state);
            }
        }

        // Find the SCC containing our cycle node
        for component in state.components {
            if component.contains(&cycle_node) {
                // Return all symbols in this SCC
                return component
                    .into_iter()
                    .map(|idx| graph[idx].clone())
                    .collect();
            }
        }

        // If no SCC found (shouldn't happen for actual cycles), fall back to single symbol
        vec![graph[cycle_node].clone()]
    }

    /// Get symbols for a specific module in dependency order
    pub fn get_module_symbols_ordered(&self, module_name: &str) -> Vec<String> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
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

                // Find all symbols involved in the cycle using SCC detection
                let cycle_symbols = Self::find_cycle_symbols_with_scc(&graph, cycle_info);

                panic!(
                    "Cannot bundle due to circular symbol dependency in module '{module_name}': \
                     {cycle_symbols:?}"
                );
            }
        }
    }

    /// Perform topological sort on symbols within circular modules
    /// Stores symbols in topological order (dependencies first)
    pub fn topological_sort_symbols(
        &mut self,
        circular_modules: &FxIndexSet<String>,
    ) -> Result<()> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
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
                log::debug!("Added node: {}.{}", module_symbol.0, module_symbol.1);
            }
        }

        // Add edges for dependencies
        for (module_symbol, deps) in &self.module_level_dependencies {
            if let Some(&from_node) = node_map.get(module_symbol) {
                for dep in deps {
                    if let Some(&to_node) = node_map.get(dep) {
                        // Edge from dependency to dependent (correct direction for topological
                        // sort)
                        log::debug!(
                            "Adding edge: {}.{} -> {}.{} (dependency -> dependent)",
                            dep.0,
                            dep.1,
                            module_symbol.0,
                            module_symbol.1
                        );
                        graph.add_edge(to_node, from_node, ());
                    }
                }
            }
        }

        // Perform topological sort
        match toposort(&graph, None) {
            Ok(sorted_nodes) => {
                // Store in topological order (dependencies first)
                self.sorted_symbols.clear();
                for node_idx in sorted_nodes {
                    self.sorted_symbols.push(graph[node_idx].clone());
                }
                Ok(())
            }
            Err(cycle) => {
                // If topological sort fails, there's a symbol-level circular dependency
                // This is a fatal error - we cannot generate correct code
                let cycle_info = cycle.node_id();
                let module_symbol = &graph[cycle_info];
                log::error!(
                    "Fatal: Circular dependency detected involving symbol '{}.{}'",
                    module_symbol.0,
                    module_symbol.1
                );

                // Find all symbols involved in the cycle using SCC detection
                let cycle_symbols = Self::find_cycle_symbols_with_scc_pairs(&graph, cycle_info);

                Err(anyhow::anyhow!(
                    "Cannot bundle due to circular symbol dependency: {:?}",
                    cycle_symbols
                        .iter()
                        .map(|(m, s)| format!("{m}.{s}"))
                        .collect::<Vec<_>>()
                ))
            }
        }
    }
}

/// Generate pre-declarations for circular dependencies

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

    #[test]
    fn test_complex_cycle_detection() {
        let mut graph = SymbolDependencyGraph::default();

        // Create a complex cycle: A -> B -> C -> D -> A
        // Plus an additional path: A -> C (creating a diamond)
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "A".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "B".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "C".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "D".to_string()));

        // A depends on B and C (diamond shape)
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "A".to_string()),
            vec![
                ("mod1".to_string(), "B".to_string()),
                ("mod1".to_string(), "C".to_string()),
            ],
        );

        // B depends on C
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "B".to_string()),
            vec![("mod1".to_string(), "C".to_string())],
        );

        // C depends on D
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "C".to_string()),
            vec![("mod1".to_string(), "D".to_string())],
        );

        // D depends on A (creates the cycle)
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "D".to_string()),
            vec![("mod1".to_string(), "A".to_string())],
        );

        let mut circular_modules = FxIndexSet::default();
        circular_modules.insert("mod1".to_string());

        // This should fail due to circular dependency
        let mut local_graph = graph.clone();
        let result = local_graph.topological_sort_symbols(&circular_modules);

        // The current implementation might miss some symbols in the cycle
        // With our improved implementation, it should detect all 4 symbols in the cycle
        assert!(result.is_err(), "Expected error due to circular dependency");
    }

    #[test]
    fn test_cycle_detection_captures_all_symbols() {
        // Test that our SCC-based implementation captures all symbols in a complex cycle
        let mut graph = SymbolDependencyGraph::default();

        // Create a complex 4-node cycle
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "A".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "B".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "C".to_string()));
        graph
            .symbol_definitions
            .insert(("mod1".to_string(), "D".to_string()));

        // A -> B -> C -> D -> A
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "A".to_string()),
            vec![("mod1".to_string(), "B".to_string())],
        );
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "B".to_string()),
            vec![("mod1".to_string(), "C".to_string())],
        );
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "C".to_string()),
            vec![("mod1".to_string(), "D".to_string())],
        );
        graph.module_level_dependencies.insert(
            ("mod1".to_string(), "D".to_string()),
            vec![("mod1".to_string(), "A".to_string())],
        );

        let circular_modules = {
            let mut set = FxIndexSet::default();
            set.insert("mod1".to_string());
            set
        };

        // Test that topological sort fails due to circular dependency
        let mut local_graph = graph.clone();
        let result = local_graph.topological_sort_symbols(&circular_modules);

        assert!(result.is_err(), "Expected error due to circular dependency");

        // Test the SCC detection directly with a simulated graph
        use petgraph::Graph;
        let mut test_graph = Graph::new();
        let node_a = test_graph.add_node(("mod1".to_string(), "A".to_string()));
        let node_b = test_graph.add_node(("mod1".to_string(), "B".to_string()));
        let node_c = test_graph.add_node(("mod1".to_string(), "C".to_string()));
        let node_d = test_graph.add_node(("mod1".to_string(), "D".to_string()));

        // Add cycle edges: A -> B -> C -> D -> A
        test_graph.add_edge(node_a, node_b, ());
        test_graph.add_edge(node_b, node_c, ());
        test_graph.add_edge(node_c, node_d, ());
        test_graph.add_edge(node_d, node_a, ());

        // Test our SCC detection method
        let cycle_symbols =
            SymbolDependencyGraph::find_cycle_symbols_with_scc_pairs(&test_graph, node_a);

        // All 4 symbols should be detected in the cycle
        assert_eq!(
            cycle_symbols.len(),
            4,
            "All 4 symbols should be detected in cycle"
        );

        // Convert to set of symbol names for easier verification
        let symbol_names: indexmap::IndexSet<_> = cycle_symbols
            .iter()
            .map(|(_, name)| name.as_str())
            .collect();
        assert!(symbol_names.contains("A"));
        assert!(symbol_names.contains("B"));
        assert!(symbol_names.contains("C"));
        assert!(symbol_names.contains("D"));
    }
}
