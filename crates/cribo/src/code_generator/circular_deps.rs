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
    /// Uses petgraph SCC detection for robust cycle detection
    fn find_cycle_symbols_with_scc(
        graph: &petgraph::Graph<String, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<String> {
        Self::find_cycle_symbols_generic(graph, cycle_node)
    }

    /// Locate the SCC for a node using petgraph's SCC utilities
    /// Works with any graph node type that implements Clone
    fn find_cycle_symbols_generic<T>(
        graph: &petgraph::Graph<T, ()>,
        cycle_node: petgraph::graph::NodeIndex,
    ) -> Vec<T>
    where
        T: Clone,
    {
        use petgraph::algo::tarjan_scc;

        let components = tarjan_scc(graph);

        // Include self-loops (single-node SCC with self-edge) as cycles
        if let Some(component) = components.into_iter().find(|c| {
            c.contains(&cycle_node) && (c.len() > 1 || graph.contains_edge(cycle_node, cycle_node))
        }) {
            return component
                .into_iter()
                .map(|idx| graph[idx].clone())
                .collect();
        }

        // If no SCC found containing the node (unexpected), return just that symbol
        vec![graph[cycle_node].clone()]
    }

    /// Get symbols for a specific module in dependency order
    pub fn get_module_symbols_ordered(&self, module_name: &str) -> Vec<String> {
        use petgraph::{
            algo::toposort,
            graph::{DiGraph, NodeIndex},
        };
        // Build a directed graph of symbol dependencies ONLY for this module
        let mut graph = DiGraph::new();
        let mut node_map: FxIndexMap<String, NodeIndex> = FxIndexMap::default();
        let mut symbols_in_module = Vec::new();

        // Add nodes for all symbols in this specific module
        for (module, symbol) in &self.symbol_definitions {
            if module == module_name {
                let node = graph.add_node(symbol.clone());
                node_map.insert(symbol.clone(), node);
                symbols_in_module.push(symbol.clone());
            }
        }

        // Add edges for dependencies within this module (flattened with early continues)
        for ((module, symbol), deps) in &self.module_level_dependencies {
            if module != module_name {
                continue;
            }
            let Some(&from_node) = node_map.get(symbol) else {
                continue;
            };
            for (dep_module, dep_symbol) in deps {
                if dep_module != module_name {
                    continue;
                }
                let Some(&to_node) = node_map.get(dep_symbol) else {
                    continue;
                };
                // Edge from dependency to dependent
                graph.add_edge(to_node, from_node, ());
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
