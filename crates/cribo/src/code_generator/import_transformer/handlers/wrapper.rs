use ruff_python_ast::{Stmt, StmtImportFrom};

use crate::{
    ast_builder::{module_wrapper, statements},
    code_generator::{bundler::Bundler, module_registry},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Handle wrapper module import transformations
pub struct WrapperHandler;

impl WrapperHandler {
    /// Log information about wrapper wildcard exports (keeps previous behavior without generating
    /// code)
    pub(in crate::code_generator::import_transformer) fn log_wrapper_wildcard_info(
        resolved: &str,
        bundler: &Bundler,
    ) {
        log::debug!("  Handling wildcard import from wrapper module '{resolved}'");
        if let Some(exports) = bundler
            .get_module_id(resolved)
            .and_then(|id| bundler.module_exports.get(&id))
        {
            if let Some(export_list) = exports {
                log::debug!("  Wrapper module '{resolved}' exports: {export_list:?}");
                for export in export_list {
                    if export == "*" {
                        continue;
                    }
                }
            } else {
                log::debug!(
                    "  Wrapper module '{resolved}' has no explicit exports; importing all public \
                     symbols"
                );
                log::warn!(
                    "  Warning: Wildcard import from wrapper module without explicit __all__ may \
                     not import all symbols correctly"
                );
            }
        } else {
            log::warn!("  Warning: Could not find exports for wrapper module '{resolved}'");
        }
    }

    /// Check if a module access should be skipped because it points to a wrapper module
    pub(in crate::code_generator::import_transformer) fn is_wrapper_module_access(
        potential_submodule: &str,
        bundler: &Bundler,
    ) -> bool {
        // If this points to a wrapper module, don't transform
        bundler
            .get_module_id(potential_submodule)
            .is_some_and(|id| !bundler.inlined_modules.contains(&id))
            && bundler
                .get_module_id(potential_submodule)
                .is_some_and(|id| bundler.inlined_modules.contains(&id))
    }

    /// Check if a module is a wrapper module (has init function but is not inlined)
    pub(in crate::code_generator::import_transformer) fn is_wrapper_module(
        module_name: &str,
        bundler: &Bundler,
    ) -> bool {
        bundler
            .get_module_id(module_name)
            .is_some_and(|id| bundler.module_init_functions.contains_key(&id))
    }

    /// Create wrapper module initialization call statements
    pub(in crate::code_generator::import_transformer) fn create_wrapper_init_statements(
        resolved: &str,
        wrapper_module_id: ModuleId,
        bundler: &Bundler,
        at_module_level: bool,
        local_variables: &FxIndexSet<String>,
        entry_is_package_init_or_main: bool,
        _current_module_id: ModuleId,
    ) -> Vec<Stmt> {
        let mut init_stmts = Vec::new();

        // Do not emit init calls for the entry package (__init__ or __main__)
        let is_entry_pkg = if entry_is_package_init_or_main {
            let entry_pkg = [
                crate::python::constants::INIT_STEM,
                crate::python::constants::MAIN_STEM,
            ]
            .iter()
            .find_map(|stem| bundler.entry_module_name.strip_suffix(&format!(".{stem}")));
            entry_pkg.is_some_and(|pkg| pkg == resolved)
        } else {
            false
        };

        if is_entry_pkg {
            log::debug!(
                "  Skipping initialization call for entry package '{resolved}' to avoid circular \
                 init"
            );
        } else {
            log::debug!(
                "  Generating initialization call for wrapper module '{resolved}' at import \
                 location"
            );

            let module_var =
                module_registry::get_module_var_identifier(wrapper_module_id, bundler.resolver);

            // Create the init call
            // Only generate init calls at module level or if no conflict with local variables
            // However, skip if it conflicts with a local variable (like function parameters)
            if at_module_level {
                init_stmts.push(module_wrapper::create_wrapper_module_init_call(&module_var));
            } else if !local_variables.contains(&module_var) {
                // We need to declare the variable as global first, then call init
                log::debug!(
                    "  Adding global declaration for module variable '{module_var}' in function \
                     scope"
                );
                init_stmts.push(statements::global(vec![module_var.as_str()]));
                init_stmts.push(module_wrapper::create_wrapper_module_init_call(&module_var));
            } else {
                log::debug!(
                    "  Skipping initialization call for wrapper module '{resolved}' due to local \
                     variable conflict with '{module_var}'"
                );
            }
        }

        init_stmts
    }

    /// Track wrapper module imports for later rewriting
    pub(in crate::code_generator::import_transformer) fn track_wrapper_imports(
        import_from: &StmtImportFrom,
        module_name_for_tracking: &str,
        wrapper_module_imports: &mut FxIndexMap<String, (String, String)>,
    ) {
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();

            // Store mapping: local_name -> (wrapper_module, imported_name)
            wrapper_module_imports.insert(
                local_name.to_string(),
                (
                    module_name_for_tracking.to_string(),
                    imported_name.to_string(),
                ),
            );

            log::debug!(
                "  Tracking wrapper import: {local_name} -> \
                 {module_name_for_tracking}.{imported_name}"
            );
        }
    }
}
