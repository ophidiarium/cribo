use ruff_python_ast::{ExprContext, Stmt, StmtImportFrom};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::{bundler::Bundler, module_registry, module_transformer::SELF_PARAM},
    types::{FxIndexMap, FxIndexSet},
};

/// Handle stdlib import transformations
pub(crate) struct StdlibHandler;

impl StdlibHandler {
    /// Check if this is a stdlib import that should be normalized
    pub(in crate::code_generator::import_transformer) fn should_normalize_stdlib_import(
        module_name: &str,
        python_version: u8,
    ) -> bool {
        // Recognize full stdlib module paths and submodules for the current Python version
        crate::resolver::is_stdlib_module(module_name, python_version)
    }

    /// Build a mapping of stdlib imports to their rewritten paths
    /// This mapping is used during expression rewriting
    pub(in crate::code_generator::import_transformer) fn build_stdlib_rename_map(
        imports: &[(String, Option<String>)],
    ) -> FxIndexMap<String, String> {
        let mut rename_map = FxIndexMap::default();

        for (module_name, alias) in imports {
            let local_name = alias.as_ref().unwrap_or(module_name);
            let rewritten_path = Bundler::get_rewritten_stdlib_path(module_name);
            rename_map.insert(local_name.clone(), rewritten_path);
        }

        rename_map
    }

    /// Handle stdlib from imports, transforming them to use _cribo proxy
    pub(in crate::code_generator::import_transformer) fn handle_stdlib_from_import(
        import_from: &StmtImportFrom,
        module_str: &str,
        python_version: u8,
        imported_stdlib_modules: &mut FxIndexSet<String>,
        import_aliases: &mut FxIndexMap<String, String>,
    ) -> Option<Vec<Stmt>> {
        if import_from.level != 0
            || !Self::should_normalize_stdlib_import(module_str, python_version)
        {
            return None;
        }

        // Track that this stdlib module was imported
        imported_stdlib_modules.insert(module_str.to_owned());
        // Also track parent modules for dotted imports
        if let Some(dot_pos) = module_str.find('.') {
            let parent = &module_str[..dot_pos];
            imported_stdlib_modules.insert(parent.to_owned());
        }

        let mut assignments = Vec::new();
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            if imported_name == "*" {
                // Preserve wildcard imports from stdlib to avoid incorrect symbol drops
                return Some(vec![Stmt::ImportFrom(import_from.clone())]);
            }

            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_path = format!(
                "{}.{module_str}.{imported_name}",
                crate::ast_builder::CRIBO_PREFIX
            );

            // Track this renaming for expression rewriting
            if module_str == "importlib" && imported_name == "import_module" {
                import_aliases.insert(
                    local_name.to_owned(),
                    format!("{module_str}.{imported_name}"),
                );
            } else {
                import_aliases.insert(local_name.to_owned(), full_path.clone());
            }

            // Create local assignment: local_name = _cribo.module.symbol
            let proxy_parts: Vec<&str> = full_path.split('.').collect();
            let value_expr = expressions::dotted_name(&proxy_parts, ExprContext::Load);
            let target = expressions::name(local_name, ExprContext::Store);
            let assign_stmt = statements::assign(vec![target], value_expr);
            assignments.push(assign_stmt);
        }

        Some(assignments)
    }

    /// Handle stdlib imports in wrapper modules
    pub(in crate::code_generator::import_transformer) fn handle_wrapper_stdlib_imports(
        stdlib_imports: &[(String, Option<String>)],
        is_wrapper_init: bool,
        module_id: crate::resolver::ModuleId,
        current_module_name: &str,
        bundler: &Bundler<'_>,
    ) -> Vec<Stmt> {
        let mut assignments = Vec::new();

        for (module_name, alias) in stdlib_imports {
            // Determine the local name that the import creates
            let local_name = if let Some(alias_name) = alias {
                // Aliased import: "import json as j" creates local "j"
                alias_name.clone()
            } else if module_name.contains('.') {
                // Dotted import without alias doesn't create a binding
                continue;
            } else {
                // Simple import: "import json" creates local "json"
                module_name.clone()
            };

            // 1) Create local alias: local = _cribo.<stdlib_module>
            let proxy_path = format!("{}.{module_name}", crate::ast_builder::CRIBO_PREFIX);
            let proxy_parts: Vec<&str> = proxy_path.split('.').collect();
            let value_expr = expressions::dotted_name(&proxy_parts, ExprContext::Load);
            let target = expressions::name(local_name.as_str(), ExprContext::Store);
            assignments.push(statements::assign(vec![target], value_expr));

            // 2) Set module attribute: <current_module>.<local> = <local>
            // In wrapper init functions, use "self" instead of the module name
            if is_wrapper_init {
                assignments.push(module_registry::create_module_attr_assignment(
                    SELF_PARAM,
                    local_name.as_str(),
                ));
            } else {
                let module_var =
                    module_registry::sanitize_module_name_for_identifier(current_module_name);
                assignments.push(module_registry::create_module_attr_assignment(
                    &module_var,
                    local_name.as_str(),
                ));
            }

            // 3) Optionally expose on self if part of exports (__all__) for this module
            // Skip this for wrapper init since we already added it above
            if !is_wrapper_init
                && let Some(Some(exports)) = bundler.module_exports.get(&module_id)
                && exports.contains(&local_name)
            {
                assignments.push(statements::assign_attribute(
                    SELF_PARAM,
                    local_name.as_str(),
                    expressions::name(local_name.as_str(), ExprContext::Load),
                ));
            }
        }

        assignments
    }
}
