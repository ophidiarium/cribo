//! Import deduplication and cleanup utilities
//!
//! This module contains functions for finding and removing duplicate or unused imports,
//! and other import-related cleanup tasks during the bundling process.

use std::path::PathBuf;

use anyhow::Result;
use ruff_python_ast::{Alias, Expr, ModModule, Stmt, StmtImport, StmtImportFrom};

use super::{bundler::HybridStaticBundler, expression_handlers};
use crate::{
    cribo_graph::CriboGraph as DependencyGraph, tree_shaking::TreeShaker, types::FxIndexSet,
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
        Stmt::Import(_) | Stmt::ImportFrom(_) => false, /* Already handled by import */
        // transformation
        Stmt::Pass(_) | Stmt::Break(_) | Stmt::Continue(_) => false,
        Stmt::Global(_) | Stmt::Nonlocal(_) => false,
        // Match and TypeAlias need special handling
        Stmt::Match(match_stmt) => {
            expression_handlers::expr_uses_importlib(&match_stmt.subject)
                || match_stmt
                    .cases
                    .iter()
                    .any(|case| case.body.iter().any(stmt_uses_importlib))
        }
        Stmt::TypeAlias(type_alias) => expression_handlers::expr_uses_importlib(&type_alias.value),
        Stmt::IpyEscapeCommand(_) => false, // IPython specific, unlikely to use importlib
    }
}

/// Check if a statement is a hoisted import
pub(super) fn is_hoisted_import(bundler: &HybridStaticBundler, stmt: &Stmt) -> bool {
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
pub(super) fn is_import_in_hoisted_stdlib(
    bundler: &HybridStaticBundler,
    module_name: &str,
) -> bool {
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
pub(super) fn add_stdlib_import(bundler: &mut HybridStaticBundler, module_name: &str) {
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

/// Add hoisted imports to the final body
pub(super) fn add_hoisted_imports(bundler: &HybridStaticBundler, final_body: &mut Vec<Stmt>) {
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
        // Skip importlib if it was fully transformed
        if module_name == "importlib" && bundler.importlib_fully_transformed {
            log::debug!("Skipping importlib from hoisted imports as it was fully transformed");
            continue;
        }

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
                bundler,
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
    _bundler: &HybridStaticBundler,
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
            Stmt::Import(StmtImport {
                node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                names: vec![Alias {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    name: ruff_python_ast::Identifier::new(
                        module_name,
                        ruff_text_size::TextRange::default(),
                    ),
                    asname: alias.asname.clone(),
                    range: ruff_text_size::TextRange::default(),
                }],
                range: ruff_text_size::TextRange::default(),
            }),
        ));
    }
}

/// Remove unused importlib references from a module
pub(super) fn remove_unused_importlib(_bundler: &HybridStaticBundler, ast: &mut ModModule) {
    ast.body.retain(|stmt| !stmt_uses_importlib(stmt));
    log::debug!("Removed unused importlib references from module");
}

