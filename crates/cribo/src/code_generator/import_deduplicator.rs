//! Import deduplication and cleanup utilities
//!
//! This module contains functions for finding and removing duplicate or unused imports,
//! and other import-related cleanup tasks during the bundling process.

use std::path::PathBuf;

use anyhow::Result;
use ruff_python_ast::{Alias, Expr, ModModule, Stmt, StmtImport, StmtImportFrom};

use super::{bundler::Bundler, expression_handlers};
use crate::{
    cribo_graph::CriboGraph as DependencyGraph, side_effects::is_safe_stdlib_module,
    tree_shaking::TreeShaker, types::FxIndexSet,
};

/// Check if a statement uses importlib
pub(super) fn stmt_uses_importlib(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(expr_stmt) => expression_handlers::expr_uses_importlib(&expr_stmt.value),
        Stmt::Assign(assign) => expression_handlers::expr_uses_importlib(&assign.value),
        Stmt::AugAssign(aug_assign) => expression_handlers::expr_uses_importlib(&aug_assign.value),
        Stmt::AnnAssign(ann_assign) => ann_assign
            .value
            .as_ref()
            .is_some_and(|v| expression_handlers::expr_uses_importlib(v)),
        Stmt::FunctionDef(func_def) => func_def.body.iter().any(stmt_uses_importlib),
        Stmt::ClassDef(class_def) => class_def.body.iter().any(stmt_uses_importlib),
        Stmt::If(if_stmt) => {
            expression_handlers::expr_uses_importlib(&if_stmt.test)
                || if_stmt.body.iter().any(stmt_uses_importlib)
                || if_stmt.elif_else_clauses.iter().any(|clause| {
                    clause
                        .test
                        .as_ref()
                        .is_some_and(expression_handlers::expr_uses_importlib)
                        || clause.body.iter().any(stmt_uses_importlib)
                })
        }
        Stmt::While(while_stmt) => {
            expression_handlers::expr_uses_importlib(&while_stmt.test)
                || while_stmt.body.iter().any(stmt_uses_importlib)
                || while_stmt.orelse.iter().any(stmt_uses_importlib)
        }
        Stmt::For(for_stmt) => {
            expression_handlers::expr_uses_importlib(&for_stmt.iter)
                || for_stmt.body.iter().any(stmt_uses_importlib)
                || for_stmt.orelse.iter().any(stmt_uses_importlib)
        }
        Stmt::With(with_stmt) => {
            with_stmt.items.iter().any(|item| {
                expression_handlers::expr_uses_importlib(&item.context_expr)
                    || item
                        .optional_vars
                        .as_ref()
                        .is_some_and(|v| expression_handlers::expr_uses_importlib(v))
            }) || with_stmt.body.iter().any(stmt_uses_importlib)
        }
        Stmt::Try(try_stmt) => {
            try_stmt.body.iter().any(stmt_uses_importlib)
                || try_stmt.handlers.iter().any(|handler| match handler {
                    ruff_python_ast::ExceptHandler::ExceptHandler(eh) => {
                        eh.type_
                            .as_ref()
                            .is_some_and(|t| expression_handlers::expr_uses_importlib(t))
                            || eh.body.iter().any(stmt_uses_importlib)
                    }
                })
                || try_stmt.orelse.iter().any(stmt_uses_importlib)
                || try_stmt.finalbody.iter().any(stmt_uses_importlib)
        }
        Stmt::Assert(assert_stmt) => {
            expression_handlers::expr_uses_importlib(&assert_stmt.test)
                || assert_stmt
                    .msg
                    .as_ref()
                    .is_some_and(|v| expression_handlers::expr_uses_importlib(v))
        }
        Stmt::Return(ret) => ret
            .value
            .as_ref()
            .is_some_and(|v| expression_handlers::expr_uses_importlib(v)),
        Stmt::Raise(raise_stmt) => {
            raise_stmt
                .exc
                .as_ref()
                .is_some_and(|v| expression_handlers::expr_uses_importlib(v))
                || raise_stmt
                    .cause
                    .as_ref()
                    .is_some_and(|v| expression_handlers::expr_uses_importlib(v))
        }
        Stmt::Delete(del) => del
            .targets
            .iter()
            .any(expression_handlers::expr_uses_importlib),
        // Statements that don't contain expressions
        Stmt::Import(_)
        | Stmt::ImportFrom(_) // Already handled by import transformation
        | Stmt::Pass(_)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Global(_)
        | Stmt::Nonlocal(_)
        | Stmt::IpyEscapeCommand(_) => false, // IPython specific, unlikely to use importlib
        // Match and TypeAlias need special handling
        Stmt::Match(match_stmt) => {
            expression_handlers::expr_uses_importlib(&match_stmt.subject)
                || match_stmt
                    .cases
                    .iter()
                    .any(|case| case.body.iter().any(stmt_uses_importlib))
        }
        Stmt::TypeAlias(type_alias) => expression_handlers::expr_uses_importlib(&type_alias.value),
    }
}

