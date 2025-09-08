//! Module registry management for code bundling
//!
//! This module handles:
//! - Module naming and identifier generation
//! - Module attribute assignments
//! - Module initialization functions

use ruff_python_ast::{Expr, ExprContext, Stmt, StmtImportFrom};
use ruff_python_stdlib::keyword::is_keyword;

use crate::{
    ast_builder,
    types::{FxIndexMap, FxIndexSet},
};

/// Create module initialization statements for wrapper modules
pub fn create_module_initialization_for_import(
    module_id: crate::resolver::ModuleId,
    module_init_functions: &FxIndexMap<crate::resolver::ModuleId, String>,
    resolver: &crate::resolver::ModuleResolver,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // Check if this is a wrapper module that needs initialization
    if let Some(init_func_name) = module_init_functions.get(&module_id) {
        let Some(module_name) = resolver.get_module_name(module_id) else {
            log::warn!(
                "Missing module name for id {}, skipping init emission",
                module_id.as_u32()
            );
            return stmts;
        };

        // Call the init function with the module as the self argument
        let module_var = get_module_var_identifier(module_id, resolver);
        let init_call = ast_builder::expressions::call(
            ast_builder::expressions::name(init_func_name, ExprContext::Load),
            vec![ast_builder::expressions::name(
                &module_var,
                ExprContext::Load,
            )],
            vec![],
        );

        // Create assignment to possibly dotted path: <pkg.subpkg.module> = init_call(...)
        stmts.push(ast_builder::statements::assign_attribute_path(
            &module_name,
            init_call,
        ));
    }

    stmts
}

/// Get synthetic module name
pub fn get_synthetic_module_name(module_name: &str, content_hash: &str) -> String {
    let module_name_escaped = sanitize_module_name_for_identifier(module_name);
    // Use first 6 characters of content hash for readability
    let short_hash = &content_hash[..6];
    format!("__cribo_{short_hash}_{module_name_escaped}")
}

/// Get the variable identifier for a module using its `ModuleId`
/// This ensures symlinks resolve to the same variable name
pub fn get_module_var_identifier(
    module_id: crate::resolver::ModuleId,
    resolver: &crate::resolver::ModuleResolver,
) -> String {
    // Get the canonical module name for this ID
    let module_name = resolver
        .get_module_name(module_id)
        .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));

    sanitize_module_name_for_identifier(&module_name)
}

/// Sanitize a module name for use in a Python identifier
/// This is a simple character replacement - collision handling should be done by the caller
pub fn sanitize_module_name_for_identifier(name: &str) -> String {
    let mut result = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();

    // If the name starts with a digit, prefix with underscore to make it a valid identifier
    if result.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        result = format!("_{result}");
    }

    // Check if the result is a Python keyword and append underscore if so
    if is_keyword(&result) {
        result.push('_');
    }

    result
}

/// Generate a unique symbol name to avoid conflicts
pub fn generate_unique_name(base_name: &str, existing_symbols: &FxIndexSet<String>) -> String {
    if !existing_symbols.contains(base_name) {
        return base_name.to_string();
    }

    // Try adding numeric suffixes
    for i in 1..1000 {
        let candidate = format!("{base_name}_{i}");
        if !existing_symbols.contains(&candidate) {
            return candidate;
        }
    }

    // Fallback with module prefix
    format!("__cribo_renamed_{base_name}")
}

/// Create a module attribute assignment statement
pub fn create_module_attr_assignment(module_var: &str, attr_name: &str) -> Stmt {
    ast_builder::statements::assign_attribute(
        module_var,
        attr_name,
        ast_builder::expressions::name(attr_name, ExprContext::Load),
    )
}

/// Create a module attribute assignment statement with a specific value
pub fn create_module_attr_assignment_with_value(
    module_var: &str,
    attr_name: &str,
    value_name: &str,
) -> Stmt {
    ast_builder::statements::assign_attribute(
        module_var,
        attr_name,
        ast_builder::expressions::name(value_name, ExprContext::Load),
    )
}