/// Deduplicate deferred imports against existing body statements
pub(super) fn deduplicate_deferred_imports_with_existing(
    bundler: &HybridStaticBundler,
    imports: Vec<Stmt>,
    existing_body: &[Stmt],
) -> Vec<Stmt> {
    let mut deduplicated = Vec::new();

    for import_stmt in imports {
        let is_duplicate = match &import_stmt {
            Stmt::ImportFrom(import_from) => {
                is_duplicate_import_from(bundler, import_from, existing_body)
            }
            Stmt::Import(import) => is_duplicate_import(bundler, import, existing_body),
            Stmt::Assign(_) => {
                // For assignment statements, check if there's an equivalent assignment
                existing_body.iter().any(|existing_stmt| {
                    if let (Stmt::Assign(new_assign), Stmt::Assign(existing_assign)) =
                        (&import_stmt, existing_stmt)
                    {
                        // Check if the assignments are the same
                        if new_assign.targets.len() == 1
                            && existing_assign.targets.len() == 1
                            && let (Expr::Name(new_target), Expr::Name(existing_target)) =
                                (&new_assign.targets[0], &existing_assign.targets[0])
                        {
                            // Check if the targets are the same
                            existing_target.id == new_target.id
                                && expression_handlers::expr_equals(
                                    &existing_assign.value,
                                    &new_assign.value,
                                )
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                })
            }
            _ => false,
        };

        if !is_duplicate {
            deduplicated.push(import_stmt);
        } else {
            log::debug!("Deduplicated import: {import_stmt:?}");
        }
    }

    deduplicated
}

/// Check if an import from statement is a duplicate
pub(super) fn is_duplicate_import_from(
    _bundler: &HybridStaticBundler,
    import_from: &StmtImportFrom,
    existing_body: &[Stmt],
) -> bool {
    existing_body.iter().any(|stmt| {
        if let Stmt::ImportFrom(existing_import) = stmt {
            existing_import.module == import_from.module
                && existing_import.level == import_from.level
                && import_names_match(&existing_import.names, &import_from.names)
        } else {
            false
        }
    })
}

/// Check if an import statement is a duplicate
pub(super) fn is_duplicate_import(
    _bundler: &HybridStaticBundler,
    import_stmt: &StmtImport,
    existing_body: &[Stmt],
) -> bool {
    existing_body.iter().any(|stmt| {
        if let Stmt::Import(existing_import) = stmt {
            import_names_match(&existing_import.names, &import_stmt.names)
        } else {
            false
        }
    })
}

/// Check if two sets of import names match
pub(super) fn import_names_match(names1: &[Alias], names2: &[Alias]) -> bool {
    if names1.len() != names2.len() {
        return false;
    }

    names1.iter().all(|alias1| {
        names2
            .iter()
            .any(|alias2| alias1.name == alias2.name && alias1.asname == alias2.asname)
    })
}

/// Check if an import statement should be removed based on unused imports analysis
pub(super) fn should_remove_import_stmt(
    _bundler: &HybridStaticBundler,
    stmt: &Stmt,
    unused_imports: &[crate::analyzers::types::UnusedImportInfo],
) -> bool {
    match stmt {
        Stmt::Import(import_stmt) => {
            // Check if all imported names are unused
            import_stmt.names.iter().all(|alias| {
                let import_name = alias.asname.as_ref().unwrap_or(&alias.name);
                unused_imports
                    .iter()
                    .any(|unused| unused.name == import_name.as_str())
            })
        }
        Stmt::ImportFrom(import_from_stmt) => {
            // Check if all imported names are unused
            import_from_stmt.names.iter().all(|alias| {
                let import_name = alias.asname.as_ref().unwrap_or(&alias.name);
                unused_imports
                    .iter()
                    .any(|unused| unused.name == import_name.as_str())
            })
        }
        _ => false,
    }
}

/// Log details about unused imports for debugging
pub(super) fn log_unused_imports_details(
    unused_imports: &[crate::analyzers::types::UnusedImportInfo],
) {
    for unused in unused_imports {
        log::debug!(
            "Unused import: {} (module: {:?})",
            unused.name,
            unused.module
        );
    }
}

/// Trim unused imports from modules using dependency graph analysis
pub(super) fn trim_unused_imports_from_modules(
    bundler: &mut HybridStaticBundler,
    modules: &[(String, ModModule, PathBuf, String)],
    graph: &DependencyGraph,
    _tree_shaker: Option<&TreeShaker>,
) -> Result<Vec<(String, ModModule, PathBuf, String)>> {
    let mut result = Vec::new();

    for (module_name, module_ast, module_path, module_content) in modules.iter() {
        let mut module_ast = module_ast.clone();
        // Get unused imports from the dependency graph
        let module_dep_graph = graph.get_module_by_name(module_name).ok_or_else(|| {
            anyhow::anyhow!("Module {} not found in dependency graph", module_name)
        })?;
        let unused_imports = crate::analyzers::ImportAnalyzer::find_unused_imports_in_module(
            module_dep_graph,
            module_path.file_name().unwrap_or_default() == "__init__.py",
        );

        if !unused_imports.is_empty() {
            log_unused_imports_details(&unused_imports);

            // Remove unused import statements
            module_ast
                .body
                .retain(|stmt| !should_remove_import_stmt(bundler, stmt, &unused_imports));

            log::debug!(
                "Removed {} unused imports from module '{}'",
                unused_imports.len(),
                module_name
            );
        }

        // Remove unused importlib if present
        remove_unused_importlib(bundler, &mut module_ast);

        result.push((
            module_name.clone(),
            module_ast,
            module_path.clone(),
            module_content.clone(),
        ));
    }

    Ok(result)
}

/// Collect unique imports from a list of statements
pub(super) fn collect_unique_imports(
    _bundler: &HybridStaticBundler,
    statements: &[Stmt],
) -> FxIndexSet<String> {
    let mut imports = FxIndexSet::default();

    for stmt in statements {
        match stmt {
            Stmt::Import(import_stmt) => {
                for alias in &import_stmt.names {
                    imports.insert(alias.name.to_string());
                }
            }
            Stmt::ImportFrom(import_from_stmt) => {
                if let Some(ref module) = import_from_stmt.module {
                    imports.insert(module.to_string());
                }
            }
            _ => {}
        }
    }

    imports
}
