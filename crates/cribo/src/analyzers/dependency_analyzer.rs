//! Dependency analysis module
//!
//! This module provides functionality for analyzing dependencies between modules,
//! including circular dependency detection and topological sorting.

use log::{debug, warn};
use ruff_python_ast::ModModule;

use crate::{
    analyzers::types::{
        CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType,
        ResolutionStrategy,
    },
    cribo_graph::{CriboGraph as DependencyGraph, ItemType},
    types::{FxIndexMap, FxIndexSet},
};

/// Result of analyzing modules in a circular dependency cycle
#[derive(Debug)]
struct CycleAnalysisResult {
    /// Whether the modules contain only constants (no functions or classes)
    has_only_constants: bool,
    /// Whether any module contains class definitions
    has_class_definitions: bool,
    /// Whether there are module-level imports
    has_module_level_imports: bool,
    /// Whether imports are only used within functions
    imports_used_in_functions_only: bool,
}

/// Dependency analyzer for module and symbol dependencies
pub struct DependencyAnalyzer;

impl DependencyAnalyzer {
    /// Build a dependency map for a subset of modules
    fn build_dependency_map(
        module_names: &[String],
        module_names_set: &FxIndexSet<String>,
        graph: &DependencyGraph,
    ) -> FxIndexMap<String, FxIndexSet<String>> {
        let mut dependency_map: FxIndexMap<String, FxIndexSet<String>> = FxIndexMap::default();

        // Initialize all modules
        for module in module_names {
            dependency_map.insert(module.clone(), FxIndexSet::default());
        }

        // For each module, find its dependencies within the subset
        for module_name in module_names {
            if let Some(&module_id) = graph.module_names.get(module_name) {
                let dependencies = graph.get_dependencies(module_id);
                for dep_id in dependencies {
                    if let Some(dep_module) = graph.modules.get(&dep_id) {
                        let dep_name = &dep_module.module_name;
                        if module_names_set.contains(dep_name)
                            && dep_name != module_name
                            && let Some(deps) = dependency_map.get_mut(module_name)
                        {
                            deps.insert(dep_name.clone());
                        }
                    }
                }
            }
        }

        dependency_map
    }

    /// Sort wrapper modules by their dependencies
    pub fn sort_wrapper_modules_by_dependencies(
        wrapper_names: Vec<String>,
        modules: &[(String, ModModule, std::path::PathBuf, String)],
        graph: &DependencyGraph,
    ) -> Vec<String> {
        // Convert wrapper_names to a set for O(1) lookups
        let wrapper_names_set: FxIndexSet<String> = wrapper_names.iter().cloned().collect();

        // Filter to only wrapper modules for dependency map building
        let wrapper_modules: Vec<String> = modules
            .iter()
            .filter_map(|(name, _, _, _)| {
                if wrapper_names_set.contains(name) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Build dependency map using the helper
        let dependency_map =
            Self::build_dependency_map(&wrapper_modules, &wrapper_names_set, graph);

        // Perform topological sort
        match Self::topological_sort(&dependency_map) {
            Ok(sorted) => sorted,
            Err(cycle) => {
                warn!(
                    "Circular dependency detected in wrapper modules: {}",
                    cycle.join(" -> ")
                );
                // Return original order if cycle detected
                wrapper_names
            }
        }
    }

    /// Sort wrapped modules (modules within a circular group) by their dependencies
    pub fn sort_wrapped_modules_by_dependencies(
        module_names: Vec<String>,
        graph: &DependencyGraph,
    ) -> Vec<String> {
        // Convert module_names to a set for O(1) lookups
        let module_names_set: FxIndexSet<String> = module_names.iter().cloned().collect();

        // Build dependency map using the helper
        let dependency_map = Self::build_dependency_map(&module_names, &module_names_set, graph);

        // Perform topological sort
        match Self::topological_sort(&dependency_map) {
            Ok(sorted) => {
                debug!("Successfully sorted wrapped modules: {sorted:?}");
                sorted
            }
            Err(cycle) => {
                debug!(
                    "Circular dependency within wrapped modules (expected): {}",
                    cycle.join(" -> ")
                );
                // For circular dependencies within wrapped modules,
                // preserve the original order
                module_names
            }
        }
    }

    /// Perform topological sort on a dependency map
    /// The dependencies map format: key depends on values
    /// e.g., {"a": ["b", "c"]} means "a depends on b and c"
    fn topological_sort(
        dependencies: &FxIndexMap<String, FxIndexSet<String>>,
    ) -> Result<Vec<String>, Vec<String>> {
        let mut in_degree: FxIndexMap<String, usize> = FxIndexMap::default();
        let mut result = Vec::new();

        // Calculate in-degrees
        for (node, _) in dependencies {
            in_degree.entry(node.clone()).or_insert(0);
        }
        for (_, deps) in dependencies {
            for dep in deps {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
            }
        }

        // Find nodes with no incoming edges
        let mut queue: Vec<String> = in_degree
            .iter()
            .filter_map(|(node, &degree)| {
                if degree == 0 {
                    Some(node.clone())
                } else {
                    None
                }
            })
            .collect();

        // Process nodes
        while let Some(node) = queue.pop() {
            result.push(node.clone());

            if let Some(deps) = dependencies.get(&node) {
                for dep in deps {
                    if let Some(degree) = in_degree.get_mut(dep) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(dep.clone());
                        }
                    }
                }
            }
        }

        // Check if all nodes were processed
        if result.len() == dependencies.len() {
            // Reverse the result to get proper dependency order
            // (dependencies come before dependents)
            Ok(result.into_iter().rev().collect())
        } else {
            // Find the actual cycle using DFS
            let processed: FxIndexSet<String> = result.into_iter().collect();
            let unprocessed: Vec<String> = dependencies
                .keys()
                .filter(|k| !processed.contains(*k))
                .cloned()
                .collect();

            // Use DFS to find the actual cycle path
            if let Some(cycle) = Self::find_cycle_dfs(dependencies, &unprocessed) {
                Err(cycle)
            } else {
                // Fallback to returning all unprocessed nodes if we can't find a specific cycle
                Err(unprocessed)
            }
        }
    }