/// Create a reassignment statement (`original_name` = `renamed_name`)
pub fn create_reassignment(original_name: &str, renamed_name: &str) -> Stmt {
    ast_builder::statements::simple_assign(
        original_name,
        ast_builder::expressions::name(renamed_name, ExprContext::Load),
    )
}

/// Information about a namespace that needs to be created
pub struct NamespaceRequirement {
    pub path: String,
    pub var_name: String,
}

/// Helper function to create an assignment if it doesn't conflict with stdlib names
fn create_assignment_if_no_stdlib_conflict(
    local_name: &str,
    value_name: &str,
    assignments: &mut Vec<Stmt>,
    python_version: u8,
) {
    // Check if the name itself is a stdlib module
    if crate::resolver::is_stdlib_module(local_name, python_version) {
        log::debug!(
            "Skipping assignment '{local_name} = {value_name}' - would conflict with stdlib name \
             '{local_name}'"
        );
    } else {
        log::debug!("Creating assignment '{local_name} = {value_name}' - no stdlib conflict");
        assignments.push(ast_builder::statements::simple_assign(
            local_name,
            ast_builder::expressions::name(value_name, ExprContext::Load),
        ));
    }
}

/// Initialize a submodule if it hasn't been initialized yet
///
/// This helper function checks if a module initialization already exists in the assignments
/// and adds it if needed, updating the tracking sets accordingly.
pub fn initialize_submodule_if_needed(
    module_id: crate::resolver::ModuleId,
    module_init_functions: &FxIndexMap<crate::resolver::ModuleId, String>,
    resolver: &crate::resolver::ModuleResolver,
    assignments: &mut Vec<Stmt>,
    locally_initialized: &mut FxIndexSet<crate::resolver::ModuleId>,
    initialized_modules: &mut FxIndexSet<crate::resolver::ModuleId>,
) {
    use crate::code_generator::expression_handlers;

    let module_path = resolver
        .get_module_name(module_id)
        .unwrap_or_else(|| "<unknown>".to_string());

    // Check if we already have this module initialization in assignments
    let already_initialized = assignments.iter().any(|stmt| {
        if let Stmt::Assign(assign) = stmt
            && assign.targets.len() == 1
            && let Expr::Attribute(attr) = &assign.targets[0]
            && let Expr::Call(call) = &assign.value.as_ref()
            && let Expr::Name(func_name) = &call.func.as_ref()
            && is_init_function(func_name.id.as_str())
        {
            let attr_path = expression_handlers::extract_attribute_path(attr);
            attr_path == module_path
        } else {
            false
        }
    });

    if !already_initialized {
        assignments.extend(create_module_initialization_for_import(
            module_id,
            module_init_functions,
            resolver,
        ));
    }
    locally_initialized.insert(module_id);
    initialized_modules.insert(module_id);
}

/// Parameters for creating assignments for inlined imports
pub struct InlinedImportParams<'a> {
    pub symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    pub module_registry: Option<&'a crate::orchestrator::ModuleRegistry>,
    pub inlined_modules: &'a FxIndexSet<crate::resolver::ModuleId>,
    pub bundled_modules: &'a FxIndexSet<crate::resolver::ModuleId>,
    pub resolver: &'a crate::resolver::ModuleResolver,
    pub python_version: u8,
    pub is_wrapper_init: bool,
    pub tree_shaking_check: Option<&'a dyn Fn(crate::resolver::ModuleId, &str) -> bool>,
}

