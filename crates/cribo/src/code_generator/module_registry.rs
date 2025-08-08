//! Module registry management for code bundling
//!
//! This module handles:
//! - Module naming and identifier generation
//! - Module attribute assignments
//! - Module initialization functions

use log::debug;
use ruff_python_ast::{Expr, ExprContext, ModModule, Stmt, StmtImport, StmtImportFrom};
use ruff_python_stdlib::keyword::is_keyword;

use crate::{
    ast_builder,
    types::{FxIndexMap, FxIndexSet},
};

/// Generate registries and hook
pub fn generate_registries_and_hook() -> Vec<Stmt> {
    // No longer needed - we don't use sys.modules or import hooks
    Vec::new()
}

/// Create module initialization statements for wrapper modules
pub fn create_module_initialization_for_import(
    module_name: &str,
    module_registry: &FxIndexMap<String, String>,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // Check if this is a wrapper module that needs initialization
    if let Some(synthetic_name) = module_registry.get(module_name) {
        // Generate the init call
        let init_func_name = get_init_function_name(synthetic_name);

        // Call the init function and get the result
        let init_call = ast_builder::expressions::call(
            ast_builder::expressions::name(&init_func_name, ExprContext::Load),
            vec![],
            vec![],
        );

        // Create assignment statement: module_name = init_func()
        stmts.push(ast_builder::statements::simple_assign(
            module_name,
            init_call,
        ));
    }

    stmts
}

/// Generate module init call
pub fn generate_module_init_call(
    _synthetic_name: &str,
    module_name: &str,
    init_func_name: Option<&str>,
    module_registry: &FxIndexMap<String, String>,
    generate_merge_module_attributes: impl Fn(&mut Vec<Stmt>, &str, &str),
) -> Vec<Stmt> {
    let mut statements = Vec::new();

    if let Some(init_func_name) = init_func_name {
        // Check if this module is a parent namespace that already exists
        // This happens when a module like 'services.auth' has both:
        // 1. Its own __init__.py (wrapper module)
        // 2. Submodules like 'services.auth.manager'
        let is_parent_namespace = module_registry
            .iter()
            .any(|(name, _)| name != module_name && name.starts_with(&format!("{module_name}.")));

        if is_parent_namespace {
            // For parent namespaces, we need to merge attributes instead of overwriting
            // Generate code that calls the init function and merges its attributes
            debug!("Module '{module_name}' is a parent namespace - generating merge code");

            // First, create a variable to hold the init result
            statements.push(ast_builder::statements::simple_assign(
                INIT_RESULT_VAR,
                ast_builder::expressions::call(
                    ast_builder::expressions::name(init_func_name, ExprContext::Load),
                    vec![],
                    vec![],
                ),
            ));

            // Generate the merge attributes code
            generate_merge_module_attributes(&mut statements, module_name, INIT_RESULT_VAR);

            // Assign the init result to the module variable
            statements.push(ast_builder::statements::simple_assign(
                module_name,
                ast_builder::expressions::name(INIT_RESULT_VAR, ExprContext::Load),
            ));
        } else {
            // Direct assignment for modules that aren't parent namespaces
            let target_expr = if module_name.contains('.') {
                // For dotted modules like models.base, create an attribute expression
                let parts: Vec<&str> = module_name.split('.').collect();
                ast_builder::expressions::dotted_name(&parts, ExprContext::Store)
            } else {
                // For simple modules, use direct name
                ast_builder::expressions::name(module_name, ExprContext::Store)
            };

            // Generate: module_name = <cribo_init_prefix>synthetic_name()
            // or: parent.child = <cribo_init_prefix>synthetic_name()
            statements.push(ast_builder::statements::assign(
                vec![target_expr],
                ast_builder::expressions::call(
                    ast_builder::expressions::name(init_func_name, ExprContext::Load),
                    vec![],
                    vec![],
                ),
            ));
        }
    } else {
        statements.push(ast_builder::statements::pass());
    }

    statements
}

