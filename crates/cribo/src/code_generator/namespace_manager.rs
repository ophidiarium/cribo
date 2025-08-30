//! Namespace management utilities for code generation.
//!
//! This module provides functions for creating and managing Python namespace objects
//! that simulate module structures in bundled code.

use std::path::PathBuf;

use log::{debug, warn};
use ruff_python_ast::{Expr, ExprContext, ModModule, Stmt, StmtImportFrom};

use crate::{
    analyzers::symbol_analyzer::SymbolAnalyzer,
    ast_builder::{expressions, statements},
    code_generator::{bundler::Bundler, module_registry::sanitize_module_name_for_identifier},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Information about a registered namespace
#[derive(Debug, Clone)]
pub struct NamespaceInfo {}

/// Context for populating namespace with module symbols.
///
/// This struct encapsulates the state required by the namespace population function,
/// which was previously accessed directly from the `Bundler` struct.
pub struct NamespacePopulationContext<'a> {
    pub inlined_modules: &'a FxIndexSet<ModuleId>,
    pub module_exports: &'a FxIndexMap<ModuleId, Option<Vec<String>>>,
    pub tree_shaking_keep_symbols: &'a Option<FxIndexMap<ModuleId, FxIndexSet<String>>>,
    pub bundled_modules: &'a FxIndexSet<ModuleId>,
    pub modules_with_accessed_all: &'a FxIndexSet<(ModuleId, String)>,
    pub wrapper_modules: &'a FxIndexSet<ModuleId>,
    pub module_asts: &'a Option<FxIndexMap<ModuleId, (ModModule, PathBuf, String)>>,
    pub global_deferred_imports: &'a FxIndexMap<(ModuleId, String), ModuleId>,
    pub module_init_functions: &'a FxIndexMap<ModuleId, String>,
    pub resolver: &'a crate::resolver::ModuleResolver,
}

// Note: NamespacePopulationContext has no inherent methods currently.
// All functionality is in the standalone functions that use it.

/// Create an attribute assignment statement, using namespace variables when available.
///
/// This function creates `parent.attr = value` statements, but intelligently uses
/// namespace variables when they exist. For example, if assigning `services.auth`,
/// it will use the `services_auth` namespace variable if it exists.
pub fn create_attribute_assignment(
    bundler: &Bundler,
    parent: &str,
    attr: &str,
    module_name: &str,
) -> Stmt {
    // Check if there's a namespace variable for the module
    let sanitized_module = sanitize_module_name_for_identifier(module_name);

    let value_expr = if bundler.created_namespaces.contains(&sanitized_module) {
        // Use the namespace variable (e.g., services_auth instead of services.auth)
        debug!("Using namespace variable '{sanitized_module}' for {parent}.{attr} = {module_name}");
        expressions::name(&sanitized_module, ExprContext::Load)
    } else if module_name.contains('.') {
        // Create a dotted expression for the module path
        let parts: Vec<&str> = module_name.split('.').collect();
        expressions::dotted_name(&parts, ExprContext::Load)
    } else {
        // Simple name
        expressions::name(module_name, ExprContext::Load)
    };

    // Create the assignment: parent.attr = value
    statements::assign_attribute(parent, attr, value_expr)
}

