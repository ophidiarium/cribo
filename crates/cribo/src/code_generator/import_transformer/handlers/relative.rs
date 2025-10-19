use ruff_python_ast::{ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{bundler::Bundler, module_registry::sanitize_module_name_for_identifier},
};

pub(in crate::code_generator::import_transformer) fn handle_unbundled_relative_import(
    _bundler: &Bundler<'_>,
    import_from: &StmtImportFrom,
    module_name: &str,
    current_module: &str,
) -> Vec<Stmt> {
    // Special case: imports from __main__ modules that aren't the entry
    // These might not be discovered if the __main__.py wasn't explicitly imported
    if module_name.ends_with(".__main__") {
        log::warn!(
            "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which is not \
             bundled. This __main__ module may not have been discovered during bundling.",
            ".".repeat(import_from.level as usize),
            import_from
                .module
                .as_ref()
                .map(|m| format!("{} ", m.as_str()))
                .unwrap_or_default(),
            import_from
                .names
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            current_module,
            module_name
        );
        // Return the original import and let it fail at runtime if the module doesn't exist
        // This is better than panicking during bundling
        return vec![Stmt::ImportFrom(import_from.clone())];
    }

    // Original panic for other non-entry relative imports
    panic!(
        "Relative import 'from {}{}import {:?}' in module '{}' resolves to '{}' which is not \
         bundled or inlined. This is a bug - relative imports are always first-party and should \
         be bundled.",
        ".".repeat(import_from.level as usize),
        import_from
            .module
            .as_ref()
            .map(|m| format!("{} ", m.as_str()))
            .unwrap_or_default(),
        import_from
            .names
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        current_module,
        module_name
    );
}

