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

    /// Check if a module path is a wrapper submodule and handle wrapper-to-wrapper imports
    pub(in crate::code_generator::import_transformer) fn handle_wrapper_submodule_import(
        full_module_path: &str,
        local_name: &str,
        bundler: &Bundler,
        is_wrapper_init: bool,
        get_module_name: &str,
        local_variables: &mut FxIndexSet<String>,
    ) -> Option<Vec<Stmt>> {
        let is_wrapper_submodule =
            if let Some(submodule_id) = bundler.get_module_id(full_module_path) {
                crate::code_generator::module_registry::is_wrapper_submodule(
                    submodule_id,
                    bundler.module_info_registry,
                    &bundler.inlined_modules,
                )
            } else {
                false
            };

        if is_wrapper_submodule {
            log::debug!("  '{full_module_path}' is a wrapper submodule");

            if is_wrapper_init {
                let mut result_stmts = Vec::new();

                // Initialize the wrapper submodule if needed
                if let Some(module_id) = bundler.get_module_id(full_module_path) {
                    let current_module_id = bundler.get_module_id(get_module_name);
                    result_stmts.extend(
                        bundler.create_module_initialization_for_import_with_current_module(
                            module_id,
                            current_module_id,
                            /* at_module_level */ true,
                        ),
                    );
                }

                // Create assignment: local_name = parent.submodule
                use ruff_python_ast::ExprContext;

                use crate::ast_builder::{expressions, statements};

                let module_expr =
                    expressions::module_reference(full_module_path, ExprContext::Load);
                result_stmts.push(statements::simple_assign(local_name, module_expr));

                // Track as local to avoid any accidental rewrites later
                local_variables.insert(local_name.to_string());

                log::debug!(
                    "  Created assignment for wrapper submodule: {local_name} = {full_module_path}"
                );

                return Some(result_stmts);
            }
            // This is an inlined module importing a wrapper submodule
            log::debug!(
                "  Inlined module '{get_module_name}' importing wrapper submodule '{full_module_path}' - deferring"
            );
        }

        None
    }

    /// Try to rewrite an attribute access where the base is a wrapper module import
    /// Returns `Some(new_expr)` if the rewrite was performed, None otherwise
    pub(in crate::code_generator::import_transformer) fn try_rewrite_wrapper_attribute(
        name: &str,
        attr_expr: &ruff_python_ast::ExprAttribute,
        wrapper_module_imports: &FxIndexMap<String, (String, String)>,
    ) -> Option<ruff_python_ast::Expr> {
        if let Some((wrapper_module, imported_name)) = wrapper_module_imports.get(name) {
            // The base is a wrapper module import, rewrite the entire attribute access
            // e.g., cookielib.CookieJar -> myrequests.compat.cookielib.CookieJar
            log::debug!(
                "Rewriting attribute '{}.{}' to '{}.{}.{}'",
                name,
                attr_expr.attr.as_str(),
                wrapper_module,
                imported_name,
                attr_expr.attr.as_str()
            );

            use ruff_python_ast::{Expr, ExprContext};

            use crate::ast_builder::expressions;

            // Create wrapper_module.imported_name.attr
            let base =
                expressions::name_attribute(wrapper_module, imported_name, ExprContext::Load);
            let mut new_expr = expressions::attribute(base, attr_expr.attr.as_str(), attr_expr.ctx);
            // Preserve the original range
            if let Expr::Attribute(attr) = &mut new_expr {
                attr.range = attr_expr.range;
            }
            return Some(new_expr);
        }
        None
    }

    /// Try to rewrite a name expression that was imported from a wrapper module
    /// Returns `Some(new_expr)` if the rewrite was performed, None otherwise  
    pub(in crate::code_generator::import_transformer) fn try_rewrite_wrapper_name(
        name: &str,
        name_expr: &ruff_python_ast::ExprName,
        wrapper_module_imports: &FxIndexMap<String, (String, String)>,
    ) -> Option<ruff_python_ast::Expr> {
        if let Some((wrapper_module, imported_name)) = wrapper_module_imports.get(name) {
            log::debug!("Rewriting name '{name}' to '{wrapper_module}.{imported_name}'");

            use ruff_python_ast::Expr;

            use crate::ast_builder::expressions;

            // Create wrapper_module.imported_name attribute access
            let mut new_expr =
                expressions::name_attribute(wrapper_module, imported_name, name_expr.ctx);
            // Preserve the original range
            if let Expr::Attribute(attr) = &mut new_expr {
                attr.range = name_expr.range;
            }
            return Some(new_expr);
        }
        None
    }
}