/// Generates submodule attributes with exclusions for namespace organization.
///
/// This function analyzes module hierarchies and creates namespace modules and assignments
/// as needed, while handling exclusions and avoiding redundant operations.
///
/// **Note**: This is the complete 310-line implementation moved from bundler.rs to achieve
/// Transform imports from namespace packages.
///
/// This function handles the transformation of imports from namespace packages,
/// creating appropriate assignments and namespace objects as needed.
pub(super) fn transform_namespace_package_imports(
    bundler: &Bundler,
    import_from: StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
        let full_module_path = format!("{module_name}.{imported_name}");
        let full_module_id = bundler.get_module_id(&full_module_path);
        let parent_module_id = bundler.get_module_id(module_name);

        if let Some(id) = full_module_id
            && bundler.bundled_modules.contains(&id)
        {
            if bundler.module_synthetic_names.contains_key(&id) {
                // Wrapper module - ensure it's initialized first, then create reference
                // First ensure parent module is initialized if it's also a wrapper
                if let Some(parent_id) = parent_module_id
                    && bundler.module_synthetic_names.contains_key(&parent_id)
                {
                    // Build a temporary map for the module initialization
                    let mut temp_registry = crate::types::FxIndexMap::default();
                    if let Some(synthetic_name) = bundler.module_synthetic_names.get(&parent_id) {
                        temp_registry.insert(module_name.to_string(), synthetic_name.clone());
                    }
                    result_stmts.extend(
                        crate::code_generator::module_registry::create_module_initialization_for_import(
                            module_name,
                            &temp_registry,
                        ),
                    );
                }
                // Initialize the wrapper module if needed
                if let Some(module_id) = bundler.resolver.get_module_id_by_name(&full_module_path) {
                    let mut temp_registry = crate::types::FxIndexMap::default();
                    if let Some(synthetic_name) = bundler.module_synthetic_names.get(&module_id) {
                        temp_registry.insert(full_module_path.clone(), synthetic_name.clone());
                    }
                    result_stmts.extend(
                        crate::code_generator::module_registry::create_module_initialization_for_import(
                            &full_module_path,
                            &temp_registry,
                        ),
                    );
                }

                // Create assignment using dotted name since it's a nested module
                let module_expr =
                    expressions::module_reference(&full_module_path, ExprContext::Load);

                result_stmts.push(statements::simple_assign(local_name, module_expr));
            } else {
                // Inlined module - create a namespace object for it
                debug!(
                    "Submodule '{imported_name}' from namespace package '{module_name}' was \
                     inlined, creating namespace"
                );

                // For namespace hybrid modules, we need to create the namespace object
                // The inlined module's symbols are already renamed with module prefix
                // e.g., message -> message_greetings_greeting
                let _inlined_key = sanitize_module_name_for_identifier(&full_module_path);

                // Create a SimpleNamespace object manually with all the inlined symbols
                // Since the module was inlined, we need to map the original names to the
                // renamed ones
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
                ));

                // Add all the renamed symbols as attributes to the namespace
                // Get the symbol renames for this module if available
                let full_module_id = bundler
                    .get_module_id(&full_module_path)
                    .expect("Module should exist");
                if let Some(module_renames) = symbol_renames.get(&full_module_id) {
                    let module_suffix = sanitize_module_name_for_identifier(&full_module_path);
                    for (original_name, renamed_name) in module_renames {
                        // Check if this is an identity mapping (no semantic rename)
                        let actual_renamed_name = if renamed_name == original_name {
                            // No semantic rename, apply module suffix pattern

                            get_unique_name_with_module_suffix(original_name, &module_suffix)
                        } else {
                            // Use the semantic rename
                            renamed_name.clone()
                        };

                        // base.original_name = actual_renamed_name
                        result_stmts.push(statements::assign(
                            vec![expressions::attribute(
                                expressions::name(local_name, ExprContext::Load),
                                original_name,
                                ExprContext::Store,
                            )],
                            expressions::name(&actual_renamed_name, ExprContext::Load),
                        ));
                    }
                } else {
                    // Fallback: try to guess the renamed symbols based on module suffix
                    warn!(
                        "No symbol renames found for inlined module '{full_module_path}', \
                         namespace will be empty"
                    );
                }
            }
        } else {
            // Not a bundled submodule, keep as attribute access
            // This might be importing a symbol from the namespace package's __init__.py
            // But since we're here, the namespace package has no __init__.py
            warn!(
                "Import '{imported_name}' from namespace package '{module_name}' is not a bundled \
                 module"
            );
        }
    }

    if result_stmts.is_empty() {
        // If we didn't transform anything, return the original
        vec![Stmt::ImportFrom(import_from)]
    } else {
        result_stmts
    }
}

/// Get a unique name for a symbol, using the module suffix pattern.
///
/// Helper function used by `transform_namespace_package_imports`.
fn get_unique_name_with_module_suffix(base_name: &str, module_name: &str) -> String {
    let module_suffix = sanitize_module_name_for_identifier(module_name);
    format!("{base_name}_{module_suffix}")
}

// NOTE: ensure_namespace_exists was removed as it became obsolete after implementing
// the centralized namespace registry. Its functionality is now handled by:
// - require_namespace() for registration
// - generate_required_namespaces() for generation