/// Transform relative import aliases for bundled modules
///
/// This function handles relative imports by creating appropriate assignments
/// for symbols imported from parent packages or submodules.
///
/// # Arguments
/// * `bundler` - Bundler instance for module resolution
/// * `import_from` - The import statement to transform
/// * `parent_package` - The parent package name for relative imports
/// * `current_module` - Current module being processed
/// * `result` - Vector to append generated statements to
/// * `add_module_attr` - Whether to add module attributes for non-private symbols
pub(crate) fn transform_relative_import_aliases(
    bundler: &Bundler<'_>,
    import_from: &StmtImportFrom,
    parent_package: &str,
    current_module: &str,
    result: &mut Vec<Stmt>,
    add_module_attr: bool,
) {
    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        if imported_name == "*" {
            continue;
        }

        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

        // Try to resolve the import to an actual file path
        // First, construct the expected module name for resolution
        let full_module_name = if parent_package.is_empty() {
            imported_name.to_owned()
        } else {
            format!("{parent_package}.{imported_name}")
        };

        log::debug!("Attempting to resolve module '{full_module_name}' to a path");

        // Try to resolve the module to a path and then to a ModuleId
        let module_id = if let Ok(Some(module_path)) =
            bundler.resolver.resolve_module_path(&full_module_name)
        {
            log::debug!(
                "Resolved '{full_module_name}' to path: {}",
                module_path.display()
            );
            bundler.resolver.get_module_id_by_path(&module_path)
        } else {
            log::debug!(
                "Could not resolve '{full_module_name}' to a path - might be a symbol import, not \
                 a module"
            );
            None
        };

        // For relative imports in bundled code, we need to distinguish between:
        // 1. Importing a submodule (e.g., from . import errors where errors.py exists)
        // 2. Importing a symbol from parent package (e.g., from . import get_console where
        //    get_console is a function)

        // This is a critical error - the module was registered without its package prefix
        assert!(
            !parent_package.is_empty(),
            "CRITICAL: Module '{current_module}' is missing its package prefix. Relative import \
             'from . import {imported_name}' cannot be resolved. This is a bug in module \
             discovery - the module should have been registered with its full package name."
        );

        // If we couldn't find a module, this might be a symbol import from the parent package
        // In that case, we should just create a simple assignment
        let Some(module_id) = module_id else {
            log::debug!(
                "Import '{imported_name}' in module '{current_module}' is likely a symbol from \
                 parent package, not a submodule"
            );

            // When importing a symbol from the parent package, we need to check if the parent
            // is inlined and if the symbol needs to be accessed through the parent's namespace
            let parent_module_id = bundler.get_module_id(parent_package);

            // Common helper to add module attribute if exportable
            let add_module_attribute_if_needed = |result: &mut Vec<Stmt>| {
                if add_module_attr && !local_name.starts_with('_') {
                    let current_module_var = sanitize_module_name_for_identifier(current_module);
                    result.push(
                        crate::code_generator::module_registry::create_module_attr_assignment(
                            &current_module_var,
                            local_name,
                        ),
                    );
                }
            };

            // Check if parent is inlined and if we're in a wrapper context
            // In wrapper init functions, symbols from inlined parent modules need special handling
            if let Some(parent_id) = parent_module_id
                && bundler.inlined_modules.contains(&parent_id)
            {
                // The parent module is inlined, so its symbols are in the global scope
                // We need to access them through the parent's namespace object
                let parent_namespace = sanitize_module_name_for_identifier(parent_package);

                log::debug!(
                    "Parent package '{parent_package}' is inlined, accessing symbol \
                     '{imported_name}' through namespace '{parent_namespace}'"
                );

                // Create: local_name = parent_namespace.imported_name
                result.push(statements::simple_assign(
                    local_name,
                    expressions::attribute(
                        expressions::name(&parent_namespace, ExprContext::Load),
                        imported_name,
                        ExprContext::Load,
                    ),
                ));

                add_module_attribute_if_needed(result);
                continue;
            }

            // For non-inlined parent or if parent not found, create a simple assignment
            // The symbol should already be available in the bundled code
            if local_name != imported_name {
                result.push(statements::simple_assign(
                    local_name,
                    expressions::name(imported_name, ExprContext::Load),
                ));
            }

            add_module_attribute_if_needed(result);
            continue;
        };

        log::debug!("Found module ID {module_id:?} for '{full_module_name}'");
        let is_bundled = bundler.bundled_modules.contains(&module_id);
        let is_inlined = bundler.inlined_modules.contains(&module_id);

        if is_bundled || is_inlined {
            // This is a bundled or inlined module, create assignment to reference it
            let module_var = crate::code_generator::module_registry::get_module_var_identifier(
                module_id,
                bundler.resolver,
            );

            // For inlined modules, we need to create a namespace object if it doesn't exist
            if is_inlined && !bundler.created_namespaces.contains(&module_var) {
                log::debug!("Creating namespace for inlined module '{full_module_name}'");

                // Create a SimpleNamespace for the inlined module
                let namespace_stmt = statements::simple_assign(
                    &module_var,
                    expressions::call(
                        expressions::attribute(
                            expressions::name("_cribo", ExprContext::Load),
                            "types.SimpleNamespace",
                            ExprContext::Load,
                        ),
                        vec![],
                        vec![expressions::keyword(
                            Some("__name__"),
                            expressions::string_literal(&full_module_name),
                        )],
                    ),
                );
                result.push(namespace_stmt);

                // Note: We can't modify bundler.created_namespaces here as it's borrowed
                // immutably The namespace will be tracked elsewhere
            }

            log::debug!("Creating assignment: {local_name} = {module_var}");

            result.push(statements::simple_assign(
                local_name,
                expressions::name(&module_var, ExprContext::Load),
            ));

            // Add as module attribute
            if add_module_attr {
                let current_module_var = sanitize_module_name_for_identifier(current_module);
                result.push(
                    crate::code_generator::module_registry::create_module_attr_assignment(
                        &current_module_var,
                        local_name,
                    ),
                );
            }
            continue;
        }

        // If not a bundled module, still create an assignment assuming the symbol exists
        // Only create assignment if names differ to avoid redundant "x = x"
        if local_name != imported_name {
            log::debug!("Creating fallback assignment: {local_name} = {imported_name}");
            result.push(statements::simple_assign(
                local_name,
                expressions::name(imported_name, ExprContext::Load),
            ));
        }

        // Add as module attribute if exportable and not private
        if add_module_attr && !local_name.starts_with('_') {
            let current_module_var = sanitize_module_name_for_identifier(current_module);
            result.push(
                crate::code_generator::module_registry::create_module_attr_assignment(
                    &current_module_var,
                    local_name,
                ),
            );
        }
    }
}
