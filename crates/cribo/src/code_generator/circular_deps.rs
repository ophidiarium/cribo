#![allow(clippy::excessive_nesting)]

use crate::types::{FxIndexMap, FxIndexSet};

/// Handles symbol-level circular dependency analysis and resolution
#[derive(Debug, Default, Clone)]
pub struct SymbolDependencyGraph {
    /// Track which symbols are defined in which modules
    pub symbol_definitions: FxIndexSet<(String, String)>,
    /// Module-level dependencies (used at definition time, not inside function bodies)
    pub module_level_dependencies: FxIndexMap<(String, String), Vec<(String, String)>>,
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
}
