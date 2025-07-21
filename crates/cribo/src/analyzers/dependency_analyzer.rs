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

/// Dependency analyzer for module and symbol dependencies
pub struct DependencyAnalyzer;

impl DependencyAnalyzer {
    /// Sort wrapper modules by their dependencies
    pub fn sort_wrapper_modules_by_dependencies(
        wrapper_names: Vec<String>,
        modules: &[(String, ModModule, std::path::PathBuf, String)],
        graph: &DependencyGraph,
    ) -> Vec<String> {
        // Build a dependency map for wrapper modules
        let mut dependency_map: FxIndexMap<String, FxIndexSet<String>> = FxIndexMap::default();

        for wrapper in &wrapper_names {
            dependency_map.insert(wrapper.clone(), FxIndexSet::default());
        }

        // For each wrapper module, find its dependencies on other wrapper modules
        for (module_name, _, _, _) in modules {
            if wrapper_names.contains(module_name)
                && let Some(&module_id) = graph.module_names.get(module_name) {
                    let dependencies = graph.get_dependencies(module_id);
                    for dep_id in dependencies {
                        if let Some(dep_module) = graph.modules.get(&dep_id) {
                            let dep_name = &dep_module.module_name;
                            if wrapper_names.contains(dep_name) && dep_name != module_name {
                                dependency_map
                                    .get_mut(module_name)
                                    .unwrap()
                                    .insert(dep_name.clone());
                            }
                        }
                    }
                }
        }

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
        // Build a dependency map for the modules
        let mut dependency_map: FxIndexMap<String, FxIndexSet<String>> = FxIndexMap::default();

        // Initialize all modules
        for module in &module_names {
            dependency_map.insert(module.clone(), FxIndexSet::default());
        }

        // For each module, find its dependencies on other modules in the group
        for module_name in &module_names {
            if let Some(&module_id) = graph.module_names.get(module_name) {
                let dependencies = graph.get_dependencies(module_id);
                for dep_id in dependencies {
                    if let Some(dep_module) = graph.modules.get(&dep_id) {
                        let dep_name = &dep_module.module_name;
                        if module_names.contains(dep_name) && dep_name != module_name {
                            dependency_map
                                .get_mut(module_name)
                                .unwrap()
                                .insert(dep_name.clone());
                        }
                    }
                }
            }
        }

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
            Ok(result)
        } else {
            // Find a cycle for error reporting
            let processed: FxIndexSet<String> = result.into_iter().collect();
            let remaining: Vec<String> = dependencies
                .keys()
                .filter(|k| !processed.contains(*k))
                .cloned()
                .collect();

            // For simplicity, return the remaining nodes as the cycle
            Err(remaining)
        }
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
        if analysis_result.0 // has_only_constants
            && !module_names.iter().any(|name| name.ends_with("__init__"))
        {
            // Modules that only contain constants create unresolvable cycles
            // Exception: __init__.py files often only have imports/exports which is normal
            return CircularDependencyType::ModuleConstants;
        }

        if analysis_result.1 {
            // has_class_definitions
            // Check if the circular imports are used for inheritance
            // If all imports in the cycle are only used in functions, it's still FunctionLevel
            if analysis_result.3 {
                // imports_used_in_functions_only
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
        if analysis_result.3 {
            // imports_used_in_functions_only
            CircularDependencyType::FunctionLevel
        } else if analysis_result.2 // has_module_level_imports
            || module_names.iter().any(|name| name.contains("__init__"))
        {
            CircularDependencyType::ImportTime
        } else {
            CircularDependencyType::FunctionLevel
        }
    }

    /// Analyze modules in a cycle to determine their characteristics
    /// Returns (has_only_constants, has_class_definitions, has_module_level_imports,
    /// imports_used_in_functions_only)
    fn analyze_cycle_modules(
        graph: &DependencyGraph,
        module_names: &[String],
    ) -> (bool, bool, bool, bool) {
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
                            // For now, assume all imports are at module level
                            // (proper scope tracking would require enhanced AST analysis)
                            has_module_level_imports = true;
                            imports_used_in_functions_only = false;
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

        (
            has_only_constants,
            has_class_definitions,
            has_module_level_imports,
            imports_used_in_functions_only,
        )
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
            ["b", "c"].iter().map(|s| s.to_string()).collect(),
        );
        deps.insert(
            "b".to_string(),
            ["d"].iter().map(|s| s.to_string()).collect(),
        );
        deps.insert(
            "c".to_string(),
            ["d"].iter().map(|s| s.to_string()).collect(),
        );
        deps.insert("d".to_string(), FxIndexSet::default());

        let result = DependencyAnalyzer::topological_sort(&deps).unwrap();

        // In our topological sort, if a depends on b,c and b,c depend on d,
        // then the order is: a (no incoming edges), then b,c, then d
        // This is because we're processing nodes that have no incoming dependencies first
        let a_pos = result.iter().position(|x| x == "a").unwrap();
        let b_pos = result.iter().position(|x| x == "b").unwrap();
        let c_pos = result.iter().position(|x| x == "c").unwrap();
        let d_pos = result.iter().position(|x| x == "d").unwrap();

        // a should come first (no incoming edges)
        assert!(a_pos < b_pos);
        assert!(a_pos < c_pos);
        // b and c should come before d
        assert!(b_pos < d_pos);
        assert!(c_pos < d_pos);
    }

    #[test]
    fn test_topological_sort_with_cycle() {
        let mut deps = FxIndexMap::default();
        deps.insert(
            "a".to_string(),
            ["b"].iter().map(|s| s.to_string()).collect(),
        );
        deps.insert(
            "b".to_string(),
            ["c"].iter().map(|s| s.to_string()).collect(),
        );
        deps.insert(
            "c".to_string(),
            ["a"].iter().map(|s| s.to_string()).collect(),
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
