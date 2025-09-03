//! Utilities for finding the source module of imported symbols.

use std::path::Path;

use ruff_python_ast::{ModModule, Stmt, StmtImportFrom};

use crate::{
    resolver::{ModuleId, ModuleResolver},
    types::{FxIndexMap, FxIndexSet},
};

/// Finds which module a symbol was imported from in a wrapper module.
///
/// This function traces through import statements to find the original source
/// of a symbol, handling both direct imports and aliased imports.
///
/// # Arguments
/// * `module_asts` - Map of `ModuleId` to (ast, `module_path`, `content_hash`)
/// * `resolver` - Module resolver for handling relative imports
/// * `wrapper_modules` - Set of `ModuleIds` that are wrapper modules
/// * `module_name` - The module to search in
/// * `symbol_name` - The symbol to find the source of
///
/// # Returns
/// * `Some((source_module, original_name))` if the symbol is imported from a wrapper module
/// * `None` if the symbol is not found, is defined locally, or is imported from a non-wrapper
///   module
pub fn find_symbol_source_from_wrapper_module(
    module_asts: &FxIndexMap<ModuleId, (ModModule, std::path::PathBuf, String)>,
    resolver: &ModuleResolver,
    wrapper_modules: &FxIndexSet<ModuleId>,
    module_name: &str,
    symbol_name: &str,
) -> Option<(String, String)> {
    log::trace!(
        "find_symbol_source_from_wrapper_module: looking for symbol '{symbol_name}' in module \
         '{module_name}'"
    );

    // Find the module's AST to check its imports
    let module_id = resolver.get_module_id_by_name(module_name)?;
    let (ast, module_path, _) = module_asts.get(&module_id)?;

    // Check if this symbol is imported from another module (including nested scopes)
    for import_from in collect_import_from_statements_in_module(ast) {
        let Some(resolved_module) = resolve_import_module(resolver, import_from, module_path)
        else {
            // Unresolvable import — skip and continue scanning remaining imports.
            continue;
        };

        // Check if our symbol is in this import
        for alias in &import_from.names {
            // Skip wildcard imports ("from ... import *"): wildcard expansion is
            // handled in the bundler's second pass (see bundler.rs) and should not
            // be matched here.
            if alias.name.as_str() == "*" {
                continue;
            }

            // Check if this alias matches our symbol_name
            // alias.asname is the local name (if aliased), alias.name is the original
            let local_name = alias
                .asname
                .as_ref()
                .map_or_else(|| alias.name.as_str(), ruff_python_ast::Identifier::as_str);

            if local_name == symbol_name {
                log::debug!(
                    "Found import: '{local_name}' (original: '{}') from module '{resolved_module}'",
                    alias.name.as_str()
                );

                // Check if the source module is a wrapper module
                if let Some(resolved_id) = resolver.get_module_id_by_name(&resolved_module)
                    && wrapper_modules.contains(&resolved_id)
                {
                    log::debug!(
                        "Source module '{resolved_module}' is a wrapper module - returning \
                         ({resolved_module}, {})",
                        alias.name.as_str()
                    );
                    // Return the immediate source from the wrapper module
                    return Some((resolved_module, alias.name.to_string()));
                }
                log::trace!("Source module '{resolved_module}' is NOT a wrapper module - skipping");
                // For non-wrapper modules, don't return anything (original behavior)
                break;
            }
        }
    }

    None
}

/// Resolves an import statement to an absolute module name.
///
/// Handles both relative and absolute imports.
pub fn resolve_import_module(
    resolver: &ModuleResolver,
    import_from: &StmtImportFrom,
    module_path: &Path,
) -> Option<String> {
    if import_from.level > 0 {
        resolver.resolve_relative_to_absolute_module_name(
            import_from.level,
            import_from
                .module
                .as_ref()
                .map(ruff_python_ast::Identifier::as_str),
            module_path,
        )
    } else {
        import_from.module.as_ref().map(|m| m.as_str().to_string())
    }
}

/// Collects all `ImportFrom` statements in a module, including those in nested scopes.
///
/// This function recursively traverses the AST to find imports inside functions,
/// classes, conditionals, and other nested structures.
fn collect_import_from_statements_in_module(ast: &ModModule) -> Vec<&StmtImportFrom> {
    let mut imports = Vec::new();
    for stmt in &ast.body {
        collect_import_from_statements(stmt, &mut imports);
    }
    imports
}

/// Recursively collects `ImportFrom` statements from a statement and its children.
fn collect_import_from_statements<'a>(stmt: &'a Stmt, acc: &mut Vec<&'a StmtImportFrom>) {
    use ruff_python_ast as ast;

    match stmt {
        Stmt::ImportFrom(import_from) => acc.push(import_from),
        Stmt::FunctionDef(f) => {
            // Functions (both sync and async) can contain imports
            for s in &f.body {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::ClassDef(c) => {
            // Classes can contain imports in their body
            for s in &c.body {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::If(if_stmt) => {
            // Process if body
            for s in &if_stmt.body {
                collect_import_from_statements(s, acc);
            }
            // Process elif and else clauses
            for clause in &if_stmt.elif_else_clauses {
                for s in &clause.body {
                    collect_import_from_statements(s, acc);
                }
            }
        }
        Stmt::While(while_stmt) => {
            for s in &while_stmt.body {
                collect_import_from_statements(s, acc);
            }
            for s in &while_stmt.orelse {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::For(for_stmt) => {
            for s in &for_stmt.body {
                collect_import_from_statements(s, acc);
            }
            for s in &for_stmt.orelse {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::Try(try_stmt) => {
            // Process try body
            for s in &try_stmt.body {
                collect_import_from_statements(s, acc);
            }
            // Process except handlers
            for handler in &try_stmt.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = handler;
                for s in &h.body {
                    collect_import_from_statements(s, acc);
                }
            }
            // Process else clause
            for s in &try_stmt.orelse {
                collect_import_from_statements(s, acc);
            }
            // Process finally clause
            for s in &try_stmt.finalbody {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::With(with_stmt) => {
            for s in &with_stmt.body {
                collect_import_from_statements(s, acc);
            }
        }
        Stmt::Match(match_stmt) => {
            // Process match case bodies
            for case in &match_stmt.cases {
                for s in &case.body {
                    collect_import_from_statements(s, acc);
                }
            }
        }
        _ => {
            // Other statement types don't contain nested statements with imports
        }
    }
}