    /// Find a cycle using DFS starting from unprocessed nodes
    fn find_cycle_dfs(
        dependencies: &FxIndexMap<String, FxIndexSet<String>>,
        unprocessed: &[String],
    ) -> Option<Vec<String>> {
        let mut visited = FxIndexSet::default();
        let mut rec_stack = FxIndexSet::default();
        let mut parent = FxIndexMap::default();

        // Try to find a cycle starting from each unprocessed node
        for start_node in unprocessed {
            if visited.contains(start_node) {
                continue;
            }

            let mut stack = vec![(start_node.clone(), false)];

            while let Some((node, backtrack)) = stack.pop() {
                if backtrack {
                    rec_stack.swap_remove(&node);
                    continue;
                }

                if rec_stack.contains(&node) {
                    // Skip - already being processed
                    continue;
                }

                if visited.contains(&node) {
                    continue;
                }

                visited.insert(node.clone());
                rec_stack.insert(node.clone());
                stack.push((node.clone(), true)); // Push backtrack marker

                if let Some(deps) = dependencies.get(&node) {
                    for dep in deps {
                        if !visited.contains(dep) {
                            parent.insert(dep.clone(), node.clone());
                            stack.push((dep.clone(), false));
                        } else if rec_stack.contains(dep) {
                            // Found a cycle - reconstruct the path
                            return Some(Self::reconstruct_cycle(&parent, node.clone(), dep));
                        }
                    }
                }
            }
        }

        None
    }

    /// Reconstruct a cycle from parent pointers
    fn reconstruct_cycle(
        parent: &FxIndexMap<String, String>,
        start_node: String,
        cycle_target: &str,
    ) -> Vec<String> {
        let mut cycle = Vec::new();
        let mut current = start_node;

        // Add the current node
        cycle.push(current.clone());

        // Follow parent pointers until we reach the dependency that creates the cycle
        while current != *cycle_target {
            let Some(parent_node) = parent.get(&current) else {
                break;
            };
            current = parent_node.clone();
            cycle.push(current.clone());
        }

        // The cycle should be in the correct order now
        cycle.reverse();

        // Remove any nodes before the actual cycle starts
        if let Some(pos) = cycle.iter().position(|x| x == cycle_target) {
            cycle = cycle[pos..].to_vec();
        }

        cycle
    }

