use anyhow::Result;
use indexmap::{IndexMap as FxIndexMap, IndexSet as FxIndexSet};
use log::error;
use ruff_python_ast::ModModule;
use rustc_hash::FxHashSet;

// use crate::semantic_analysis::ClassDependencyCollector;
use crate::transformation_context::TransformationContext;
use crate::visitors::ImportDiscoveryVisitor;

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
        _module_name: &str,
        _ast: &ModModule,
        _transform_context: &TransformationContext,
        _normalized_imports: &FxIndexMap<String, String>,
    ) {
        // Use ImportDiscoveryVisitor to find imports
        let _import_visitor = ImportDiscoveryVisitor::new();
        // Note: ImportDiscoveryVisitor uses the Visitor trait which expects &ModModule, not &mut
        // import_visitor.visit_module(ast);

        // TODO: Implement ClassDependencyCollector to analyze dependencies
        // For now, this is a placeholder implementation
        // The actual implementation would analyze class dependencies,
        // track symbol definitions, and determine module-level dependencies
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