/// Create assignments for inlined imports
/// Returns (statements, `namespace_requirements`)
pub fn create_assignments_for_inlined_imports(
    import_from: &StmtImportFrom,
    module_name: &str,
    params: InlinedImportParams,
) -> (Vec<Stmt>, Vec<NamespaceRequirement>) {
    let mut assignments = Vec::new();
    let mut namespace_requirements = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name);

        // Skip wildcard imports - they are handled separately by the caller
        // Wildcard imports don't create individual assignments in wrapper modules
        if imported_name == "*" {
            log::debug!("Skipping wildcard import from '{module_name}' - handled separately");
            continue;
        }

        // Check if we're importing a module itself (not a symbol from it)
        // This happens when the imported name refers to a submodule
        let full_module_path = format!("{module_name}.{imported_name}");

        // Check if this is a module import
        // First check if it's a wrapped module
        if let Some(module_id) = params.resolver.get_module_id_by_name(&full_module_path) {
            if params
                .module_registry
                .is_some_and(|reg| reg.contains_module(&module_id))
            {
                // Skip wrapped modules - they will be handled as deferred imports
                log::debug!("Module '{full_module_path}' is a wrapped module, deferring import");
                continue;
            } else if params.inlined_modules.contains(&module_id)
                || params.bundled_modules.contains(&module_id)
            {
                // Create a namespace object for the inlined module
                log::debug!(
                    "Creating namespace object for module '{imported_name}' imported from \
                     '{module_name}' - module was inlined"
                );

                // Record that we need a namespace for this module
                let sanitized_name = get_module_var_identifier(module_id, params.resolver);

                namespace_requirements.push(NamespaceRequirement {
                    path: full_module_path.clone(),
                    var_name: sanitized_name.clone(),
                });

                // If local name differs from sanitized name, create alias
                // But skip if it would conflict with a stdlib name in scope
                if local_name.as_str() != sanitized_name {
                    create_assignment_if_no_stdlib_conflict(
                        local_name.as_str(),
                        &sanitized_name,
                        &mut assignments,
                        params.python_version,
                    );
                }
            } else {
                // Regular symbol import
                // Try to resolve the parent module; it may be absent for namespace packages
                let module_id_opt = params.resolver.get_module_id_by_name(module_name);

                // Apply tree-shaking outside wrapper init
                if !params.is_wrapper_init
                    && let Some(id) = module_id_opt
                    && let Some(check_fn) = params.tree_shaking_check
                    && !check_fn(id, imported_name)
                {
                    log::debug!(
                        "Skipping assignment for tree-shaken symbol '{imported_name}' from module \
                         '{module_name}' (outside wrapper init)"
                    );
                    continue;
                }

                // Determine the actual (possibly renamed) symbol name if we have rename info
                let actual_name = if let Some(id) = module_id_opt
                    && let Some(module_renames) = params.symbol_renames.get(&id)
                {
                    module_renames
                        .get(imported_name)
                        .map_or(imported_name, std::string::String::as_str)
                } else {
                    imported_name
                };

                // When we're inside a wrapper init function and importing from an inlined module,
                // we need to qualify the symbol with the module's namespace variable. Only possible
                // if the parent module actually exists and was inlined.
                let source_ref = if params.is_wrapper_init
                    && module_id_opt.is_some_and(|id| params.inlined_modules.contains(&id))
                {
                    // The module is inlined, so its symbols are attached to a namespace object
                    let ns = module_id_opt.map_or_else(
                        || sanitize_module_name_for_identifier(module_name),
                        |id| get_module_var_identifier(id, params.resolver),
                    );
                    format!("{ns}.{actual_name}")
                } else {
                    actual_name.to_string()
                };

                // Only create assignment if the names are different or we need qualification
                // But skip if it would conflict with a stdlib name in scope
                if local_name.as_str() != source_ref {
                    create_assignment_if_no_stdlib_conflict(
                        local_name.as_str(),
                        &source_ref,
                        &mut assignments,
                        params.python_version,
                    );
                } else if params.is_wrapper_init
                    && module_id_opt.is_some_and(|id| params.inlined_modules.contains(&id))
                {
                    // Even if names match, we need the assignment to access through namespace
                    create_assignment_if_no_stdlib_conflict(
                        local_name.as_str(),
                        &source_ref,
                        &mut assignments,
                        params.python_version,
                    );
                }
            }
        } else {
            // Module doesn't exist, treat as regular symbol import
            // Check if this symbol was renamed during inlining
            let parent_module_id = params.resolver.get_module_id_by_name(module_name);

            // IMPORTANT: When we're inside a wrapper init function, we must not skip
            // assignments based on tree-shaking. The wrapper's body may still reference
            // these names (e.g., as base classes) even if they aren't exported or used
            // by the entry module. Skipping here can lead to NameError at runtime.
            // Therefore, only apply the tree-shaking check when we're NOT in a
            // wrapper init context.
            if !params.is_wrapper_init
                && let Some(id) = parent_module_id
                && let Some(check_fn) = params.tree_shaking_check
                && !check_fn(id, imported_name)
            {
                log::debug!(
                    "Skipping assignment for tree-shaken symbol '{imported_name}' from module \
                     '{module_name}' (outside wrapper init)"
                );
                continue;
            }

            let actual_name = if let Some(id) = parent_module_id
                && let Some(module_renames) = params.symbol_renames.get(&id)
            {
                module_renames
                    .get(imported_name)
                    .map_or(imported_name, std::string::String::as_str)
            } else {
                imported_name
            };

            // When we're inside a wrapper init function and importing from an inlined module,
            // we need to qualify the symbol with the module's namespace variable
            let source_ref = if params.is_wrapper_init
                && parent_module_id.is_some_and(|id| params.inlined_modules.contains(&id))
            {
                // The module is inlined, so its symbols are attached to a namespace object
                let ns = parent_module_id.map_or_else(
                    || sanitize_module_name_for_identifier(module_name),
                    |id| get_module_var_identifier(id, params.resolver),
                );
                format!("{ns}.{actual_name}")
            } else {
                actual_name.to_string()
            };

            // Only create assignment if the names are different or we need qualification
            // But skip if it would conflict with a stdlib name in scope
            if local_name.as_str() != source_ref {
                create_assignment_if_no_stdlib_conflict(
                    local_name.as_str(),
                    &source_ref,
                    &mut assignments,
                    params.python_version,
                );
            } else if params.is_wrapper_init
                && parent_module_id.is_some_and(|id| params.inlined_modules.contains(&id))
            {
                // Even if names match, we need the assignment to access through namespace
                create_assignment_if_no_stdlib_conflict(
                    local_name.as_str(),
                    &source_ref,
                    &mut assignments,
                    params.python_version,
                );
            }
        }
    }

    (assignments, namespace_requirements)
}