    /// Analyze circular dependencies and classify them
    pub fn analyze_circular_dependencies(graph: &DependencyGraph) -> CircularDependencyAnalysis {
        let sccs = graph.find_strongly_connected_components();

        let mut resolvable_cycles = Vec::new();
        let mut unresolvable_cycles = Vec::new();

        for scc in sccs {
            if scc.len() <= 1 {
                continue; // Not a cycle
            }

            // Convert module IDs to names
            let module_names: Vec<String> = scc
                .iter()
                .filter_map(|&module_id| {
                    graph.modules.get(&module_id).map(|m| m.module_name.clone())
                })
                .collect();

            if module_names.is_empty() {
                continue;
            }

            let cycle_type = Self::classify_cycle_type(graph, &module_names);
            let suggested_resolution =
                Self::suggest_resolution_for_cycle(&cycle_type, &module_names);

            let group = CircularDependencyGroup {
                modules: module_names,
                cycle_type: cycle_type.clone(),
                suggested_resolution,
            };

            // Categorize based on cycle type
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
        }
    }

    /// Classify the type of circular dependency
    fn classify_cycle_type(
        graph: &DependencyGraph,
        module_names: &[String],
    ) -> CircularDependencyType {
        // Check if this is a parent-child package cycle
        // These occur when a package imports from its subpackage (e.g., pkg/__init__.py imports
        // from pkg.submodule)
        if Self::is_parent_child_package_cycle(module_names) {
            // This is a normal Python pattern, not a problematic cycle
            return CircularDependencyType::FunctionLevel; // Most permissive type
        }

        // Check if imports can be moved to functions
        // Special case: if modules have NO items (empty or only imports), treat as FunctionLevel
        // This handles simple circular import cases like stickytape tests
        let all_empty = Self::all_modules_empty_or_imports_only(graph, module_names);

        if all_empty {
            // Simple circular imports can often be resolved
            return CircularDependencyType::FunctionLevel;
        }

        // Perform AST analysis on the modules in the cycle
        let analysis_result = Self::analyze_cycle_modules(graph, module_names);

        // Use AST analysis results for classification
        if analysis_result.has_only_constants
            && !module_names.iter().any(|name| name.ends_with("__init__"))
        {
            // Modules that only contain constants create unresolvable cycles
            // Exception: __init__.py files often only have imports/exports which is normal
            return CircularDependencyType::ModuleConstants;
        }

        if analysis_result.has_class_definitions {
            // Check if the circular imports are used for inheritance
            // If all imports in the cycle are only used in functions, it's still FunctionLevel
            if analysis_result.imports_used_in_functions_only {
                return CircularDependencyType::FunctionLevel;
            }
            // Otherwise, it's a true class-level cycle
            return CircularDependencyType::ClassLevel;
        }

        // Fall back to name-based heuristics if AST analysis is inconclusive
        for module_name in module_names {
            if module_name.contains("constants") || module_name.contains("config") {
                return CircularDependencyType::ModuleConstants;
            }
            if module_name.contains("class") || module_name.ends_with("_class") {
                return CircularDependencyType::ClassLevel;
            }
        }

        // Default classification based on remaining heuristics
        if analysis_result.imports_used_in_functions_only {
            CircularDependencyType::FunctionLevel
        } else if analysis_result.has_module_level_imports
            || module_names.iter().any(|name| name.contains("__init__"))
        {
            CircularDependencyType::ImportTime
        } else {
            CircularDependencyType::FunctionLevel
        }
    }

    /// Analyze modules in a cycle to determine their characteristics
    /// Returns a `CycleAnalysisResult` containing the analysis of the modules in the cycle.
    fn analyze_cycle_modules(
        graph: &DependencyGraph,
        module_names: &[String],
    ) -> CycleAnalysisResult {
        let mut has_only_constants = true;
        let mut has_class_definitions = false;
        let mut has_module_level_imports = false;
        let mut imports_used_in_functions_only = true;

        for module_name in module_names {
            if let Some(module) = graph.get_module_by_name(module_name) {
                for item in module.items.values() {
                    match &item.item_type {
                        ItemType::FunctionDef { .. } => {
                            has_only_constants = false;
                        }
                        ItemType::ClassDef { .. } => {
                            has_only_constants = false;
                            has_class_definitions = true;
                        }
                        ItemType::Import { .. } | ItemType::FromImport { .. } => {
                            // Since we can't determine scope from ItemData directly,
                            // check if this import is only referenced within function definitions
                            // This is a heuristic: if an import has no direct module-level usage,
                            // it's likely a function-scoped import
                            let import_vars = &item.var_decls;

                            // Check if any of the imported names are used at module level
                            let used_at_module_level = module.items.values().any(|other_item| {
                                // Skip function and class definitions when checking usage
                                if matches!(
                                    other_item.item_type,
                                    ItemType::FunctionDef { .. } | ItemType::ClassDef { .. }
                                ) {
                                    return false;
                                }

                                // Check if this item uses any of the imported variables
                                import_vars
                                    .iter()
                                    .any(|import_var| other_item.read_vars.contains(import_var))
                            });

                            if used_at_module_level {
                                has_module_level_imports = true;
                                imports_used_in_functions_only = false;
                            }
                            // If not used at module level, the import is likely function-scoped
                        }
                        ItemType::Assignment { .. } => {
                            // Not all assignments are constants
                            has_only_constants = false;
                        }
                        _ => {}
                    }
                }
            }
        }

        CycleAnalysisResult {
            has_only_constants,
            has_class_definitions,
            has_module_level_imports,
            imports_used_in_functions_only,
        }
    }

