#![allow(clippy::excessive_nesting)]

use anyhow::Result;
use ruff_python_ast::{ModModule, Stmt};

use crate::{
    ast_builder::{expressions, statements},
    types::{FxIndexMap, FxIndexSet},
};

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
                    if target == cycle_info {
                        break;
                    }
                    cycle_symbols.push(graph[target].clone());
                }

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

                // Find all symbols involved in the cycle
                let mut cycle_symbols = vec![module_symbol.clone()];
                let current = cycle_info;

                // Walk through edges to find the cycle
                for edge in graph.edges(current) {
                    let target = edge.target();
                    if target == cycle_info {
                        break;
                    }
                    cycle_symbols.push(graph[target].clone());
                }

                panic!(
                    "Cannot bundle due to circular symbol dependency: {:?}",
                    cycle_symbols
                        .iter()
                        .map(|(m, s)| format!("{m}.{s}"))
                        .collect::<Vec<_>>()
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
                            graph,
                            circular_modules,
                        );
                    }
                    crate::cribo_graph::ItemType::ClassDef { name } => {
                        self.analyze_class_dependencies(
                            module_name,
                            name,
                            item_data,
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
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        let key = (module_name.to_string(), function_name.to_string());
        let mut all_dependencies = Vec::new();
        let mut module_level_deps = Vec::new();

        // Track what this function reads at module level (e.g., decorators, default args)
        for var in &item_data.read_vars {
            // Check if this variable is from a circular module
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
            {
                let dep = (dep_module.clone(), var.clone());
                all_dependencies.push(dep.clone());
                // Module-level reads need pre-declaration only if from different module
                if dep_module != module_name {
                    module_level_deps.push(dep);
                }
            }
        }

        // Also check eventual reads (inside the function body) - these don't need pre-declaration
        for var in &item_data.eventual_read_vars {
            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
                && dep_module != module_name
            {
                all_dependencies.push((dep_module, var.clone()));
                // Note: NOT added to module_level_deps since these are lazy
            }
        }

        self.dependencies.insert(key.clone(), all_dependencies);
        self.module_level_dependencies
            .insert(key.clone(), module_level_deps);
        self.symbol_definitions.insert(key);
    }

    /// Analyze dependencies for an assignment
    fn analyze_assignment_dependencies(
        &mut self,
        module_name: &str,
        target_name: &str,
        item_data: &crate::cribo_graph::ItemData,
        graph: &crate::cribo_graph::CriboGraph,
        circular_modules: &FxIndexSet<String>,
    ) {
        let key = (module_name.to_string(), target_name.to_string());
        let mut dependencies = Vec::new();

        // Assignments are evaluated immediately - all dependencies are module-level
        for var in &item_data.read_vars {
            // Skip self-references (e.g., initialize = initialize)
            if var == target_name
                && self.find_symbol_module(var, module_name, graph) == Some(module_name.to_string())
            {
                log::debug!("Skipping self-reference in assignment: {target_name} = {var}");
                continue;
            }

            if let Some(dep_module) = self.find_symbol_module(var, module_name, graph)
                && circular_modules.contains(&dep_module)
            {
                dependencies.push((dep_module, var.clone()));
            }
        }

        self.dependencies.insert(key.clone(), dependencies.clone());
        self.module_level_dependencies
            .insert(key.clone(), dependencies); // All assignment deps are module-level
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
}

/// Generate pre-declarations for circular dependencies
pub(super) fn generate_predeclarations(
    bundler: &mut super::Bundler,
    inlinable_modules: &[(String, ModModule, std::path::PathBuf, String)],
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    python_version: u8,
) -> Vec<Stmt> {
    let mut predeclarations = Vec::new();

    if bundler.circular_modules.is_empty() {
        return predeclarations;
    }

    log::debug!("Analyzing circular modules for necessary pre-declarations");

    // Collect all symbols that need pre-declaration based on actual forward references
    let mut symbols_needing_predeclaration = FxIndexSet::default();

    // First pass: Build a map of where each symbol will be defined in the final output
    let mut symbol_definition_order = FxIndexMap::default();
    let mut order_index = 0;

    for (module_name, _, _, _) in inlinable_modules {
        if let Some(module_renames) = symbol_renames.get(module_name) {
            for (original_name, _) in module_renames {
                symbol_definition_order
                    .insert((module_name.clone(), original_name.clone()), order_index);
                order_index += 1;
            }
        }
    }

    // Second pass: Find actual forward references using module-level dependencies
    for ((module, symbol), module_level_deps) in &bundler.symbol_dep_graph.module_level_dependencies
    {
        if bundler.circular_modules.contains(module) && !module_level_deps.is_empty() {
            // Create symbol_key once outside the inner loop since it doesn't change
            let symbol_key = (module.clone(), symbol.clone());

            // Check each module-level dependency
            for (dep_module, dep_symbol) in module_level_deps {
                if bundler.circular_modules.contains(dep_module) {
                    // Get the order indices
                    // Note: We clone here for the lookup, but this is only done for circular
                    // modules which are typically a small subset of all modules
                    let dep_key = (dep_module.clone(), dep_symbol.clone());

                    if let (Some(&sym_idx), Some(&dep_idx)) = (
                        symbol_definition_order.get(&symbol_key),
                        symbol_definition_order.get(&dep_key),
                    ) {
                        // Check if this creates a forward reference
                        if dep_idx > sym_idx {
                            log::debug!(
                                "Found forward reference: {module}.{symbol} (order {sym_idx}) \
                                 uses {dep_module}.{dep_symbol} (order {dep_idx}) at module level"
                            );
                            // Only clone when we actually need to insert
                            symbols_needing_predeclaration.insert(dep_key);
                        }
                    }
                }
            }
        }
    }

    // Now generate pre-declarations only for symbols that actually need them
    log::debug!("Symbols needing pre-declaration: {symbols_needing_predeclaration:?}");
    for (module_name, symbol_name) in symbols_needing_predeclaration {
        if let Some(module_renames) = symbol_renames.get(&module_name)
            && let Some(renamed_name) = module_renames.get(&symbol_name)
        {
            // Skip pre-declarations for built-in types
            // Built-in types are always available and pre-declaring them as None causes
            // issues
            if ruff_python_stdlib::builtins::is_python_builtin(renamed_name, python_version, false)
            {
                log::debug!(
                    "Skipping pre-declaration for built-in type: {renamed_name} (from \
                     {module_name}.{symbol_name})"
                );
                continue;
            }

            log::debug!(
                "Pre-declaring {renamed_name} (from {module_name}.{symbol_name}) due to forward \
                 reference"
            );
            let mut stmt = statements::simple_assign(renamed_name, expressions::none_literal());

            // Set custom node index for tracking
            if let Stmt::Assign(assign) = &mut stmt {
                assign.node_index = bundler.create_transformed_node(format!(
                    "Pre-declaration for circular dependency: {renamed_name}"
                ));
            }

            predeclarations.push(stmt);

            // Track the pre-declaration
            bundler
                .circular_predeclarations
                .entry(module_name.clone())
                .or_default()
                .insert(symbol_name.clone(), renamed_name.clone());
        }
    }

    predeclarations
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