/// Prefix for all cribo-generated init-related names
const CRIBO_INIT_PREFIX: &str = "_cribo_init_";

/// The init result variable name
pub const INIT_RESULT_VAR: &str = "__cribo_init_result";

/// The module `SimpleNamespace` variable name in init functions
/// Use single underscore to prevent Python mangling
/// Generate init function name from synthetic name
pub fn get_init_function_name(synthetic_name: &str) -> String {
    format!("{CRIBO_INIT_PREFIX}{synthetic_name}")
}

/// Check if a function name is an init function
pub fn is_init_function(name: &str) -> bool {
    name.starts_with(CRIBO_INIT_PREFIX)
}

/// Register a module with its synthetic name and init function
/// Returns (`synthetic_name`, `init_func_name`)
pub fn register_module(
    module_id: crate::resolver::ModuleId,
    module_name: &str,
    content_hash: &str,
    module_synthetic_names: &mut FxIndexMap<crate::resolver::ModuleId, String>,
    module_init_functions: &mut FxIndexMap<crate::resolver::ModuleId, String>,
) -> (String, String) {
    // Generate synthetic name
    let synthetic_name = get_synthetic_module_name(module_name, content_hash);

    // Register module with synthetic name
    module_synthetic_names.insert(module_id, synthetic_name.clone());

    // Register init function
    let init_func_name = get_init_function_name(&synthetic_name);
    module_init_functions.insert(module_id, init_func_name.clone());

    (synthetic_name, init_func_name)
}

/// Check if a module is a wrapper submodule (not inlined)
///
/// A module is considered a wrapper submodule if:
/// - It exists in the module registry (meaning it has an init function)
/// - It is NOT in the inlined modules set
pub fn is_wrapper_submodule(
    module_id: crate::resolver::ModuleId,
    module_info_registry: Option<&crate::orchestrator::ModuleRegistry>,
    inlined_modules: &FxIndexSet<crate::resolver::ModuleId>,
) -> bool {
    module_info_registry.is_some_and(|reg| reg.contains_module(&module_id))
        && !inlined_modules.contains(&module_id)
}