    /// Check if all modules in the cycle are empty or contain only imports
    fn all_modules_empty_or_imports_only(graph: &DependencyGraph, module_names: &[String]) -> bool {
        for module_name in module_names {
            if let Some(module) = graph.get_module_by_name(module_name) {
                for item in module.items.values() {
                    match &item.item_type {
                        ItemType::Import { .. } | ItemType::FromImport { .. } => {
                            // Imports are allowed
                        }
                        _ => {
                            // Any other item means it's not empty/imports-only
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    /// Check if modules form a parent-child package relationship
    fn is_parent_child_package_cycle(module_names: &[String]) -> bool {
        for parent in module_names {
            for child in module_names {
                if parent != child && child.starts_with(&format!("{parent}.")) {
                    return true;
                }
            }
        }
        false
    }

    /// Suggest resolution strategy for a cycle
    fn suggest_resolution_for_cycle(
        cycle_type: &CircularDependencyType,
        _module_names: &[String],
    ) -> ResolutionStrategy {
        match cycle_type {
            CircularDependencyType::FunctionLevel => ResolutionStrategy::FunctionScopedImport,
            CircularDependencyType::ClassLevel => ResolutionStrategy::LazyImport,
            CircularDependencyType::ModuleConstants => ResolutionStrategy::Unresolvable {
                reason: "Module-level constants create temporal paradox - consider moving to a \
                         shared configuration module"
                    .into(),
            },
            CircularDependencyType::ImportTime => ResolutionStrategy::ModuleSplit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topological_sort_no_cycles() {
        let mut deps = FxIndexMap::default();
        deps.insert(
            "a".to_string(),
            ["b", "c"].iter().map(|s| (*s).to_string()).collect(),
        );
        deps.insert(
            "b".to_string(),
            ["d"].iter().map(|s| (*s).to_string()).collect(),
        );
        deps.insert(
            "c".to_string(),
            ["d"].iter().map(|s| (*s).to_string()).collect(),
        );
        deps.insert("d".to_string(), FxIndexSet::default());

        let result = DependencyAnalyzer::topological_sort(&deps)
            .expect("Topological sort should succeed for DAG");

        // In our topological sort, if a depends on b,c and b,c depend on d,
        // then the order should be: d first (no dependencies), then b,c (depend on d), then a
        // (depends on b,c) This ensures dependencies are processed before dependents
        let a_pos = result
            .iter()
            .position(|x| x == "a")
            .expect("Module 'a' should be in the result");
        let b_pos = result
            .iter()
            .position(|x| x == "b")
            .expect("Module 'b' should be in the result");
        let c_pos = result
            .iter()
            .position(|x| x == "c")
            .expect("Module 'c' should be in the result");
        let d_pos = result
            .iter()
            .position(|x| x == "d")
            .expect("Module 'd' should be in the result");

        // d should come first (no dependencies)
        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
        // b and c should come before a (since a depends on them)
        assert!(b_pos < a_pos);
        assert!(c_pos < a_pos);
    }

    #[test]
    fn test_topological_sort_with_cycle() {
        let mut deps = FxIndexMap::default();
        deps.insert(
            "a".to_string(),
            ["b"].iter().map(|s| (*s).to_string()).collect(),
        );
        deps.insert(
            "b".to_string(),
            ["c"].iter().map(|s| (*s).to_string()).collect(),
        );
        deps.insert(
            "c".to_string(),
            ["a"].iter().map(|s| (*s).to_string()).collect(),
        );

        let result = DependencyAnalyzer::topological_sort(&deps);
        assert!(result.is_err());

        if let Err(cycle) = result {
            assert_eq!(cycle.len(), 3);
            assert!(cycle.contains(&"a".to_string()));
            assert!(cycle.contains(&"b".to_string()));
            assert!(cycle.contains(&"c".to_string()));
        }
    }
}
