//! Dependency analysis module
//!
//! This module provides functionality for analyzing dependencies between modules,
//! including circular dependency detection and topological sorting.

use crate::{
    analyzers::types::{
        CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType,
        ResolutionStrategy,
    },
    dependency_graph::{DependencyGraph, ItemType},
};

/// Analyze circular dependencies and classify them
pub(crate) fn analyze_circular_dependencies(graph: &DependencyGraph) -> CircularDependencyAnalysis {
    let sccs = graph.find_strongly_connected_components();

    let mut resolvable_cycles = Vec::new();
    let mut unresolvable_cycles = Vec::new();

    for scc in sccs {
        if scc.len() <= 1 {
            continue; // Not a cycle
        }

        // Work directly with module IDs (already resolver::ModuleId since DependencyGraph
        // re-exports it)
        let module_ids: Vec<crate::resolver::ModuleId> = scc.clone();
        // Non-empty by construction (scc.len() > 1 above)

        let cycle_type = classify_cycle_type(graph, &module_ids);
        let suggested_resolution = suggest_resolution_for_cycle(&cycle_type);

        let group = CircularDependencyGroup {
            modules: module_ids,
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
    module_ids: &[crate::resolver::ModuleId],
) -> CircularDependencyType {
    // Get module names for analysis
    let module_names: Vec<String> = module_ids
        .iter()
        .filter_map(|id| graph.modules.get(id).map(|m| m.module_name.clone()))
        .collect();

    // Check if this is a parent-child package cycle
    // These occur when a package imports from its subpackage (e.g., pkg/__init__.py imports
    // from pkg.submodule)
    if is_parent_child_package_cycle(&module_names) {
        // This is a normal Python pattern, not a problematic cycle
        return CircularDependencyType::FunctionLevel; // Most permissive type
    }

    // Check if imports can be moved to functions
    // Special case: if modules have NO items (empty or only imports), treat as FunctionLevel
    // This handles simple circular import cases like stickytape tests
    let all_empty = all_modules_empty_or_imports_only(graph, module_ids);

    if all_empty {
        // Simple circular imports can often be resolved
        return CircularDependencyType::FunctionLevel;
    }

    // Perform AST analysis on the modules in the cycle
    let analysis_result = analyze_cycle_modules(graph, module_ids);

    // Use AST analysis results for classification
    if analysis_result.has_only_constants
        && !module_names
            .iter()
            .any(|name| crate::util::is_init_module(name))
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

    // Cross-cycle module-level reads: bidirectional temporal paradox detected via AST.
    // Two or more modules import from each other and consume those imports in module-level
    // assignments (e.g., A_VALUE = B_VALUE + 1 / B_VALUE = A_VALUE * 2). Neither module
    // can initialize first.
    if analysis_result.has_cross_cycle_module_level_reads {
        return CircularDependencyType::ModuleConstants;
    }

    // Default classification based on remaining heuristics
    if analysis_result.imports_used_in_functions_only {
        CircularDependencyType::FunctionLevel
    } else if analysis_result.has_module_level_imports
        || module_names
            .iter()
            .any(|name| crate::util::is_init_module(name))
    {
        CircularDependencyType::ImportTime
    } else {
        CircularDependencyType::FunctionLevel
    }
}

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
    /// Whether 2+ modules in the cycle have module-level assignments that read
    /// names imported from other cycle members (bidirectional temporal paradox)
    has_cross_cycle_module_level_reads: bool,
}

/// Analyze modules in a cycle to determine their characteristics
/// Returns a `CycleAnalysisResult` containing the analysis of the modules in the cycle.
fn analyze_cycle_modules(
    graph: &DependencyGraph,
    module_ids: &[crate::resolver::ModuleId],
) -> CycleAnalysisResult {
    // Collect cycle member names for cross-cycle detection
    let cycle_member_names: crate::types::FxIndexSet<String> = module_ids
        .iter()
        .filter_map(|id| graph.modules.get(id).map(|m| m.module_name.clone()))
        .collect();

    let mut has_only_constants = true;
    let mut has_class_definitions = false;
    let mut has_module_level_imports = false;
    let mut imports_used_in_functions_only = true;
    let mut modules_with_cross_cycle_reads = 0_u32;

    for id in module_ids {
        if let Some(module) = graph.get_module(*id) {
            // Collect names imported from OTHER cycle members (absolute imports only)
            let mut cross_cycle_imported_names = crate::types::FxIndexSet::<String>::default();

            for item in module.items.values() {
                match &item.item_type {
                    ItemType::FunctionDef { .. } => {
                        has_only_constants = false;
                    }
                    ItemType::ClassDef { .. } => {
                        has_only_constants = false;
                        has_class_definitions = true;
                    }
                    ItemType::FromImport {
                        module: from_module,
                        names,
                        level,
                        ..
                    } => {
                        // Track names imported from cycle members (for cross-cycle detection)
                        if *level == 0 && cycle_member_names.contains(from_module) {
                            for (name, alias) in names {
                                let local_name = alias.as_ref().unwrap_or(name);
                                cross_cycle_imported_names.insert(local_name.clone());
                            }
                        }

                        // Existing: check if import is used at module level
                        let import_vars = &item.var_decls;
                        let used_at_module_level = module.items.values().any(|other_item| {
                            if matches!(
                                other_item.item_type,
                                ItemType::FunctionDef { .. } | ItemType::ClassDef { .. }
                            ) {
                                return false;
                            }
                            import_vars
                                .iter()
                                .any(|import_var| other_item.read_vars.contains(import_var))
                        });

                        if used_at_module_level {
                            has_module_level_imports = true;
                            imports_used_in_functions_only = false;
                        }
                    }
                    ItemType::Import { .. } => {
                        // `import X` — check if used at module level (same as FromImport)
                        let import_vars = &item.var_decls;
                        let used_at_module_level = module.items.values().any(|other_item| {
                            if matches!(
                                other_item.item_type,
                                ItemType::FunctionDef { .. } | ItemType::ClassDef { .. }
                            ) {
                                return false;
                            }
                            import_vars
                                .iter()
                                .any(|import_var| other_item.read_vars.contains(import_var))
                        });

                        if used_at_module_level {
                            has_module_level_imports = true;
                            imports_used_in_functions_only = false;
                        }
                    }
                    ItemType::Assignment { .. } => {
                        // Assignments are constant-like; don't clear has_only_constants.
                    }
                    _ => {}
                }
            }

            // Check if any module-level item reads a name imported from a cycle member.
            // This detects temporal paradoxes: `from cycle_member import X` + `Y = X + 1`
            if !cross_cycle_imported_names.is_empty() {
                let has_module_level_read = module.items.values().any(|item| {
                    // Only check non-function/class/import items (module-level statements)
                    !matches!(
                        item.item_type,
                        ItemType::FunctionDef { .. }
                            | ItemType::ClassDef { .. }
                            | ItemType::Import { .. }
                            | ItemType::FromImport { .. }
                    ) && cross_cycle_imported_names
                        .iter()
                        .any(|name| item.read_vars.contains(name))
                });
                if has_module_level_read {
                    modules_with_cross_cycle_reads += 1;
                }
            }
        }
    }

    CycleAnalysisResult {
        has_only_constants,
        has_class_definitions,
        has_module_level_imports,
        imports_used_in_functions_only,
        // Bidirectional temporal paradox: 2+ modules read cross-cycle imports at module level
        has_cross_cycle_module_level_reads: modules_with_cross_cycle_reads >= 2,
    }
}

/// Check if all modules in the cycle are empty or contain only imports
fn all_modules_empty_or_imports_only(
    graph: &DependencyGraph,
    module_ids: &[crate::resolver::ModuleId],
) -> bool {
    for id in module_ids {
        if let Some(module) = graph.get_module(*id) {
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
fn suggest_resolution_for_cycle(cycle_type: &CircularDependencyType) -> ResolutionStrategy {
    match cycle_type {
        CircularDependencyType::ModuleConstants => ResolutionStrategy::Unresolvable {
            reason: "Module-level constants create temporal paradox - consider moving to a shared \
                     configuration module"
                .into(),
        },
        _ => ResolutionStrategy::Resolvable,
    }
}
