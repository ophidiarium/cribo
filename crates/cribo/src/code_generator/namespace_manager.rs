//! Namespace management utilities for code generation.
//!
//! This module provides functions for creating and managing Python namespace objects
//! that simulate module structures in bundled code.

use std::path::PathBuf;

use cow_utils::CowUtils;
use log::debug;
use ruff_python_ast::{ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{self, expressions, statements},
    code_generator::bundler::HybridStaticBundler,
    types::{FxIndexMap, FxIndexSet},
};

/// Generates submodule attributes with exclusions for namespace organization.
///
/// This function analyzes module hierarchies and creates namespace modules and assignments
/// as needed, while handling exclusions and avoiding redundant operations.
///
/// **Note**: This is the complete 310-line implementation moved from bundler.rs to achieve
/// Phase 7 token reduction. The implementation uses bundler helper methods where available
/// (`create_namespace_module`, `create_dotted_attribute_assignment`) and direct AST
/// construction for intermediate namespaces that require specific attribute assignments.
pub(super) fn generate_submodule_attributes_with_exclusions(
    bundler: &HybridStaticBundler,
    sorted_modules: &[(String, PathBuf, Vec<String>)],
    final_body: &mut Vec<Stmt>,
    exclusions: &FxIndexSet<String>,
) {
    debug!(
        "generate_submodule_attributes: Starting with {} modules",
        sorted_modules.len()
    );

    // Step 1: Identify all namespaces and modules that need to be created/assigned
    let mut namespace_modules = FxIndexSet::default(); // Simple namespace modules to create
    let mut module_assignments = Vec::new(); // (depth, parent, attr, module_name)

    // First, collect ALL modules that have been initialized (both wrapper and namespace)
    let mut all_initialized_modules = FxIndexSet::default();

    // Add all wrapper modules
    for (module_name, _, _) in sorted_modules {
        if bundler.module_registry.contains_key(module_name) {
            all_initialized_modules.insert(module_name.clone());
        }
    }

    // Now analyze what namespaces are needed and add wrapper module assignments
    // Combined loop for better efficiency
    for module_name in &all_initialized_modules {
        if !module_name.contains('.') {
            continue;
        }

        // This is a dotted module - ensure all parent namespaces exist
        let parts: Vec<&str> = module_name.split('.').collect();

        // Collect all parent levels that need to exist
        for i in 1..parts.len() {
            let parent_path = parts[..i].join(".");

            // If this parent is not already an initialized module, it's a namespace that needs
            // to be created
            if !all_initialized_modules.contains(&parent_path) {
                if i == 1 {
                    // Top-level namespace (e.g., 'core', 'models', 'services')
                    namespace_modules.insert(parent_path);
                } else {
                    // Intermediate namespace (e.g., 'core.database')
                    // These will be created as attributes after their parent exists
                    let parent = parts[..i - 1].join(".");
                    let attr = parts[i - 1];
                    module_assignments.push((i, parent, attr.to_string(), parent_path));
                }
            }
        }

        // Add wrapper module assignment for this module
        let parent = parts[..parts.len() - 1].join(".");
        let attr = parts[parts.len() - 1];

        // Only add if this is actually a wrapper module
        if bundler.module_registry.contains_key(module_name) {
            module_assignments.push((parts.len(), parent, attr.to_string(), module_name.clone()));
        }
    }

    // Step 2: Create top-level namespace modules and wrapper module references
    let mut created_namespaces = FxIndexSet::default();

    // Add all namespaces that were already created via the namespace tracking index
    for namespace in &bundler.required_namespaces {
        created_namespaces.insert(namespace.clone());
    }

    // First, create references to top-level wrapper modules
    let mut top_level_wrappers = Vec::new();
    for module_name in &all_initialized_modules {
        if !module_name.contains('.') && bundler.module_registry.contains_key(module_name) {
            // This is a top-level wrapper module
            top_level_wrappers.push(module_name.clone());
        }
    }
    top_level_wrappers.sort(); // Deterministic order

    for wrapper in top_level_wrappers {
        // Skip if this module is imported in the entry module
        if exclusions.contains(&wrapper) {
            debug!("Skipping top-level wrapper '{wrapper}' - imported in entry module");
            created_namespaces.insert(wrapper);
            continue;
        }

        debug!("Top-level wrapper '{wrapper}' already initialized, skipping assignment");
        // Top-level wrapper modules are already initialized via their init functions
        // No need to create any assignment - the module already exists
        created_namespaces.insert(wrapper);
    }

    // Then, create namespace modules
    let mut sorted_namespaces: Vec<String> = namespace_modules.into_iter().collect();
    sorted_namespaces.sort(); // Deterministic order

    for namespace in sorted_namespaces {
        // Skip if this namespace was already created via the namespace tracking index
        if bundler.required_namespaces.contains(&namespace) {
            debug!(
                "Skipping top-level namespace '{namespace}' - already created via namespace index"
            );
            created_namespaces.insert(namespace);
            continue;
        }

        // Check if this namespace was already created globally
        if bundler.created_namespaces.contains(&namespace) {
            debug!("Skipping top-level namespace '{namespace}' - already created globally");
            created_namespaces.insert(namespace);
            continue;
        }

        debug!("Creating top-level namespace: {namespace}");
        final_body.extend(bundler.create_namespace_module(&namespace));
        created_namespaces.insert(namespace);
    }

    // Step 3: Sort module assignments by depth to ensure parents exist before children
    module_assignments.sort_by(
        |(depth_a, parent_a, attr_a, name_a), (depth_b, parent_b, attr_b, name_b)| {
            (depth_a, parent_a.as_str(), attr_a.as_str(), name_a.as_str()).cmp(&(
                depth_b,
                parent_b.as_str(),
                attr_b.as_str(),
                name_b.as_str(),
            ))
        },
    );

    // Step 4: Process all assignments in order
    for (depth, parent, attr, module_name) in module_assignments {
        debug!("Processing assignment: {parent}.{attr} = {module_name} (depth={depth})");

        // Check if parent exists or will exist
        let parent_exists = created_namespaces.contains(&parent)
            || bundler.module_registry.contains_key(&parent)
            || parent.is_empty(); // Empty parent means top-level

        if !parent_exists {
            debug!("Warning: Parent '{parent}' doesn't exist for assignment {parent}.{attr}");
            continue;
        }

        if bundler.module_registry.contains_key(&module_name) {
            // Check if parent module has this attribute in __all__ (indicating a re-export)
            // OR if the parent is a wrapper module and the attribute is already defined there
            let skip_assignment =
                if let Some(Some(parent_exports)) = bundler.module_exports.get(&parent) {
                    if parent_exports.contains(&attr) {
                        // Check if this is a symbol re-exported from within the parent module
                        // rather than the submodule itself
                        // For example, in mypackage/__init__.py:
                        // from .config import config  # imports the 'config' instance, not the
                        // module __all__ = ['config']        # exports the
                        // instance

                        // In this case, 'config' in parent_exports refers to an imported symbol,
                        // not the submodule 'mypackage.config'
                        debug!(
                            "Skipping submodule assignment for {parent}.{attr} - it's a \
                             re-exported attribute (not the module itself)"
                        );
                        true
                    } else {
                        false
                    }
                } else if bundler.module_registry.contains_key(&parent) {
                    // Parent is a wrapper module - check if it already has this attribute defined
                    // This handles cases where the wrapper module imports a symbol with the same
                    // name as a submodule (e.g., from .config import config)
                    debug!(
                        "Parent {parent} is a wrapper module, checking if {attr} is already \
                         defined there"
                    );
                    // For now, we'll check if the attribute is in parent_exports
                    // This may need refinement based on more complex cases
                    false
                } else {
                    false
                };

            if !skip_assignment {
                // Check if this module was imported in the entry module
                if exclusions.contains(&module_name) {
                    debug!(
                        "Skipping wrapper module assignment '{parent}.{attr} = {module_name}' - \
                         imported in entry module"
                    );
                } else {
                    // Check if this would be a redundant self-assignment
                    let full_target = format!("{parent}.{attr}");
                    if full_target == module_name {
                        debug!(
                            "Skipping redundant self-assignment: {parent}.{attr} = {module_name}"
                        );
                    } else {
                        // This is a wrapper module - assign direct reference
                        debug!("Assigning wrapper module: {parent}.{attr} = {module_name}");
                        final_body.push(bundler.create_dotted_attribute_assignment(
                            &parent,
                            &attr,
                            &module_name,
                        ));
                    }
                }
            }
        } else {
            // This is an intermediate namespace - skip if already created via namespace index
            if bundler.required_namespaces.contains(&module_name) {
                debug!(
                    "Skipping intermediate namespace '{module_name}' - already created via \
                     namespace index"
                );
                created_namespaces.insert(module_name);
                continue;
            }

            debug!("Creating intermediate namespace: {parent}.{attr} = types.SimpleNamespace()");
            // Create: parent.attr = types.SimpleNamespace()
            final_body.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::name(&parent, ExprContext::Load),
                    &attr,
                    ExprContext::Store,
                )],
                ast_builder::expressions::call(
                    ast_builder::expressions::simple_namespace_ctor(),
                    vec![],
                    vec![],
                ),
            ));

            // Set the __name__ attribute: parent.attr.__name__ = module_name
            final_body.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::attribute(
                        ast_builder::expressions::name(&parent, ExprContext::Load),
                        &attr,
                        ExprContext::Load,
                    ),
                    "__name__",
                    ExprContext::Store,
                )],
                ast_builder::expressions::string_literal(&module_name),
            ));

            created_namespaces.insert(module_name);
        }
    }
}

