//! Namespace management utilities for code generation.
//!
//! This module provides functions for creating and managing Python namespace objects
//! that simulate module structures in bundled code.

use std::path::PathBuf;

use log::{debug, info};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, Identifier, Keyword, ModModule, Stmt, StmtImportFrom,
};
use ruff_text_size::TextRange;

use crate::{
    analyzers::symbol_analyzer::SymbolAnalyzer,
    ast_builder::{self, expressions, statements},
    code_generator::{bundler::Bundler, module_registry::sanitize_module_name_for_identifier},
    types::{FxIndexMap, FxIndexSet},
};

/// Context for populating namespace with module symbols.
///
/// This struct encapsulates the state required by the namespace population function,
/// which was previously accessed directly from the `Bundler` struct.
pub struct NamespacePopulationContext<'a> {
    pub inlined_modules: &'a FxIndexSet<String>,
    pub module_exports: &'a FxIndexMap<String, Option<Vec<String>>>,
    pub tree_shaking_keep_symbols: &'a Option<FxIndexMap<String, FxIndexSet<String>>>,
    pub bundled_modules: &'a FxIndexSet<String>,
    pub namespace_assignments_made: &'a mut FxIndexSet<(String, String)>,
    pub modules_with_accessed_all: &'a FxIndexSet<(String, String)>,
    pub module_registry: &'a FxIndexMap<String, String>,
    pub module_asts: &'a Option<Vec<(String, ModModule, PathBuf, String)>>,
    pub symbols_populated_after_deferred: &'a FxIndexSet<(String, String)>,
    pub namespaces_with_initial_symbols: &'a FxIndexSet<String>,
    pub global_deferred_imports: &'a FxIndexMap<(String, String), String>,
    pub init_functions: &'a FxIndexMap<String, String>,
    pub resolver: &'a crate::resolver::ModuleResolver,
}

