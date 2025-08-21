//! Import deduplication and cleanup utilities
//!
//! This module contains functions for finding and removing duplicate or unused imports,
//! and other import-related cleanup tasks during the bundling process.

use std::path::PathBuf;

use ruff_python_ast::{Alias, Expr, ModModule, Stmt, StmtImport, StmtImportFrom};

use super::{bundler::Bundler, expression_handlers};
use crate::{
    code_generator::module_registry::is_init_function, cribo_graph::CriboGraph as DependencyGraph,
    tree_shaking::TreeShaker, types::FxIndexSet,
};

/// Check if a statement is a hoisted import
pub(super) fn is_hoisted_import(_bundler: &Bundler, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ImportFrom(import_from) => {
            if let Some(ref module) = import_from.module {
                let module_name = module.as_str();
                // Check if this is a __future__ import (always hoisted)
                if module_name == "__future__" {
                    return true;
                }
                // Stdlib imports are no longer hoisted - handled by proxy
            }
            false
        }
        Stmt::Import(_import_stmt) => {
            // Stdlib imports are no longer hoisted - handled by proxy
            false
        }
        _ => false,
    }
}

/// Collect imports from a module for hoisting
pub(super) fn collect_imports_from_module(
    bundler: &mut Bundler,
    ast: &ModModule,
    module_name: &str,
    python_version: u8,
) {
    log::debug!("Collecting imports from module: {module_name}");
    for stmt in &ast.body {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                // Skip relative imports - they can never be stdlib imports
                if import_from.level > 0 {
                    log::trace!(
                        "Skipping relative import: from {} import {:?} (level: {})",
                        import_from
                            .module
                            .as_ref()
                            .map_or("", ruff_python_ast::Identifier::as_str),
                        import_from
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>(),
                        import_from.level
                    );
                    // Do not process relative imports as stdlib
                    continue;
                }
                if let Some(module) = &import_from.module {
                    let module_str = module.as_str();

                    log::debug!(
                        "Checking import: from {} import {:?} (level: {})",
                        module_str,
                        import_from
                            .names
                            .iter()
                            .map(|a| a.name.as_str())
                            .collect::<Vec<_>>(),
                        import_from.level
                    );

                    // Stdlib imports are now handled by the _cribo proxy
                    // We only need to track aliases for expression transformation
                    let root_module = module_str.split('.').next().unwrap_or(module_str);
                    if module_str != "__future__"
                        && ruff_python_stdlib::sys::is_known_standard_library(
                            python_version,
                            root_module,
                        )
                    {
                        // Track aliases for stdlib modules (for expression transformation)
                        for alias in &import_from.names {
                            if let Some(alias_name) = alias.asname.as_ref() {
                                bundler.stdlib_module_aliases.insert(
                                    alias_name.as_str().to_string(),
                                    module_str.to_string(),
                                );
                            }
                        }
                    }
                }
            }
            Stmt::Import(import_stmt) => {
                // Track stdlib module aliases for expression transformation
                for alias in &import_stmt.names {
                    let imported_module_name = alias.name.as_str();
                    let root_module = imported_module_name
                        .split('.')
                        .next()
                        .unwrap_or(imported_module_name);
                    if ruff_python_stdlib::sys::is_known_standard_library(
                        python_version,
                        root_module,
                    ) {
                        // Track the module and its alias
                        if let Some(alias_name) = alias.asname.as_ref() {
                            bundler.stdlib_module_aliases.insert(
                                alias_name.as_str().to_string(),
                                imported_module_name.to_string(),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Add hoisted imports to the final body
pub(super) fn add_hoisted_imports(bundler: &Bundler, final_body: &mut Vec<Stmt>) {
    use crate::ast_builder::{other, statements};

    // Future imports first - combine all into a single import statement
    if !bundler.future_imports.is_empty() {
        // Sort future imports for deterministic output
        let mut sorted_imports: Vec<String> = bundler.future_imports.iter().cloned().collect();
        sorted_imports.sort();

        let aliases: Vec<Alias> = sorted_imports
            .into_iter()
            .map(|import| other::alias(&import, None))
            .collect();

        final_body.push(statements::import_from(Some("__future__"), aliases, 0));
    }

    // Stdlib imports are now handled by _cribo proxy, no need to add them here
    // Third-party imports are NEVER hoisted because they may have side effects
    // (e.g., registering plugins, modifying global state, network calls).
    // Third-party imports remain in their original location to preserve execution order.
}

/// Deduplicate deferred imports against existing body statements
pub(super) fn deduplicate_deferred_imports_with_existing(
    imports: Vec<Stmt>,
    existing_body: &[Stmt],
) -> Vec<Stmt> {
    let mut seen_init_calls = FxIndexSet::default();
    let mut seen_assignments = FxIndexSet::default();
    let mut result = Vec::new();

    // First, collect all existing assignments from the body
    for stmt in existing_body {
        if let Stmt::Assign(assign) = stmt
            && assign.targets.len() == 1
        {
            // Handle attribute assignments like schemas.user = ...
            if let Expr::Attribute(target_attr) = &assign.targets[0] {
                let target_path = expression_handlers::extract_attribute_path(target_attr);

                // Handle init function calls
                if let Expr::Call(call) = &assign.value.as_ref()
                    && let Expr::Name(name) = &call.func.as_ref()
                {
                    let func_name = name.id.as_str();
                    if is_init_function(func_name) {
                        // Use just the target path as the key for module init assignments
                        let key = target_path.clone();
                        log::debug!("Found existing module init assignment: {key} = {func_name}");
                        seen_assignments.insert(key);
                    } else {
                        // For other attribute assignments like pkg_compat.bytes = bytes
                        // Only track simple name assignments to avoid catching namespace creations
                        if let Expr::Name(value_name) = &assign.value.as_ref() {
                            let value_key = format!("{} = {}", target_path, value_name.id.as_str());
                            seen_assignments.insert(value_key);
                        }
                    }
                } else {
                    // For non-call attribute assignments
                    // Only track simple name assignments
                    if let Expr::Name(value_name) = &assign.value.as_ref() {
                        let value_key = format!("{} = {}", target_path, value_name.id.as_str());
                        seen_assignments.insert(value_key);
                    }
                }
            }
            // Handle simple name assignments
            else if let Expr::Name(target) = &assign.targets[0] {
                let target_str = target.id.as_str();

                // Handle simple name assignments
                if let Expr::Name(value) = &assign.value.as_ref() {
                    let key = format!("{} = {}", target_str, value.id.as_str());
                    seen_assignments.insert(key);
                }
                // Handle attribute assignments like User = services.auth.manager.User
                else if let Expr::Attribute(attr) = &assign.value.as_ref() {
                    let attr_path = expression_handlers::extract_attribute_path(attr);
                    let key = format!("{target_str} = {attr_path}");
                    seen_assignments.insert(key);
                }
            }
        }
    }

    log::debug!(
        "Found {} existing assignments in body",
        seen_assignments.len()
    );
    log::debug!("Deduplicating {} deferred imports", imports.len());

    // Now process the deferred imports
    for (idx, stmt) in imports.into_iter().enumerate() {
        log::debug!("Processing deferred import {idx}: {stmt:?}");
        match &stmt {
            // Check for init function calls
            Stmt::Expr(expr_stmt) => {
                if let Expr::Call(call) = &expr_stmt.value.as_ref() {
                    if let Expr::Name(name) = &call.func.as_ref() {
                        let func_name = name.id.as_str();
                        if is_init_function(func_name) {
                            if seen_init_calls.insert(func_name.to_string()) {
                                result.push(stmt);
                            } else {
                                log::debug!("Skipping duplicate init call: {func_name}");
                            }
                        } else {
                            result.push(stmt);
                        }
                    } else {
                        result.push(stmt);
                    }
                } else {
                    result.push(stmt);
                }
            }
            // Check for symbol assignments
            Stmt::Assign(assign) => {
                // First check if this is an attribute assignment with an init function call
                // like: schemas.user = <cribo_init_prefix>__cribo_f275a8_schemas_user()
                if assign.targets.len() == 1
                    && let Expr::Attribute(target_attr) = &assign.targets[0]
                {
                    let target_path = expression_handlers::extract_attribute_path(target_attr);

                    // Check if value is an init function call
                    if let Expr::Call(call) = &assign.value.as_ref()
                        && let Expr::Name(name) = &call.func.as_ref()
                    {
                        let func_name = name.id.as_str();
                        if is_init_function(func_name) {
                            // For module init assignments, just check the target path
                            // since the same module should only be initialized once
                            let key = target_path.clone();
                            log::debug!(
                                "Checking deferred module init assignment: {key} = {func_name}"
                            );
                            if seen_assignments.contains(&key) {
                                log::debug!(
                                    "Skipping duplicate module init assignment: {key} = \
                                     {func_name}"
                                );
                                continue; // Skip this statement entirely
                            }
                            log::debug!("Adding new module init assignment: {key} = {func_name}");
                            seen_assignments.insert(key);
                            result.push(stmt);
                            continue;
                        }
                    }

                    // Also handle general attribute assignments like pkg_compat.bytes = bytes
                    // But NOT namespace creations (types.SimpleNamespace())
                    // Only deduplicate simple name assignments to attributes
                    if let Expr::Name(value_name) = &assign.value.as_ref() {
                        let key = format!("{} = {}", target_path, value_name.id.as_str());

                        if seen_assignments.contains(&key) {
                            log::debug!("Skipping duplicate attribute assignment: {key}");
                            continue;
                        }

                        seen_assignments.insert(key.clone());
                        log::debug!("Adding attribute assignment: {key}");
                    }
                    // For other types (like namespace creations), don't deduplicate
                    result.push(stmt);
                    continue;
                }

                // Check for simple assignments like: Logger = Logger_4
                if assign.targets.len() == 1 {
                    if let Expr::Name(target) = &assign.targets[0] {
                        if let Expr::Name(value) = &assign.value.as_ref() {
                            // This is a simple name assignment
                            let target_str = target.id.as_str();
                            let value_str = value.id.as_str();
                            let key = format!("{target_str} = {value_str}");

                            // Check for self-assignment
                            if target_str == value_str {
                                log::debug!("Found self-assignment in deferred imports: {key}");
                                // Skip self-assignments entirely
                                log::debug!("Skipping self-assignment: {key}");
                            } else if seen_assignments.insert(key.clone()) {
                                log::debug!("First occurrence of simple assignment: {key}");
                                result.push(stmt);
                            } else {
                                log::debug!("Skipping duplicate simple assignment: {key}");
                            }
                        } else {
                            // Not a simple name assignment, check for duplicates
                            // Handle attribute assignments like User =
                            // services.auth.manager.User
                            let target_str = target.id.as_str();

                            // For attribute assignments, extract the actual attribute path
                            let key = if let Expr::Attribute(attr) = &assign.value.as_ref() {
                                // Extract the full attribute path (e.g.,
                                // services.auth.manager.User)
                                let attr_path = expression_handlers::extract_attribute_path(attr);
                                format!("{target_str} = {attr_path}")
                            } else {
                                // Fallback to debug format for other types
                                let value_str = format!("{:?}", assign.value);
                                format!("{target_str} = {value_str}")
                            };

                            if seen_assignments.insert(key.clone()) {
                                log::debug!("First occurrence of attribute assignment: {key}");
                                result.push(stmt);
                            } else {
                                log::debug!("Skipping duplicate attribute assignment: {key}");
                            }
                        }
                    } else {
                        // Target is not a simple name, include it
                        result.push(stmt);
                    }
                } else {
                    // Multiple targets, include it
                    result.push(stmt);
                }
            }
            _ => result.push(stmt),
        }
    }

    result
}

/// Check if an import from statement is a duplicate
pub(super) fn is_duplicate_import_from(
    bundler: &Bundler,
    import_from: &StmtImportFrom,
    existing_body: &[Stmt],
    python_version: u8,
) -> bool {
    if let Some(ref module) = import_from.module {
        let module_name = module.as_str();
        // For third-party imports, check if they're already in the body
        // Check if it's a stdlib module
        let root_module = module_name.split('.').next().unwrap_or(module_name);
        let is_stdlib =
            ruff_python_stdlib::sys::is_known_standard_library(python_version, root_module);
        let is_third_party = !is_stdlib && !is_bundled_module_or_package(bundler, module_name);

        if is_third_party {
            return existing_body.iter().any(|existing| {
                if let Stmt::ImportFrom(existing_import) = existing {
                    existing_import
                        .module
                        .as_ref()
                        .map(ruff_python_ast::Identifier::as_str)
                        == Some(module_name)
                        && import_names_match(&import_from.names, &existing_import.names)
                } else {
                    false
                }
            });
        }
    }
    false
}

/// Check if an import statement is a duplicate
pub(super) fn is_duplicate_import(
    _bundler: &Bundler,
    import_stmt: &StmtImport,
    existing_body: &[Stmt],
) -> bool {
    import_stmt.names.iter().any(|alias| {
        existing_body.iter().any(|existing| {
            if let Stmt::Import(existing_import) = existing {
                existing_import.names.iter().any(|existing_alias| {
                    existing_alias.name == alias.name && existing_alias.asname == alias.asname
                })
            } else {
                false
            }
        })
    })
}

/// Check if two sets of import names match
pub(super) fn import_names_match(names1: &[Alias], names2: &[Alias]) -> bool {
    if names1.len() != names2.len() {
        return false;
    }
    // Check if all names match (order doesn't matter)
    names1.iter().all(|n1| {
        names2
            .iter()
            .any(|n2| n1.name == n2.name && n1.asname == n2.asname)
    })
}

/// Check if a module is bundled or is a package containing bundled modules
pub(super) fn is_bundled_module_or_package(bundler: &Bundler, module_name: &str) -> bool {
    // Direct check
    if bundler.bundled_modules.contains(module_name) {
        return true;
    }
    // Check if it's a package containing bundled modules
    // e.g., if "greetings.greeting" is bundled, then "greetings" is a package
    let package_prefix = format!("{module_name}.");
    bundler
        .bundled_modules
        .iter()
        .any(|bundled| bundled.starts_with(&package_prefix))
}

/// Trim unused imports from modules using dependency graph analysis
pub(super) fn trim_unused_imports_from_modules(
    modules: &[(String, ModModule, PathBuf, String)],
    graph: &DependencyGraph,
    tree_shaker: Option<&TreeShaker>,
    python_version: u8,
) -> Vec<(String, ModModule, PathBuf, String)> {
    let mut trimmed_modules = Vec::new();

    for (module_name, ast, module_path, content_hash) in modules {
        log::debug!("Trimming unused imports from module: {module_name}");
        let mut ast = ast.clone(); // Clone here to allow mutation

        // Check if this is an __init__.py file
        let is_init_py =
            module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

        // Get unused imports from the graph
        if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
            // Check if this module has side effects (will become a wrapper module)
            let has_side_effects = !module_dep_graph.side_effect_items.is_empty();

            if has_side_effects {
                log::debug!(
                    "Module '{module_name}' has side effects - skipping stdlib import removal"
                );
            }

            let mut unused_imports =
                crate::analyzers::import_analyzer::ImportAnalyzer::find_unused_imports_in_module(
                    module_dep_graph,
                    is_init_py,
                );

            // If tree shaking is enabled, also check if imported symbols were removed
            // Note: We only apply tree-shaking logic to "from module import symbol" style
            // imports, not to "import module" style imports, since module
            // imports set up namespace objects
            if let Some(shaker) = tree_shaker {
                // Only apply tree-shaking-aware import removal if tree shaking is actually
                // enabled Get the symbols that survive tree-shaking for
                // this module
                let used_symbols = shaker.get_used_symbols_for_module(module_name);

                // Check each import to see if it's only used by tree-shaken code
                let import_items = module_dep_graph.get_all_import_items();
                log::debug!(
                    "Checking {} import items in module '{}' for tree-shaking",
                    import_items.len(),
                    module_name
                );
                for (item_id, import_item) in import_items {
                    match &import_item.item_type {
                        crate::cribo_graph::ItemType::FromImport {
                            module: from_module,
                            names,
                            ..
                        } => {
                            // For from imports, check each imported name
                            for (imported_name, alias_opt) in names {
                                let local_name = alias_opt.as_ref().unwrap_or(imported_name);

                                // Skip if already marked as unused
                                if unused_imports.iter().any(|u| u.name == *local_name) {
                                    continue;
                                }

                                // Skip if this is a re-export (in __all__ or explicit
                                // re-export)
                                if import_item.reexported_names.contains(local_name)
                                    || module_dep_graph.is_in_all_export(local_name)
                                {
                                    log::debug!(
                                        "Skipping tree-shaking for re-exported import \
                                         '{local_name}' from '{from_module}'"
                                    );
                                    continue;
                                }

                                // Check if this imported symbol itself is marked as used by tree shaker
                                // This handles the case where the symbol is accessed via module attributes
                                // (e.g., yaml_module.OtherYAMLObject where OtherYAMLObject is from an import)
                                // Check both the local name (alias) and the original imported name
                                if shaker.is_symbol_used(module_name, local_name)
                                    || shaker.is_symbol_used(module_name, imported_name)
                                {
                                    log::debug!(
                                        "Skipping tree-shaking for import '{local_name}' from \
                                         '{from_module}' - symbol is marked as used"
                                    );
                                    continue;
                                }

                                // Check if this import is actually importing a submodule
                                // For example, "from mypackage import utils" where utils is
                                // mypackage.utils
                                let is_submodule_import = {
                                    let potential_submodule =
                                        format!("{from_module}.{imported_name}");
                                    // Check if this module exists in the graph
                                    graph.get_module_by_name(&potential_submodule).is_some()
                                };

                                // If this is a submodule import, check if the submodule has side
                                // effects or is otherwise needed
                                let submodule_needed = if is_submodule_import {
                                    let submodule_name = format!("{from_module}.{imported_name}");
                                    log::debug!(
                                        "Import '{local_name}' is a submodule import for \
                                         '{submodule_name}'"
                                    );
                                    // Check if the submodule has side effects or symbols that
                                    // survived Even if no
                                    // symbols survived, if it has side effects, we need to keep it
                                    let has_side_effects =
                                        shaker.module_has_side_effects(&submodule_name);
                                    let has_used_symbols = !shaker
                                        .get_used_symbols_for_module(&submodule_name)
                                        .is_empty();

                                    log::debug!(
                                        "Submodule '{submodule_name}' - has_side_effects: \
                                         {has_side_effects}, has_used_symbols: {has_used_symbols}"
                                    );

                                    has_side_effects || has_used_symbols
                                } else {
                                    false
                                };

                                // Check if this import is only used by symbols that were
                                // tree-shaken
                                let used_by_surviving_code = submodule_needed
                                    || is_import_used_by_surviving_symbols(
                                        &used_symbols,
                                        module_dep_graph,
                                        local_name,
                                    )
                                    || is_import_used_by_side_effect_code(
                                        shaker,
                                        module_name,
                                        module_dep_graph,
                                        local_name,
                                    );

                                if !used_by_surviving_code {
                                    // This import is not used by any surviving symbol or
                                    // module-level code
                                    log::debug!(
                                        "Import '{local_name}' from '{from_module}' is not used \
                                         by surviving code after tree-shaking"
                                    );
                                    unused_imports.push(
                                        crate::analyzers::types::UnusedImportInfo {
                                            name: local_name.clone(),
                                            module: from_module.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        crate::cribo_graph::ItemType::Import { module, .. } => {
                            // For regular imports (import module), check if they're only used
                            // by tree-shaken code
                            let import_name = module.split('.').next_back().unwrap_or(module);

                            log::debug!(
                                "Checking module import '{import_name}' (full: '{module}') for \
                                 tree-shaking"
                            );

                            // Skip if already marked as unused
                            if unused_imports.iter().any(|u| u.name == *import_name) {
                                continue;
                            }

                            // Skip if this is a re-export
                            if import_item.reexported_names.contains(import_name)
                                || module_dep_graph.is_in_all_export(import_name)
                            {
                                log::debug!(
                                    "Skipping tree-shaking for re-exported import '{import_name}'"
                                );
                                continue;
                            }

                            // Check if this import is only used by symbols that were
                            // tree-shaken
                            log::debug!(
                                "Checking if any of {} surviving symbols use import \
                                 '{import_name}'",
                                used_symbols.len()
                            );
                            let mut used_by_surviving_code = is_import_used_by_surviving_symbols(
                                &used_symbols,
                                module_dep_graph,
                                import_name,
                            );

                            // Also check if any module-level code that has side effects uses it
                            if !used_by_surviving_code {
                                log::debug!(
                                    "No surviving symbols use '{import_name}', checking \
                                     module-level side effects"
                                );
                                used_by_surviving_code = is_module_import_used_by_side_effects(
                                    module_dep_graph,
                                    import_name,
                                );
                            }

                            // Special case: Check if this import is only used by assignment
                            // statements that were removed by tree-shaking
                            if !used_by_surviving_code {
                                used_by_surviving_code = is_import_used_by_surviving_assignments(
                                    module_dep_graph,
                                    import_name,
                                    &used_symbols,
                                );
                            }

                            if !used_by_surviving_code {
                                log::debug!(
                                    "Import '{import_name}' from module '{module}' is not used by \
                                     surviving code after tree-shaking (item_id: {item_id:?})"
                                );
                                unused_imports.push(crate::analyzers::types::UnusedImportInfo {
                                    name: import_name.to_string(),
                                    module: module.clone(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            if !unused_imports.is_empty() {
                // If this is a wrapper module (has side effects), filter out stdlib imports
                // from the unused list since they should be preserved as part of the module's API
                if has_side_effects {
                    let original_count = unused_imports.len();
                    unused_imports.retain(|import_info| {
                        // Check if this is a stdlib import
                        let root_module = import_info
                            .module
                            .split('.')
                            .next()
                            .unwrap_or(&import_info.module);
                        let is_stdlib = ruff_python_stdlib::sys::is_known_standard_library(
                            python_version,
                            root_module,
                        );

                        if is_stdlib {
                            log::debug!(
                                "Preserving stdlib import '{}' from '{}' in wrapper module",
                                import_info.name,
                                import_info.module
                            );
                            false // Remove from unused list (preserve the import)
                        } else {
                            true // Keep in unused list (will be removed)
                        }
                    });

                    if original_count != unused_imports.len() {
                        log::debug!(
                            "Filtered {} stdlib imports from unused list for wrapper module '{}'",
                            original_count - unused_imports.len(),
                            module_name
                        );
                    }
                }

                if !unused_imports.is_empty() {
                    log::debug!(
                        "Found {} unused imports in {}",
                        unused_imports.len(),
                        module_name
                    );
                    // Log unused imports details
                    log_unused_imports_details(&unused_imports);

                    // Filter out unused imports from the AST
                    ast.body
                        .retain(|stmt| !should_remove_import_stmt(stmt, &unused_imports));
                }
            }
        }

        trimmed_modules.push((
            module_name.clone(),
            ast,
            module_path.clone(),
            content_hash.clone(),
        ));
    }

    log::debug!(
        "Successfully trimmed unused imports from {} modules",
        trimmed_modules.len()
    );
    trimmed_modules
}

/// Check if an import is used by any surviving symbol after tree-shaking
fn is_import_used_by_surviving_symbols(
    used_symbols: &FxIndexSet<String>,
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    local_name: &str,
) -> bool {
    used_symbols
        .iter()
        .any(|symbol| module_dep_graph.does_symbol_use_import(symbol, local_name))
}

/// Check if an import is used by module-level code with side effects
fn is_import_used_by_side_effect_code(
    shaker: &TreeShaker,
    module_name: &str,
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    local_name: &str,
) -> bool {
    if !shaker.module_has_side_effects(module_name) {
        return false;
    }

    module_dep_graph.items.values().any(|item| {
        matches!(
            item.item_type,
            crate::cribo_graph::ItemType::Expression
                | crate::cribo_graph::ItemType::Assignment { .. }
        ) && item.read_vars.contains(local_name)
    })
}

/// Check if a module import is used by surviving code in a module with side effects
fn is_module_import_used_by_side_effects(
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    import_name: &str,
) -> bool {
    module_dep_graph.items.values().any(|item| {
        item.has_side_effects
            && !matches!(
                item.item_type,
                crate::cribo_graph::ItemType::Import { .. }
                    | crate::cribo_graph::ItemType::FromImport { .. }
            )
            && (item.read_vars.contains(import_name)
                || item.eventual_read_vars.contains(import_name))
    })
}

/// Check if an import is used by surviving assignment statements
fn is_import_used_by_surviving_assignments(
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    import_name: &str,
    used_symbols: &FxIndexSet<String>,
) -> bool {
    module_dep_graph.items.values().any(|item| {
        if let crate::cribo_graph::ItemType::Assignment { targets } = &item.item_type {
            item.read_vars.contains(import_name)
                && targets.iter().any(|target| used_symbols.contains(target))
        } else {
            false
        }
    })
}

/// Log details about unused imports for debugging
fn log_unused_imports_details(unused_imports: &[crate::analyzers::types::UnusedImportInfo]) {
    if log::log_enabled!(log::Level::Debug) {
        for unused in unused_imports {
            log::debug!("  - {} from {}", unused.name, unused.module);
        }
    }
}

/// Check if an import statement should be removed based on unused imports
fn should_remove_import_stmt(
    stmt: &Stmt,
    unused_imports: &[crate::analyzers::types::UnusedImportInfo],
) -> bool {
    match stmt {
        Stmt::Import(import_stmt) => {
            // Check if all names in this import are unused
            let should_remove = import_stmt.names.iter().all(|alias| {
                let local_name = alias
                    .asname
                    .as_ref()
                    .map_or(alias.name.as_str(), ruff_python_ast::Identifier::as_str);

                unused_imports.iter().any(|unused| {
                    log::trace!(
                        "Checking if import '{}' matches unused '{}' from '{}'",
                        local_name,
                        unused.name,
                        unused.module
                    );
                    // For regular imports, match by name only
                    unused.name == local_name
                })
            });

            if should_remove {
                log::debug!(
                    "Removing import statement: {:?}",
                    import_stmt
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                );
            }
            should_remove
        }
        Stmt::ImportFrom(import_from_stmt) => {
            // For from imports, we need to check if all imported names are unused
            let should_remove = import_from_stmt.names.iter().all(|alias| {
                let local_name = alias
                    .asname
                    .as_ref()
                    .map_or(alias.name.as_str(), ruff_python_ast::Identifier::as_str);

                unused_imports.iter().any(|unused| {
                    // Match by both name and module for from imports
                    unused.name == local_name
                })
            });

            if should_remove {
                log::debug!(
                    "Removing from import: from {} import {:?}",
                    import_from_stmt
                        .module
                        .as_ref()
                        .map_or("<None>", ruff_python_ast::Identifier::as_str),
                    import_from_stmt
                        .names
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                );
            }
            should_remove
        }
        _ => false,
    }
}