/// Transform imports from namespace packages.
///
/// This function handles the transformation of imports from namespace packages,
/// creating appropriate assignments and namespace objects as needed.
pub(super) fn transform_namespace_package_imports(
    bundler: &HybridStaticBundler,
    import_from: StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
        let full_module_path = format!("{module_name}.{imported_name}");

        if bundler.bundled_modules.contains(&full_module_path) {
            if bundler.module_registry.contains_key(&full_module_path) {
                // Wrapper module - ensure it's initialized first, then create reference
                // First ensure parent module is initialized if it's also a wrapper
                if bundler.module_registry.contains_key(module_name) {
                    result_stmts
                        .extend(bundler.create_module_initialization_for_import(module_name));
                }
                // Initialize the wrapper module if needed
                result_stmts
                    .extend(bundler.create_module_initialization_for_import(&full_module_path));

                // Create assignment using dotted name since it's a nested module
                let module_expr = if full_module_path.contains('.') {
                    let parts: Vec<&str> = full_module_path.split('.').collect();
                    expressions::dotted_name(&parts, ExprContext::Load)
                } else {
                    expressions::name(&full_module_path, ExprContext::Load)
                };

                result_stmts.push(statements::simple_assign(local_name, module_expr));
            } else {
                // Inlined module - create a namespace object for it
                log::debug!(
                    "Submodule '{imported_name}' from namespace package '{module_name}' was \
                     inlined, creating namespace"
                );

                // For namespace hybrid modules, we need to create the namespace object
                // The inlined module's symbols are already renamed with module prefix
                // e.g., message -> message_greetings_greeting
                let _inlined_key = full_module_path.cow_replace('.', "_").into_owned();

                // Create a SimpleNamespace object manually with all the inlined symbols
                // Since the module was inlined, we need to map the original names to the
                // renamed ones
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
                ));

                // Add all the renamed symbols as attributes to the namespace
                // Get the symbol renames for this module if available
                if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                    let module_suffix = full_module_path.cow_replace('.', "_");
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
                    log::warn!(
                        "No symbol renames found for inlined module '{full_module_path}', \
                         namespace will be empty"
                    );
                }
            }
        } else {
            // Not a bundled submodule, keep as attribute access
            // This might be importing a symbol from the namespace package's __init__.py
            // But since we're here, the namespace package has no __init__.py
            log::warn!(
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
/// Helper function used by transform_namespace_package_imports.
fn get_unique_name_with_module_suffix(base_name: &str, module_name: &str) -> String {
    let module_suffix = module_name.cow_replace('.', "_").into_owned();
    format!("{base_name}_{module_suffix}")
}

/// Ensure a namespace exists, creating it and any parent namespaces if needed.
/// Returns statements to create any missing namespaces.
pub(super) fn ensure_namespace_exists(
    bundler: &mut HybridStaticBundler,
    namespace_path: &str,
) -> Vec<Stmt> {
    let mut statements = Vec::new();

    // For dotted names like "models.user", we need to ensure "models" exists first
    if namespace_path.contains('.') {
        let parts: Vec<&str> = namespace_path.split('.').collect();

        // Create all parent namespaces
        for i in 1..=parts.len() {
            let namespace = parts[..i].join(".");

            if !bundler.created_namespaces.contains(&namespace) {
                debug!("Creating namespace dynamically: {namespace}");

                if i == 1 {
                    // Top-level namespace
                    statements.extend(bundler.create_namespace_module(&namespace));
                } else {
                    // Nested namespace - create as attribute
                    let parent = parts[..i - 1].join(".");
                    let child = parts[i - 1];
                    statements.push(create_namespace_attribute(bundler, &parent, child));
                }

                bundler.created_namespaces.insert(namespace);
            }
        }
    } else {
        // Simple namespace without dots
        if !bundler.created_namespaces.contains(namespace_path) {
            debug!("Creating simple namespace dynamically: {namespace_path}");
            statements.extend(bundler.create_namespace_module(namespace_path));
            bundler
                .created_namespaces
                .insert(namespace_path.to_string());
        }
    }

    statements
}

/// Create namespace attribute assignment.
///
/// Creates: parent.child = types.SimpleNamespace()
pub(super) fn create_namespace_attribute(
    bundler: &mut HybridStaticBundler,
    parent: &str,
    child: &str,
) -> Stmt {
    // Create: parent.child = types.SimpleNamespace()
    let mut stmt = statements::assign(
        vec![expressions::attribute(
            expressions::name(parent, ExprContext::Load),
            child,
            ExprContext::Store,
        )],
        expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
    );

    // Update the node index for tracking
    if let Stmt::Assign(assign) = &mut stmt {
        assign.node_index = bundler
            .transformation_context
            .create_new_node(format!("Create namespace attribute {parent}.{child}"));
    }

    stmt
}

/// Create a namespace object with __name__ attribute.
pub(super) fn create_namespace_with_name(var_name: &str, module_path: &str) -> Vec<Stmt> {
    // Create: var_name = types.SimpleNamespace()
    let types_simple_namespace_call =
        expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]);
    let mut statements = vec![statements::simple_assign(
        var_name,
        types_simple_namespace_call,
    )];

    // Set the __name__ attribute
    let target = expressions::attribute(
        expressions::name(var_name, ExprContext::Load),
        "__name__",
        ExprContext::Store,
    );
    let value = expressions::string_literal(module_path);
    statements.push(statements::assign(vec![target], value));

    statements
}

