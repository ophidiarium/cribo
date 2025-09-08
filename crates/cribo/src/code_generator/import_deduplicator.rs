//! Import deduplication and cleanup utilities
//!
//! This module contains functions for finding and removing duplicate or unused imports,
//! and other import-related cleanup tasks during the bundling process.

use std::path::PathBuf;

use ruff_python_ast::{Alias, ModModule, Stmt, StmtImport, StmtImportFrom};

use super::bundler::Bundler;
use crate::{
    cribo_graph::CriboGraph as DependencyGraph,
    types::{FxIndexMap, FxIndexSet},
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
    // Direct check - convert module_name to ModuleId for lookup
    if bundler
        .get_module_id(module_name)
        .is_some_and(|id| bundler.bundled_modules.contains(&id))
    {
        return true;
    }
    // Check if it's a package containing bundled modules
    // e.g., if "greetings.greeting" is bundled, then "greetings" is a package
    let package_prefix = format!("{module_name}.");
    bundler.bundled_modules.iter().any(|bundled_id| {
        bundler
            .resolver
            .get_module_name(*bundled_id)
            .is_some_and(|name| name.starts_with(&package_prefix))
    })
}

/// Trim unused imports from modules using dependency graph analysis
pub(super) fn trim_unused_imports_from_modules(
    modules: &FxIndexMap<crate::resolver::ModuleId, (ModModule, PathBuf, String)>,
    graph: &DependencyGraph,
    tree_shaker: Option<&crate::tree_shaking::TreeShaker<'_>>,
    python_version: u8,
    circular_modules: &FxIndexSet<crate::resolver::ModuleId>,
) -> FxIndexMap<crate::resolver::ModuleId, (ModModule, PathBuf, String)> {
    let mut trimmed_modules = FxIndexMap::default();

    for (module_id, (ast, module_path, content_hash)) in modules {
        log::debug!("Trimming unused imports from module: {module_id:?}");
        let mut ast = ast.clone(); // Clone here to allow mutation

        // Check if this is an __init__.py file
        let is_init_py = module_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(crate::python::module_path::is_init_file_name);

        // Get unused imports from the graph
        if let Some(module_dep_graph) = graph.get_module(*module_id) {
            // Check if this module has side effects (will become a wrapper module)
            let has_side_effects = !module_dep_graph.side_effect_items.is_empty();

            if has_side_effects {
                log::debug!(
                    "Module {module_id:?} has side effects - skipping stdlib import removal"
                );
            }

            let mut unused_imports =
                crate::analyzers::import_analyzer::ImportAnalyzer::find_unused_imports_in_module(
                    module_dep_graph,
                    is_init_py,
                );

            // Skip tree-shaking based import removal for circular modules
            // Circular modules become init functions that include ALL their original code,
            // even the parts that would be tree-shaken, so we need to keep all imports
            let is_circular_module = circular_modules.contains(module_id);
            log::debug!(
                "Module {module_id:?} - checking if circular: {is_circular_module}, \
                 circular_modules: {circular_modules:?}"
            );
            if is_circular_module {
                log::debug!(
                    "Module {module_id:?} is circular - skipping tree-shaking based import removal"
                );
                // For circular modules, also preserve imports that are module-level symbols
                // These can be accessed from other modules as module attributes even if not
                // directly used within this module A module-level symbol is one
                // that is either:
                // 1. Imported at module level (not inside a function/class)
                // 2. In __all__ export list
                // 3. Explicitly re-exported
                let original_count = unused_imports.len();
                unused_imports.retain(|import_info| {
                    // Check if this symbol is in __all__ or explicitly re-exported
                    let is_in_all = module_dep_graph.is_in_all_export(&import_info.name);

                    // Check if any import item has this as a reexported name
                    let is_reexported = module_dep_graph
                        .items
                        .values()
                        .any(|item| item.reexported_names.contains(&import_info.name));

                    // Check if this import is at module level (any import item that imports this
                    // name) In circular modules, all imports at module level
                    // become module attributes in the init function
                    let is_module_level_import = module_dep_graph
                        .items
                        .values()
                        .any(|item| item.imported_names.contains(&import_info.name));

                    let should_preserve = is_in_all || is_reexported || is_module_level_import;

                    if should_preserve {
                        log::debug!(
                            "Preserving import '{}' in circular module - module-level import \
                             (in_all: {}, reexported: {}, module_level: {})",
                            import_info.name,
                            is_in_all,
                            is_reexported,
                            is_module_level_import
                        );
                    }

                    !should_preserve // Keep in unused list only if NOT to be preserved
                });

                if original_count != unused_imports.len() {
                    log::debug!(
                        "Filtered {} module-level imports from unused list in circular module",
                        original_count - unused_imports.len()
                    );
                }
            }

            // If tree shaking is enabled, also check if imported symbols were removed
            // Note: We only apply tree-shaking logic to "from module import symbol" style
            // imports, not to "import module" style imports, since module
            // imports set up namespace objects
            if let Some(shaker) = tree_shaker
                && !is_circular_module
            {
                // Only apply tree-shaking-aware import removal if tree shaking is actually
                // enabled Get the symbols that survive tree-shaking for
                // this module
                // TreeShaker still uses string-based module names, get it from the dep graph
                let module_name = &module_dep_graph.module_name;
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

                                // Check if this imported symbol itself is marked as used by tree
                                // shaker This handles the case
                                // where the symbol is accessed via module attributes
                                // (e.g., yaml_module.OtherYAMLObject where OtherYAMLObject is from
                                // an import) Check both the local
                                // name (alias) and the original imported name
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
                                    let has_side_effects = graph
                                        .get_module_by_name(&submodule_name)
                                        .map(|m| m.module_id)
                                        .is_some_and(|id| shaker.module_has_side_effects(id));
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
                                let mut used_by_surviving_code = submodule_needed
                                    || is_import_used_by_surviving_symbols(
                                        &used_symbols,
                                        module_dep_graph,
                                        local_name,
                                    )
                                    || is_import_used_by_side_effect_code(
                                        shaker,
                                        module_dep_graph,
                                        local_name,
                                    );

                                // Also check if this import is used by symbols in __all__
                                // These symbols might be exported even if not directly used
                                if !used_by_surviving_code {
                                    log::debug!(
                                        "Checking if '{local_name}' is used by __all__ symbols"
                                    );
                                    used_by_surviving_code =
                                        is_import_used_by_all_exports(module_dep_graph, local_name);
                                }

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

                            // Check if the imported module itself has side effects and needs
                            // initialization This handles the case
                            // where a wrapper module with side effects is imported
                            // but not directly used (e.g., import mypackage where mypackage has
                            // print statements)
                            let module_has_side_effects = graph
                                .get_module_by_name(module)
                                .map(|m| m.module_id)
                                .is_some_and(|id| shaker.module_has_side_effects(id));
                            if module_has_side_effects {
                                log::debug!(
                                    "Module '{module}' has side effects - preserving import for \
                                     initialization"
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

                            // Also check if this import is used by symbols in __all__
                            // These symbols might be exported even if not directly used
                            if !used_by_surviving_code {
                                log::debug!(
                                    "Checking if '{import_name}' is used by __all__ symbols"
                                );
                                used_by_surviving_code =
                                    is_import_used_by_all_exports(module_dep_graph, import_name);
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
                            module_dep_graph.module_name
                        );
                    }
                }

                if !unused_imports.is_empty() {
                    log::debug!(
                        "Found {} unused imports in {}",
                        unused_imports.len(),
                        module_dep_graph.module_name
                    );
                    // Log unused imports details
                    log_unused_imports_details(&unused_imports);

                    // Filter out unused imports from the AST
                    ast.body
                        .retain(|stmt| !should_remove_import_stmt(stmt, &unused_imports));
                }
            }
        }

        trimmed_modules.insert(*module_id, (ast, module_path.clone(), content_hash.clone()));
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
    log::debug!(
        "Checking if any of {} surviving symbols use import '{}'",
        used_symbols.len(),
        local_name
    );
    let result = used_symbols.iter().any(|symbol| {
        let uses = module_dep_graph.does_symbol_use_import(symbol, local_name);
        if uses {
            log::debug!("  Symbol '{symbol}' uses import '{local_name}'");
        }
        uses
    });
    if !result {
        log::debug!("  No surviving symbols use import '{local_name}'");
    }
    result
}

/// Check if an import is used by any symbols in __all__ exports
fn is_import_used_by_all_exports(
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    local_name: &str,
) -> bool {
    // Find __all__ assignment and get the exported symbols
    let mut all_exports = Vec::new();
    for item_data in module_dep_graph.items.values() {
        if let crate::cribo_graph::ItemType::Assignment { targets, .. } = &item_data.item_type
            && targets.contains(&"__all__".to_string())
        {
            // The eventual_read_vars contains the names in __all__
            all_exports.extend(item_data.eventual_read_vars.iter().cloned());
            break;
        }
    }

    if all_exports.is_empty() {
        return false;
    }

    log::debug!(
        "Checking if import '{}' is used by any of {} __all__ symbols",
        local_name,
        all_exports.len()
    );

    // Check if any __all__ symbol uses this import
    for export_name in all_exports {
        let uses = module_dep_graph.does_symbol_use_import(&export_name, local_name);
        if uses {
            log::debug!("  __all__ symbol '{export_name}' uses import '{local_name}'");
            return true;
        }
    }

    log::debug!("  Import '{local_name}' is not used by any __all__ symbols");
    false
}

/// Check if an import is used by module-level code with side effects
fn is_import_used_by_side_effect_code(
    shaker: &crate::tree_shaking::TreeShaker<'_>,
    module_dep_graph: &crate::cribo_graph::ModuleDepGraph,
    local_name: &str,
) -> bool {
    let module_id = module_dep_graph.module_id;

    if !shaker.module_has_side_effects(module_id) {
        return false;
    }

    module_dep_graph.items.values().any(|item| {
        matches!(
            item.item_type,
            crate::cribo_graph::ItemType::Expression
                | crate::cribo_graph::ItemType::Assignment { .. }
                | crate::cribo_graph::ItemType::Other
        ) && (item.read_vars.contains(local_name) || item.eventual_read_vars.contains(local_name))
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

                // For relative imports (level > 0), we can't directly compare module names
                // since UnusedImportInfo has resolved names but import_from_stmt has raw syntax.
                // For absolute imports, we can compare the module names directly.
                if import_from_stmt.level > 0 {
                    // Relative import - just match by name since we can't easily resolve the module
                    // here This is safe because the UnusedImportInfo was
                    // created from the same module context
                    unused_imports
                        .iter()
                        .any(|unused| unused.name == local_name)
                } else {
                    // Absolute import - match by both name and module
                    let from_module = import_from_stmt
                        .module
                        .as_ref()
                        .map_or("", ruff_python_ast::Identifier::as_str);
                    unused_imports
                        .iter()
                        .any(|unused| unused.name == local_name && unused.module == from_module)
                }
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