/// Check if a statement is a hoisted import
pub(super) fn is_hoisted_import(bundler: &Bundler, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ImportFrom(import_from) => {
            if let Some(ref module) = import_from.module {
                let module_name = module.as_str();
                // Check if this is a __future__ import (always hoisted)
                if module_name == "__future__" {
                    return true;
                }
                // Check if this is a stdlib import that we've hoisted
                if crate::side_effects::is_safe_stdlib_module(module_name) {
                    // Check if this exact import is in our hoisted stdlib imports
                    return is_import_in_hoisted_stdlib(bundler, module_name);
                }
                // We no longer hoist third-party imports, so they should never be considered
                // hoisted Only stdlib and __future__ imports are hoisted
            }
            false
        }
        Stmt::Import(import_stmt) => {
            // Check if any of the imported modules are hoisted (stdlib or third-party)
            import_stmt.names.iter().any(|alias| {
                let module_name = alias.name.as_str();
                // Check stdlib imports
                if crate::side_effects::is_safe_stdlib_module(module_name) {
                    bundler.stdlib_import_statements.iter().any(|hoisted| {
                        matches!(hoisted, Stmt::Import(hoisted_import)
                            if hoisted_import.names.iter().any(|h| h.name == alias.name))
                    })
                }
                // We no longer hoist third-party imports
                else {
                    false
                }
            })
        }
        _ => false,
    }
}

/// Check if a specific module is in our hoisted stdlib imports
pub(super) fn is_import_in_hoisted_stdlib(bundler: &Bundler, module_name: &str) -> bool {
    // Check if module is in our from imports map
    if bundler.stdlib_import_from_map.contains_key(module_name) {
        return true;
    }

    // Check if module is in our regular import statements
    bundler.stdlib_import_statements.iter().any(|hoisted| {
        matches!(hoisted, Stmt::Import(hoisted_import)
            if hoisted_import.names.iter().any(|alias| alias.name.as_str() == module_name))
    })
}

/// Add a regular stdlib import (e.g., "sys", "types")
/// This creates an import statement and adds it to the tracked imports
pub(super) fn add_stdlib_import(bundler: &mut Bundler, module_name: &str) {
    // Check if we already have this import to avoid duplicates
    let already_imported = bundler.stdlib_import_statements.iter().any(|stmt| {
        if let Stmt::Import(import_stmt) = stmt {
            import_stmt
                .names
                .iter()
                .any(|alias| alias.name.as_str() == module_name)
        } else {
            false
        }
    });

    if already_imported {
        log::debug!("Stdlib import '{module_name}' already exists, skipping");
        return;
    }

    let import_stmt =
        crate::ast_builder::statements::import(vec![crate::ast_builder::other::alias(
            module_name,
            None,
        )]);
    bundler.stdlib_import_statements.push(import_stmt);
}