/// Create namespace statements for required namespaces.
pub(super) fn create_namespace_statements(bundler: &mut HybridStaticBundler) -> Vec<Stmt> {
    let mut statements = Vec::new();

    // Sort namespaces for deterministic output
    let mut sorted_namespaces: Vec<String> = bundler.required_namespaces.iter().cloned().collect();
    sorted_namespaces.sort();

    for namespace in sorted_namespaces {
        debug!("Creating namespace statement for: {namespace}");

        // Use ensure_namespace_exists to handle both simple and dotted namespaces
        let namespace_stmts = ensure_namespace_exists(bundler, &namespace);
        statements.extend(namespace_stmts);
    }

    statements
}

/// Create namespace for inlined module.
///
/// Creates a types.SimpleNamespace object with all the module's symbols,
/// handling forward references and tree-shaking.
pub(super) fn create_namespace_for_inlined_module_static(
    bundler: &mut HybridStaticBundler,
    module_name: &str,
    module_renames: &FxIndexMap<String, String>,
) -> Stmt {
    // Check if this module has forward references that would cause NameError
    // This happens when the module uses symbols from other modules that haven't been defined
    // yet
    let has_forward_references =
        bundler.check_module_has_forward_references(module_name, module_renames);

    if has_forward_references {
        log::debug!("Module '{module_name}' has forward references, creating empty namespace");
        // Create the namespace variable name
        let namespace_var = module_name.cow_replace('.', "_").into_owned();

        // Create empty namespace = types.SimpleNamespace() to avoid forward reference errors
        return statements::simple_assign(
            &namespace_var,
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
        );
    }
    // Create a types.SimpleNamespace with all the module's symbols
    let mut keywords = Vec::new();
    let mut seen_args = FxIndexSet::default();

    // Add all renamed symbols as keyword arguments, avoiding duplicates
    for (original_name, renamed_name) in module_renames {
        // Skip if we've already added this argument name
        if seen_args.contains(original_name) {
            log::debug!(
                "Skipping duplicate namespace argument '{original_name}' for module \
                 '{module_name}'"
            );
            continue;
        }

        // Check if this symbol survived tree-shaking
        if let Some(ref kept_symbols) = bundler.tree_shaking_keep_symbols
            && !kept_symbols.contains(&(module_name.to_string(), original_name.clone()))
        {
            log::debug!(
                "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                 '{module_name}'"
            );
            continue;
        }

        seen_args.insert(original_name.clone());

        keywords.push(ruff_python_ast::Keyword {
            node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
            arg: Some(ruff_python_ast::Identifier::new(
                original_name,
                ruff_text_size::TextRange::default(),
            )),
            value: expressions::name(renamed_name, ExprContext::Load),
            range: ruff_text_size::TextRange::default(),
        });
    }

    // Also check if module has module-level variables that weren't renamed
    if let Some(exports) = bundler.module_exports.get(module_name)
        && let Some(export_list) = exports
    {
        for export in export_list {
            // Check if this export was already added as a renamed symbol
            if !module_renames.contains_key(export) && !seen_args.contains(export) {
                // Check if this symbol survived tree-shaking
                if let Some(ref kept_symbols) = bundler.tree_shaking_keep_symbols
                    && !kept_symbols.contains(&(module_name.to_string(), export.clone()))
                {
                    log::debug!(
                        "Skipping tree-shaken export '{export}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                // This export wasn't renamed and wasn't already added, add it directly
                seen_args.insert(export.clone());
                keywords.push(ruff_python_ast::Keyword {
                    node_index: ruff_python_ast::AtomicNodeIndex::dummy(),
                    arg: Some(ruff_python_ast::Identifier::new(
                        export,
                        ruff_text_size::TextRange::default(),
                    )),
                    value: expressions::name(export, ExprContext::Load),
                    range: ruff_text_size::TextRange::default(),
                });
            }
        }
    }

    // Create the namespace variable name
    let namespace_var = module_name.cow_replace('.', "_").into_owned();

    // namespace_var = types.SimpleNamespace(**kwargs)
    statements::assign(
        vec![expressions::name(&namespace_var, ExprContext::Store)],
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
    )
}