impl NamespacePopulationContext<'_> {
    /// Check if a symbol is kept by tree shaking.
    pub fn is_symbol_kept_by_tree_shaking(&self, module_name: &str, symbol_name: &str) -> bool {
        match &self.tree_shaking_keep_symbols {
            Some(kept_symbols) => kept_symbols
                .get(module_name)
                .is_some_and(|symbols| symbols.contains(symbol_name)),
            None => true, // No tree shaking, all symbols are kept
        }
    }
}

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
    bundler: &mut Bundler,
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

    // Also add all inlined modules - they have been initialized too
    for module_name in &bundler.inlined_modules {
        all_initialized_modules.insert(module_name.clone());
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

        // Add module assignment for this module (wrapper or inlined)
        let parent = parts[..parts.len() - 1].join(".");
        let attr = parts[parts.len() - 1];

        // Add if this is a wrapper module OR an inlined module
        if bundler.module_registry.contains_key(module_name)
            || bundler.inlined_modules.contains(module_name)
        {
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

        if bundler.module_registry.contains_key(&module_name)
            || bundler.inlined_modules.contains(&module_name)
        {
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
                } else if bundler.inlined_modules.contains(&module_name)
                    && !bundler.module_registry.contains_key(&module_name)
                {
                    // For inlined modules that are NOT wrapper modules, handle namespace assignment
                    handle_inlined_module_assignment(
                        bundler,
                        &parent,
                        &attr,
                        &module_name,
                        final_body,
                    );
                } else {
                    debug!("Module '{module_name}' is not in inlined_modules, checking assignment");
                    // Check if this would be a redundant self-assignment
                    let full_target = format!("{parent}.{attr}");
                    if full_target == module_name {
                        debug!(
                            "Skipping redundant self-assignment: {parent}.{attr} = {module_name}"
                        );
                    } else {
                        // This is a wrapper module - assign direct reference
                        debug!("Assigning wrapper module: {parent}.{attr} = {module_name}");

                        // DEBUGGING: Check what assignment is being created
                        let assignment = bundler.create_dotted_attribute_assignment(
                            &parent,
                            &attr,
                            &module_name,
                        );
                        debug!("Created assignment: {assignment:?}");

                        final_body.push(assignment);
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

            // Also skip if this is an inlined module - it will be handled elsewhere
            if bundler.inlined_modules.contains(&module_name) {
                debug!("Skipping intermediate namespace '{module_name}' - it's an inlined module");
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
    bundler: &Bundler,
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
                    result_stmts.extend(
                        crate::code_generator::module_registry::create_module_initialization_for_import(
                            module_name,
                            &bundler.module_registry,
                        ),
                    );
                }
                // Initialize the wrapper module if needed
                result_stmts.extend(
                    crate::code_generator::module_registry::create_module_initialization_for_import(
                        &full_module_path,
                        &bundler.module_registry,
                    ),
                );

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
                if let Some(module_renames) = symbol_renames.get(&full_module_path) {
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
/// Helper function used by `transform_namespace_package_imports`.
fn get_unique_name_with_module_suffix(base_name: &str, module_name: &str) -> String {
    let module_suffix = sanitize_module_name_for_identifier(module_name);
    format!("{base_name}_{module_suffix}")
}

/// Ensure a namespace exists, creating it and any parent namespaces if needed.
/// Returns statements to create any missing namespaces.
pub(super) fn ensure_namespace_exists(bundler: &mut Bundler, namespace_path: &str) -> Vec<Stmt> {
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
/// Creates: parent.child = `types.SimpleNamespace()`
pub(super) fn create_namespace_attribute(bundler: &mut Bundler, parent: &str, child: &str) -> Stmt {
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
pub(super) fn create_namespace_statements(bundler: &mut Bundler) -> Vec<Stmt> {
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
/// Returns None if the namespace was already created directly.
pub(super) fn create_namespace_for_inlined_module_static(
    bundler: &mut Bundler,
    module_name: &str,
    module_renames: &FxIndexMap<String, String>,
) -> Option<Stmt> {
    // If this namespace was already created directly (e.g., core.utils), skip creating underscore
    // variable
    if bundler.required_namespaces.contains(module_name) {
        log::debug!("Module '{module_name}' namespace already created directly, skipping");
        return None;
    }

    // Check if this module has forward references that would cause NameError
    // This happens when the module uses symbols from other modules that haven't been defined
    // yet
    let has_forward_references =
        bundler.check_module_has_forward_references(module_name, module_renames);

    if has_forward_references {
        log::debug!("Module '{module_name}' has forward references, creating empty namespace");
        // Create the namespace variable name
        let namespace_var = sanitize_module_name_for_identifier(module_name);

        // Create empty namespace = types.SimpleNamespace() to avoid forward reference errors
        return Some(statements::simple_assign(
            &namespace_var,
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
        ));
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
        if !bundler.is_symbol_kept_by_tree_shaking(module_name, original_name) {
            log::debug!(
                "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                 '{module_name}'"
            );
            continue;
        }

        seen_args.insert(original_name.clone());

        keywords.push(Keyword {
            node_index: AtomicNodeIndex::dummy(),
            arg: Some(Identifier::new(original_name, TextRange::default())),
            value: expressions::name(renamed_name, ExprContext::Load),
            range: TextRange::default(),
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
                if !bundler.is_symbol_kept_by_tree_shaking(module_name, export) {
                    log::debug!(
                        "Skipping tree-shaken export '{export}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                // This export wasn't renamed and wasn't already added, add it directly
                seen_args.insert(export.clone());
                keywords.push(Keyword {
                    node_index: AtomicNodeIndex::dummy(),
                    arg: Some(Identifier::new(export, TextRange::default())),
                    value: expressions::name(export, ExprContext::Load),
                    range: TextRange::default(),
                });
            }
        }
    }

    // Create the namespace variable name
    let namespace_var = sanitize_module_name_for_identifier(module_name);

    // namespace_var = types.SimpleNamespace(**kwargs)
    Some(statements::assign(
        vec![expressions::name(&namespace_var, ExprContext::Store)],
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
    ))
}

/// Handle assignment for inlined modules that are not wrapper modules.
///
/// This helper function reduces nesting in `generate_submodule_attributes_with_exclusions`
/// by extracting the logic for handling inlined module namespace assignments.
fn handle_inlined_module_assignment(
    bundler: &mut Bundler,
    parent: &str,
    attr: &str,
    module_name: &str,
    final_body: &mut Vec<Stmt>,
) {
    // Check if namespace has wrapper submodules
    let has_initialized_wrapper_submodules = bundler
        .module_registry
        .keys()
        .any(|wrapper_name| wrapper_name.starts_with(&format!("{module_name}.")));

    if has_initialized_wrapper_submodules {
        debug!(
            "Skipping namespace assignment for '{module_name}' - it already has initialized \
             wrapper submodules"
        );
        return;
    }

    // Check if namespace was already created directly
    if bundler.required_namespaces.contains(module_name) {
        debug!(
            "Skipping underscore namespace creation for '{module_name}' - already created directly"
        );
        return;
    }

    // Create namespace variable and assignment
    let namespace_var = sanitize_module_name_for_identifier(module_name);
    debug!("Assigning inlined module namespace: {parent}.{attr} = {namespace_var}");

    // Ensure namespace variable exists
    if !bundler.created_namespaces.contains(&namespace_var) {
        debug!("Creating empty namespace for module '{module_name}' before assignment");
        // Create empty namespace = types.SimpleNamespace()
        final_body.push(statements::simple_assign(
            &namespace_var,
            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
        ));
        bundler.created_namespaces.insert(namespace_var.clone());
    }

    // Create assignment: parent.attr = namespace_var
    final_body.push(statements::assign(
        vec![expressions::attribute(
            expressions::name(parent, ExprContext::Load),
            attr,
            ExprContext::Store,
        )],
        expressions::name(&namespace_var, ExprContext::Load),
    ));
}

/// Populate a namespace object with all symbols from a given module, applying renames.
///
/// This function generates AST statements to populate a namespace object with symbols
/// from a module, handling tree-shaking, re-exports, and symbol renaming.
pub fn populate_namespace_with_module_symbols(
    ctx: &mut NamespacePopulationContext,
    target_name: &str,
    module_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    // Get the module's exports
    if let Some(exports) = ctx.module_exports.get(module_name).and_then(|e| e.as_ref()) {
        // Build the namespace access expression for the target
        let parts: Vec<&str> = target_name.split('.').collect();

        // First, add __all__ attribute to the namespace
        // Create the target expression for __all__
        let all_target = expressions::dotted_name(&parts, ExprContext::Load);

        // Filter exports to only include symbols that survived tree-shaking
        let filtered_exports = SymbolAnalyzer::filter_exports_by_tree_shaking(
            exports,
            module_name,
            ctx.tree_shaking_keep_symbols.as_ref(),
            true,
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
            .any(|(_, alias)| alias == target_name)
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

            info!(
                "Created __all__ assignment for namespace '{target_name}' with exports: \
                 {filtered_exports:?} (accessed in code)"
            );
        } else {
            debug!(
                "Skipping __all__ assignment for namespace '{target_name}' - not accessed in code"
            );
        }

        // Skip individual symbol assignments if this namespace was already created with initial
        // symbols
        if ctx.namespaces_with_initial_symbols.contains(module_name) {
            debug!(
                "Skipping individual symbol assignments for '{module_name}' - namespace created \
                 with initial symbols"
            );
            return result_stmts;
        }

        // For each exported symbol that survived tree-shaking, add it to the namespace
        'symbol_loop: for symbol in &filtered_exports {
            let symbol_name = symbol.as_str();

            // For re-exported symbols, check if the original symbol is kept by tree-shaking
            let should_include = if ctx.tree_shaking_keep_symbols.is_some() {
                // First check if this symbol is directly defined in this module
                if ctx.is_symbol_kept_by_tree_shaking(module_name, symbol_name) {
                    true
                } else {
                    // If not, check if this is a re-exported symbol from another module
                    // For modules with __all__, we always include symbols that are re-exported
                    // even if they're not directly defined in the module
                    let module_has_all_export = ctx
                        .module_exports
                        .get(module_name)
                        .and_then(|exports| exports.as_ref())
                        .is_some_and(|exports| exports.contains(&symbol_name.to_string()));

                    if module_has_all_export {
                        debug!(
                            "Including re-exported symbol {symbol_name} from module {module_name} \
                             (in __all__)"
                        );
                        true
                    } else {
                        false
                    }
                }
            } else {
                // No tree-shaking, include everything
                true
            };

            if !should_include {
                debug!(
                    "Skipping namespace assignment for {module_name}.{symbol_name} - removed by \
                     tree-shaking"
                );
                continue;
            }

            // Check if this symbol is actually a submodule
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let is_bundled_submodule = ctx.bundled_modules.contains(&full_submodule_path);
            let is_inlined = ctx.inlined_modules.contains(&full_submodule_path);
            let uses_init_function = ctx.module_registry.contains_key(&full_submodule_path);

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
                    if let Some(submodule_exports) = ctx
                        .module_exports
                        .get(&full_submodule_path)
                        .and_then(|e| e.as_ref())
                        && submodule_exports.contains(&symbol_name.to_string())
                    {
                        // The submodule exports a symbol with the same name as itself
                        // Check if the parent module re-exports this symbol
                        debug!(
                            "Submodule '{full_submodule_path}' exports symbol '{symbol_name}' \
                             with same name"
                        );

                        // Get the renamed symbol from the submodule
                        if let Some(submodule_renames) = symbol_renames.get(&full_submodule_path)
                            && let Some(renamed) = submodule_renames.get(symbol_name)
                        {
                            info!(
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
                }

                // Skip other submodules - they are handled separately
                // This prevents creating invalid assignments like `mypkg.compat = compat`
                // when `compat` is a submodule, not a local variable
                continue;
            }

            // Get the renamed symbol if it exists
            let actual_symbol_name = if let Some(module_renames) = symbol_renames.get(module_name) {
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

            // Check if this specific symbol was already populated after deferred imports
            // This happens for modules that had forward references and were populated later
            if ctx
                .symbols_populated_after_deferred
                .contains(&(module_name.to_string(), symbol_name.to_string()))
                && target_name == sanitize_module_name_for_identifier(module_name).as_str()
            {
                debug!(
                    "Skipping symbol assignment {target_name}.{symbol_name} = \
                     {actual_symbol_name} - this specific symbol was already populated after \
                     deferred imports"
                );
                continue;
            }

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
                    "Skipping duplicate namespace assignment: {target_name}.{symbol_name} = \
                     {actual_symbol_name}"
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
                    && let Some(Some(parent_exports)) = ctx.module_exports.get(parent_module)
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
                            "Skipping duplicate namespace assignment: {target_name}.{symbol_name} \
                             = {actual_symbol_name} (already exists in result_stmts) - in \
                             populate_namespace_with_module_symbols"
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
                let key = (module_name.to_string(), symbol_name.to_string());
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
            if ctx.module_registry.contains_key(module_name) {
                debug!(
                    "Module '{module_name}' is a wrapper module, checking if symbol \
                     '{symbol_name}' is imported from inlined submodule"
                );
                // This is a wrapper module - check if symbol is re-exported from inlined
                // submodule
                if let Some(module_asts) = ctx.module_asts.as_ref() {
                    // Find the module's AST to check its imports
                    if let Some((_, ast, module_path, _)) = module_asts
                        .iter()
                        .find(|(name, _, _, _)| name == module_name)
                    {
                        // Check if this symbol is imported from an inlined submodule
                        for stmt in &ast.body {
                            if let Stmt::ImportFrom(import_from) = stmt {
                                let resolved_module = if import_from.level > 0 {
                                    ctx.resolver.resolve_relative_to_absolute_module_name(
                                        import_from.level,
                                        import_from
                                            .module
                                            .as_ref()
                                            .map(ruff_python_ast::Identifier::as_str),
                                        module_path,
                                    )
                                } else {
                                    import_from.module.as_ref().map(|m| m.as_str().to_string())
                                };
                                if let Some(ref resolved) = resolved_module {
                                    // Check if the resolved module is inlined
                                    if ctx.inlined_modules.contains(resolved) {
                                        // Check if our symbol is in this import
                                        for alias in &import_from.names {
                                            if alias.name.as_str() == symbol_name {
                                                debug!(
                                                    "Skipping namespace assignment for \
                                                     '{symbol_name}' - already imported from \
                                                     inlined module '{resolved}' and added as \
                                                     module attribute"
                                                );
                                                // Skip this symbol - it's already added via
                                                // module attributes
                                                continue 'symbol_loop;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Check if this is a submodule that uses an init function
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let uses_init_function = ctx
                .module_registry
                .get(&full_submodule_path)
                .and_then(|synthetic_name| ctx.init_functions.get(synthetic_name))
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
            let is_inlined_submodule = ctx.inlined_modules.contains(&full_submodule_path);
            if is_inlined_submodule {
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it's an \
                     inlined submodule"
                );
                continue;
            }

            // Check if this is a submodule at all (vs a symbol defined in the module)
            let is_bundled_submodule = ctx.bundled_modules.contains(&full_submodule_path);
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

            info!(
                "Creating namespace assignment: {target_name}.{symbol_name} = \
                 {actual_symbol_name} (in populate_namespace_with_module_symbols)"
            );

            // Now add the symbol as an attribute (e.g., greetings.greeting.get_greeting =
            // get_greeting_greetings_greeting)
            result_stmts.push(statements::assign(
                vec![expressions::attribute(
                    target,
                    symbol_name,
                    ExprContext::Store,
                )],
                expressions::name(&actual_symbol_name, ExprContext::Load),
            ));

            // Track that we've made this assignment
            let assignment_key = (target_name.to_string(), symbol_name.to_string());
            ctx.namespace_assignments_made.insert(assignment_key);
        }
    }

    result_stmts
}