/// Get synthetic module name
pub fn get_synthetic_module_name(module_name: &str, content_hash: &str) -> String {
    let module_name_escaped = sanitize_module_name_for_identifier(module_name);
    // Use first 6 characters of content hash for readability
    let short_hash = &content_hash[..6];
    format!("__cribo_{short_hash}_{module_name_escaped}")
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

/// Check if a local name conflicts with any symbol in the module
pub fn check_local_name_conflict(ast: &ModModule, name: &str) -> bool {
    for stmt in &ast.body {
        match stmt {
            Stmt::ClassDef(class_def) => {
                if class_def.name.as_str() == name {
                    return true;
                }
            }
            Stmt::FunctionDef(func_def) => {
                if func_def.name.as_str() == name {
                    return true;
                }
            }
            Stmt::Assign(assign_stmt) => {
                for target in &assign_stmt.targets {
                    if let Expr::Name(name_expr) = target
                        && name_expr.id.as_str() == name
                    {
                        return true;
                    }
                }
            }
            Stmt::Import(StmtImport { names, .. }) => {
                // Check import statements that remain in the module (third-party imports)
                for alias in names {
                    let local_name = alias.asname.as_ref().unwrap_or(&alias.name);
                    if local_name.as_str() == name {
                        return true;
                    }
                }
            }
            Stmt::ImportFrom(StmtImportFrom { names, .. }) => {
                // Check from imports that remain in the module (third-party imports)
                for alias in names {
                    let local_name = alias.asname.as_ref().unwrap_or(&alias.name);
                    if local_name.as_str() == name {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
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

/// Create assignments for inlined imports
/// Returns (statements, `namespace_requirements`)
#[allow(clippy::too_many_arguments)]
pub fn create_assignments_for_inlined_imports(
    import_from: &StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
    module_registry: &FxIndexMap<String, String>,
    inlined_modules: &FxIndexSet<String>,
    bundled_modules: &FxIndexSet<String>,
) -> (Vec<Stmt>, Vec<NamespaceRequirement>) {
    let mut assignments = Vec::new();
    let mut namespace_requirements = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name);

        // Check if we're importing a module itself (not a symbol from it)
        // This happens when the imported name refers to a submodule
        let full_module_path = format!("{module_name}.{imported_name}");

        // Check if this is a module import
        // First check if it's a wrapped module
        if module_registry.contains_key(&full_module_path) {
            // Skip wrapped modules - they will be handled as deferred imports
            log::debug!("Module '{full_module_path}' is a wrapped module, deferring import");
            continue;
        } else if inlined_modules.contains(&full_module_path)
            || bundled_modules.contains(&full_module_path)
        {
            // Create a namespace object for the inlined module
            log::debug!(
                "Creating namespace object for module '{imported_name}' imported from \
                 '{module_name}' - module was inlined"
            );

            // Record that we need a namespace for this module
            let sanitized_name = sanitize_module_name_for_identifier(&full_module_path);

            namespace_requirements.push(NamespaceRequirement {
                path: full_module_path.clone(),
                var_name: sanitized_name.clone(),
            });

            // If local name differs from sanitized name, create alias
            if local_name.as_str() != sanitized_name {
                assignments.push(ast_builder::statements::simple_assign(
                    local_name.as_str(),
                    ast_builder::expressions::name(&sanitized_name, ExprContext::Load),
                ));
            }
        } else {
            // Regular symbol import
            // Check if this symbol was renamed during inlining
            let actual_name = if let Some(module_renames) = symbol_renames.get(module_name) {
                module_renames
                    .get(imported_name)
                    .map_or(imported_name, std::string::String::as_str)
            } else {
                imported_name
            };

            // Only create assignment if the names are different
            if local_name.as_str() != actual_name {
                log::debug!(
                    "Creating assignment: {local_name} = {actual_name} (from inlined module \
                     '{module_name}')"
                );

                let assignment = ast_builder::statements::simple_assign(
                    local_name.as_str(),
                    ast_builder::expressions::name(actual_name, ExprContext::Load),
                );
                assignments.push(assignment);
            }
        }
    }

    (assignments, namespace_requirements)
}

/// Prefix for all cribo-generated init-related names
const CRIBO_INIT_PREFIX: &str = "__cribo_init_";

/// The init result variable name
pub const INIT_RESULT_VAR: &str = "__cribo_init_result";

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
    module_name: &str,
    content_hash: &str,
    module_registry: &mut FxIndexMap<String, String>,
    init_functions: &mut FxIndexMap<String, String>,
) -> (String, String) {
    // Generate synthetic name
    let synthetic_name = get_synthetic_module_name(module_name, content_hash);

    // Register module with synthetic name
    module_registry.insert(module_name.to_string(), synthetic_name.clone());

    // Register init function
    let init_func_name = get_init_function_name(&synthetic_name);
    init_functions.insert(synthetic_name.clone(), init_func_name.clone());

    (synthetic_name, init_func_name)
}
