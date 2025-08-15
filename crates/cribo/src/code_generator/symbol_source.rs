//! Utilities for finding the source module of imported symbols.

use std::path::Path;

use ruff_python_ast::{ModModule, Stmt, StmtImportFrom};

use crate::{resolver::ModuleResolver, types::FxIndexMap};

/// Finds which module a symbol was imported from in a wrapper module.
///
/// This function traces through import statements to find the original source
/// of a symbol, handling both direct imports and aliased imports.
///
/// # Arguments
/// * `module_asts` - Map of module names to their ASTs and paths
/// * `resolver` - Module resolver for handling relative imports
/// * `module_registry` - Registry of wrapper modules
/// * `module_name` - The module to search in
/// * `symbol_name` - The symbol to find the source of
///
/// # Returns
/// * `Some((source_module, original_name))` if the symbol is imported
/// * `None` if the symbol is not found or is defined locally
pub fn find_symbol_source_from_wrapper_module(
    module_asts: &[(String, ModModule, std::path::PathBuf, String)],
    resolver: &ModuleResolver,
    module_registry: &FxIndexMap<String, String>,
    module_name: &str,
    symbol_name: &str,
) -> Option<(String, String)> {
    // Find the module's AST to check its imports
    let (_, ast, module_path, _) = module_asts
        .iter()
        .find(|(name, _, _, _)| name == module_name)?;

    // Check if this symbol is imported from another module
    for stmt in &ast.body {
        let Stmt::ImportFrom(import_from) = stmt else {
            continue;
        };

        let resolved_module = resolve_import_module(resolver, import_from, module_path)?;

        // Check if our symbol is in this import
        for alias in &import_from.names {
            // Check if this alias matches our symbol_name
            // alias.asname is the local name (if aliased), alias.name is the original
            let local_name = alias
                .asname
                .as_ref()
                .map_or_else(|| alias.name.as_str(), ruff_python_ast::Identifier::as_str);

            if local_name == symbol_name {
                // Check if the source module is a wrapper module
                if module_registry.contains_key(&resolved_module) {
                    // Return the immediate source from the wrapper module
                    return Some((resolved_module, alias.name.to_string()));
                }
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
fn resolve_import_module(
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