/// Create namespace for inlined module.
///
/// Populate a namespace object with all symbols from a given module, applying renames.
///
/// This function generates AST statements to populate a namespace object with symbols
/// from a module, handling tree-shaking, re-exports, and symbol renaming.
pub fn populate_namespace_with_module_symbols(
    ctx: &mut NamespacePopulationContext,
    target_name: &str,
    module_id: ModuleId,
    symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    // Get the module name from the resolver
    let Some(module_info) = ctx.resolver.get_module(module_id) else {
        return result_stmts;
    };
    let module_name = &module_info.name;

    // Get the module's exports
    if let Some(exports) = ctx.module_exports.get(&module_id).and_then(|e| e.as_ref()) {
        // Build the namespace access expression for the target
        let parts: Vec<&str> = target_name.split('.').collect();

        // First, add __all__ attribute to the namespace
        // Create the target expression for __all__
        let all_target = expressions::dotted_name(&parts, ExprContext::Load);

        // Filter exports to only include symbols that survived tree-shaking
        let filtered_exports = SymbolAnalyzer::filter_exports_by_tree_shaking(
            exports,
            &module_id,
            ctx.tree_shaking_keep_symbols.as_ref(),
            true,
            ctx.resolver,
        );

        // Check if __all__ assignment already exists for this namespace
        let all_assignment_exists = result_stmts.iter().any(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                && let Expr::Name(base) = attr.value.as_ref()
            {
                return base.id.as_str() == target_name && attr.attr.as_str() == "__all__";
            }
            false
        });

        if all_assignment_exists {
            debug!("Skipping duplicate __all__ assignment for namespace '{target_name}'");
        } else if ctx
            .modules_with_accessed_all
            .iter()
            .any(|(_, accessed_module)| accessed_module == module_name)
        {
            // Only create __all__ assignment if the code actually accesses it
            let all_list = expressions::list(
                filtered_exports
                    .iter()
                    .map(|name| expressions::string_literal(name.as_str()))
                    .collect(),
                ExprContext::Load,
            );

            // Create __all__ assignment statement
            result_stmts.push(statements::assign(
                vec![expressions::attribute(
                    all_target,
                    "__all__",
                    ExprContext::Store,
                )],
                all_list,
            ));

            debug!(
                "Created __all__ assignment for namespace '{target_name}' with exports: \
                 {filtered_exports:?} (accessed in code)"
            );
        } else {
            debug!(
                "Skipping __all__ assignment for namespace '{target_name}' - not accessed in code"
            );
        }

        // For each exported symbol that survived tree-shaking, add it to the namespace
        'symbol_loop: for symbol in &filtered_exports {
            let symbol_name = symbol.as_str();
            // filtered_exports dictates inclusion; no extra checks needed here.

            // Check if this symbol is actually a submodule
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let submodule_id = ctx.resolver.get_module_id_by_name(&full_submodule_path);
            let is_bundled_submodule =
                submodule_id.is_some_and(|id| ctx.bundled_modules.contains(&id));
            let is_inlined = submodule_id.is_some_and(|id| ctx.inlined_modules.contains(&id));
            let uses_init_function =
                submodule_id.is_some_and(|id| ctx.wrapper_modules.contains(&id));

            if is_bundled_submodule {
                debug!(
                    "Symbol '{symbol_name}' in module '{module_name}' is a submodule (bundled: \
                     {is_bundled_submodule}, inlined: {is_inlined}, uses_init: \
                     {uses_init_function})"
                );

                // For inlined submodules, check if the parent module re-exports a symbol
                // with the same name as the submodule (e.g., __version__ from __version__
                // module)
                if is_inlined {
                    // Check if the submodule has a symbol with the same name as itself
                    let Some(submodule_id) = submodule_id else {
                        continue;
                    };

                    let Some(submodule_exports) = ctx
                        .module_exports
                        .get(&submodule_id)
                        .and_then(|e| e.as_ref())
                    else {
                        continue;
                    };

                    if !submodule_exports.contains(&symbol_name.to_string()) {
                        continue;
                    }

                    // The submodule exports a symbol with the same name as itself
                    // Check if the parent module re-exports this symbol
                    debug!(
                        "Submodule '{full_submodule_path}' exports symbol '{symbol_name}' \
                         with same name"
                    );

                    // Get the renamed symbol from the submodule
                    if let Some(submodule_renames) = symbol_renames.get(&submodule_id)
                        && let Some(renamed) = submodule_renames.get(symbol_name)
                    {
                        debug!(
                            "Creating namespace assignment: {target_name}.{symbol_name} = \
                             {renamed} (re-exported from submodule)"
                        );

                        // Create the assignment
                        let target = expressions::dotted_name(&parts, ExprContext::Load);
                        result_stmts.push(statements::assign(
                            vec![expressions::attribute(
                                target,
                                symbol_name,
                                ExprContext::Store,
                            )],
                            expressions::name(renamed, ExprContext::Load),
                        ));
                        continue 'symbol_loop;
                    }
                }

                // Skip other submodules - they are handled separately
                // This prevents creating invalid assignments like `mypkg.compat = compat`
                // when `compat` is a submodule, not a local variable
                continue;
            }

            // Get the renamed symbol if it exists
            let actual_symbol_name = if let Some(module_renames) = symbol_renames.get(&module_id) {
                module_renames
                    .get(symbol_name)
                    .cloned()
                    .unwrap_or_else(|| symbol_name.to_string())
            } else {
                symbol_name.to_string()
            };

            // Create the target expression
            // For simple modules, this will be the module name directly
            // For dotted modules (e.g., greetings.greeting), build the chain
            let target = expressions::dotted_name(&parts, ExprContext::Load);

            // Check if this assignment already exists in result_stmts
            let assignment_exists = result_stmts.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && assign.targets.len() == 1
                    && let Expr::Attribute(attr) = &assign.targets[0]
                {
                    // Check if this is the same assignment target
                    if let Expr::Name(base) = attr.value.as_ref() {
                        return base.id.as_str() == target_name
                            && attr.attr.as_str() == symbol_name;
                    }
                }
                false
            });

            if assignment_exists {
                debug!(
                    "[populate_namespace_with_module_symbols_with_renames] Skipping duplicate \
                     namespace assignment: {target_name}.{symbol_name} = {actual_symbol_name} \
                     (assignment already exists)"
                );
                continue;
            }

            // Also check if this is a parent module assignment that might already exist
            // For example, if we're processing mypkg.exceptions and the symbol CustomJSONError
            // is in mypkg's __all__, check if mypkg.CustomJSONError = CustomJSONError already
            // exists
            if module_name.contains('.') {
                let parent_module = module_name
                    .rsplit_once('.')
                    .map_or("", |(parent, _)| parent);
                if !parent_module.is_empty()
                    && let Some(parent_id) = ctx.resolver.get_module_id_by_name(parent_module)
                    && let Some(Some(parent_exports)) = ctx.module_exports.get(&parent_id)
                    && parent_exports.contains(&symbol_name.to_string())
                {
                    // This symbol is re-exported by the parent module
                    // Check if the parent assignment already exists
                    let parent_assignment_exists = result_stmts.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Attribute(attr) = &assign.targets[0]
                        {
                            // Check if this is the same assignment
                            if let Expr::Name(base) = attr.value.as_ref() {
                                return base.id.as_str() == parent_module
                                    && attr.attr.as_str() == symbol_name;
                            }
                        }
                        false
                    });

                    if parent_assignment_exists {
                        debug!(
                            "[populate_namespace_with_module_symbols_with_renames/parent] \
                             Skipping duplicate namespace assignment: {target_name}.{symbol_name} \
                             = {actual_symbol_name} (parent assignment already exists in \
                             result_stmts)"
                        );
                        continue;
                    }
                }
            }

            // Check if symbol is a dunder name
            if symbol_name.starts_with("__") && symbol_name.ends_with("__") {
                // For dunder names, check if they're in the __all__ list
                if !exports.contains(&symbol_name.to_string()) {
                    debug!(
                        "Skipping dunder name '{symbol_name}' not in __all__ for module \
                         '{module_name}'"
                    );
                    continue;
                }
            }

            // Also check if this assignment was already made by deferred imports
            // This handles the case where imports create namespace assignments that
            // would be duplicated by __all__ processing
            if !ctx.global_deferred_imports.is_empty() {
                // Check if this symbol was deferred by the same module (intra-module imports)
                let module_id = ctx
                    .resolver
                    .get_module_id_by_name(module_name)
                    .expect("Module ID must exist for module");
                let key = (module_id, symbol_name.to_string());
                if ctx.global_deferred_imports.contains_key(&key) {
                    debug!(
                        "Skipping namespace assignment for '{symbol_name}' - already created by \
                         deferred import from module '{module_name}'"
                    );
                    continue;
                }
            }

            // For wrapper modules, check if the symbol is imported from an inlined submodule
            // These symbols are already added via module attribute assignments
            if ctx.wrapper_modules.contains(&module_id)
                && is_symbol_from_inlined_submodule(ctx, module_name, symbol_name)
            {
                continue 'symbol_loop;
            }

            // Check if this is a submodule that uses an init function
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let submodule_id = ctx.resolver.get_module_id_by_name(&full_submodule_path);
            let uses_init_function = submodule_id
                .and_then(|id| ctx.module_init_functions.get(&id))
                .is_some();

            if uses_init_function {
                // This is a submodule that uses an init function
                // The assignment will be handled by the init function call
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it uses an \
                     init function"
                );
                continue;
            }

            // Check if this is an inlined submodule (no local variable exists)
            let is_inlined_submodule =
                submodule_id.is_some_and(|id| ctx.inlined_modules.contains(&id));
            if is_inlined_submodule {
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it's an \
                     inlined submodule"
                );
                continue;
            }

            // Check if this is a submodule at all (vs a symbol defined in the module)
            let is_bundled_submodule =
                submodule_id.is_some_and(|id| ctx.bundled_modules.contains(&id));
            if is_bundled_submodule {
                // This is a submodule that's bundled but neither inlined nor uses init
                // function This can happen when the submodule is
                // handled differently (e.g., by deferred imports)
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it's a \
                     bundled submodule"
                );
                continue;
            }

            // Check if this symbol is re-exported from a wrapper module
            // If so, we need to reference it from that module's namespace
            let symbol_expr = if let Some((source_module, original_name)) =
                find_symbol_source_module(ctx, module_name, symbol_name)
            {
                // Symbol is imported from a wrapper module
                // After the wrapper module's init function runs, the symbol will be available
                // as source_module.original_name (handles aliases correctly)
                debug!(
                    "Creating namespace assignment: {target_name}.{symbol_name} = \
                     {source_module}.{original_name} (re-exported from wrapper module)"
                );

                // Create a reference to the symbol from the source module
                let source_parts: Vec<&str> = source_module.split('.').collect();
                let source_expr = expressions::dotted_name(&source_parts, ExprContext::Load);
                expressions::attribute(source_expr, &original_name, ExprContext::Load)
            } else {
                // Symbol is defined in this module or renamed
                debug!(
                    "Creating namespace assignment: {target_name}.{symbol_name} = \
                     {actual_symbol_name} (local symbol)"
                );
                expressions::name(&actual_symbol_name, ExprContext::Load)
            };

            // Now add the symbol as an attribute
            result_stmts.push(statements::assign(
                vec![expressions::attribute(
                    target,
                    symbol_name,
                    ExprContext::Store,
                )],
                symbol_expr,
            ));
        }
    }

    result_stmts
}