/// Collect imports from a module for hoisting
pub(super) fn collect_imports_from_module(
    bundler: &mut Bundler,
    ast: &ModModule,
    module_name: &str,
) {
    log::debug!("Collecting imports from module: {module_name}");
    for stmt in &ast.body {
        match stmt {
            Stmt::ImportFrom(import_from) => {
                if let Some(module) = &import_from.module {
                    let module_str = module.as_str();

                    // Check if this is a safe stdlib module, skipping __future__ imports.
                    if module_str != "__future__" && is_safe_stdlib_module(module_str) {
                        let import_map = bundler
                            .stdlib_import_from_map
                            .entry(module_str.to_string())
                            .or_default();

                        for alias in &import_from.names {
                            let name = alias.name.as_str();
                            let alias_name = alias.asname.as_ref().map(|a| a.as_str().to_string());
                            import_map.insert(name.to_string(), alias_name);
                        }
                    }
                }
            }
            Stmt::Import(import_stmt) => {
                // Track regular import statements for stdlib modules
                if import_stmt.names.iter().any(|alias| {
                    let imported_module_name = alias.name.as_str();
                    is_safe_stdlib_module(imported_module_name) && alias.asname.is_none()
                }) {
                    bundler.stdlib_import_statements.push(stmt.clone());
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

    // Then stdlib from imports - deduplicated and sorted by module name
    let mut sorted_modules: Vec<_> = bundler.stdlib_import_from_map.iter().collect();
    sorted_modules.sort_by_key(|(module_name, _)| *module_name);

    for (module_name, imported_names) in sorted_modules {
        // Sort the imported names for deterministic output
        let mut sorted_names: Vec<(String, Option<String>)> = imported_names
            .iter()
            .map(|(name, alias)| (name.clone(), alias.clone()))
            .collect();
        sorted_names.sort_by_key(|(name, _)| name.clone());

        let aliases: Vec<Alias> = sorted_names
            .into_iter()
            .map(|(name, alias_opt)| other::alias(&name, alias_opt.as_deref()))
            .collect();

        final_body.push(statements::import_from(Some(module_name), aliases, 0));
    }

    // IMPORTANT: Only safe stdlib imports are hoisted to the bundle top level.
    // Third-party imports are NEVER hoisted because they may have side effects
    // (e.g., registering plugins, modifying global state, network calls).
    // Third-party imports remain in their original location to preserve execution order.

    // Regular stdlib import statements - deduplicated and sorted by module name
    let mut seen_modules = crate::types::FxIndexSet::default();
    let mut unique_imports = Vec::new();

    for stmt in &bundler.stdlib_import_statements {
        if let Stmt::Import(import_stmt) = stmt {
            collect_unique_imports_for_hoisting(
                import_stmt,
                &mut seen_modules,
                &mut unique_imports,
            );
        }
    }

    // Sort by module name for deterministic output
    unique_imports.sort_by_key(|(module_name, _)| module_name.clone());

    for (_, import_stmt) in unique_imports {
        final_body.push(import_stmt);
    }

    // NOTE: We do NOT hoist third-party regular import statements for the same reason
    // as above - they may have side effects and should remain in their original context.
}

/// Collect unique imports from an import statement for hoisting
fn collect_unique_imports_for_hoisting(
    import_stmt: &StmtImport,
    seen_modules: &mut crate::types::FxIndexSet<String>,
    unique_imports: &mut Vec<(String, Stmt)>,
) {
    for alias in &import_stmt.names {
        let module_name = alias.name.as_str();
        if seen_modules.contains(module_name) {
            continue;
        }
        seen_modules.insert(module_name.to_string());
        // Create import statement preserving the original alias
        unique_imports.push((
            module_name.to_string(),
            crate::ast_builder::statements::import(vec![crate::ast_builder::other::alias(
                module_name,
                alias
                    .asname
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str),
            )]),
        ));
    }
}

/// Remove unused importlib references from a module
pub(super) fn remove_unused_importlib(ast: &mut ModModule) {
    // Check if importlib is actually used in the code
    let mut importlib_used = false;
    for stmt in &ast.body {
        if stmt_uses_importlib(stmt) {
            importlib_used = true;
            break;
        }
    }

    if !importlib_used {
        log::debug!("importlib is unused after transformation, removing import");
        ast.body.retain(|stmt| match stmt {
            Stmt::Import(import_stmt) => !import_stmt
                .names
                .iter()
                .any(|alias| alias.name.as_str() == "importlib"),
            Stmt::ImportFrom(import_from_stmt) => import_from_stmt
                .module
                .as_ref()
                .is_none_or(|m| m.as_str() != "importlib"),
            _ => true,
        });
    }
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
                    if crate::code_generator::module_registry::is_init_function(func_name) {
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
                        if crate::code_generator::module_registry::is_init_function(func_name) {
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
                        if crate::code_generator::module_registry::is_init_function(func_name) {
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
) -> bool {
    if let Some(ref module) = import_from.module {
        let module_name = module.as_str();
        // For third-party imports, check if they're already in the body
        let is_third_party = !is_safe_stdlib_module(module_name)
            && !is_bundled_module_or_package(bundler, module_name);

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
) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
    let mut trimmed_modules = Vec::new();

    for (module_name, ast, module_path, content_hash) in modules {
        log::debug!("Trimming unused imports from module: {module_name}");
        let mut ast = ast.clone(); // Clone here to allow mutation

        // Check if this is an __init__.py file
        let is_init_py =
            module_path.file_name().and_then(|name| name.to_str()) == Some("__init__.py");

        // Get unused imports from the graph
        if let Some(module_dep_graph) = graph.get_module_by_name(module_name) {
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

                                // Check if this import is only used by symbols that were
                                // tree-shaken
                                let used_by_surviving_code = is_import_used_by_surviving_symbols(
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

                            // Extra check for normalized imports: If this is a normalized
                            // import and no assignments using
                            // it survived, it should be removed
                            if import_item.is_normalized_import {
                                log::debug!(
                                    "Import '{import_name}' is a normalized import \
                                     (used_by_surviving_code: {used_by_surviving_code})"
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
    Ok(trimmed_modules)
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