/// Check if a symbol in a wrapper module is imported from an inlined submodule.
///
/// This helper function reduces nesting in `populate_namespace_with_module_symbols`
/// by extracting the logic for checking if a symbol is already handled via module
/// attribute assignments.
fn is_symbol_from_inlined_submodule(
    ctx: &NamespacePopulationContext,
    module_name: &str,
    symbol_name: &str,
) -> bool {
    debug!(
        "Module '{module_name}' is a wrapper module, checking if symbol '{symbol_name}' is \
         imported from inlined submodule"
    );

    let Some(module_asts) = ctx.module_asts.as_ref() else {
        return false;
    };

    // Get the module ID for this module
    let Some(module_id) = ctx.resolver.get_module_id_by_name(module_name) else {
        return false;
    };

    // Find the module's AST to check its imports
    let Some((ast, module_path, _)) = module_asts.get(&module_id) else {
        return false;
    };

    // Check if this symbol is imported from an inlined submodule
    for stmt in &ast.body {
        let Stmt::ImportFrom(import_from) = stmt else {
            continue;
        };

        let resolved_module = crate::code_generator::symbol_source::resolve_import_module(
            ctx.resolver,
            import_from,
            module_path,
        );

        if let Some(ref resolved) = resolved_module {
            // Check if the resolved module is inlined
            if let Some(resolved_id) = ctx.resolver.get_module_id_by_name(resolved)
                && ctx.inlined_modules.contains(&resolved_id)
            {
                // Check if our symbol is in this import
                for alias in &import_from.names {
                    if alias.name.as_str() == symbol_name {
                        debug!(
                            "Skipping namespace assignment for '{symbol_name}' - already imported \
                             from inlined module '{resolved}' and added as module attribute"
                        );
                        // Skip this symbol - it's already added via module attributes
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Find the source module and original name for a re-exported symbol.
///
/// This helper function checks if a symbol is imported from another module
/// and returns the source module name and original symbol name if it's a wrapper module.
/// This handles import aliases correctly (e.g., `from .base import YAMLObject as YO`).
fn find_symbol_source_module(
    ctx: &NamespacePopulationContext,
    module_name: &str,
    symbol_name: &str,
) -> Option<(String, String)> {
    let module_asts = ctx.module_asts.as_ref()?;

    crate::code_generator::symbol_source::find_symbol_source_from_wrapper_module(
        module_asts,
        ctx.resolver,
        ctx.wrapper_modules,
        module_name,
        symbol_name,
    )
}
