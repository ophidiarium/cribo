use ruff_python_ast::{AtomicNodeIndex, Expr, ExprAttribute, ExprCall, ExprContext, ExprName};

use crate::{
    code_generator::bundler::Bundler,
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Handle dynamic import transformations (`importlib.import_module`)
pub(crate) struct DynamicHandler;

impl DynamicHandler {
    /// Check if this is an `importlib.import_module()` call
    pub(in crate::code_generator::import_transformer) fn is_importlib_import_module_call(
        call: &ExprCall,
        import_aliases: &FxIndexMap<String, String>,
    ) -> bool {
        match &call.func.as_ref() {
            // Direct call: importlib.import_module()
            Expr::Attribute(attr) if attr.attr.as_str() == "import_module" => {
                match &attr.value.as_ref() {
                    Expr::Name(name) => {
                        let name_str = name.id.as_str();
                        // Check if it's 'importlib' directly or an alias that maps to 'importlib'
                        name_str == "importlib"
                            || import_aliases.get(name_str) == Some(&"importlib".to_owned())
                    }
                    _ => false,
                }
            }
            // Function call: im() where im is import_module
            Expr::Name(name) => {
                // Check if this name is an alias for importlib.import_module
                import_aliases
                    .get(name.id.as_str())
                    .is_some_and(|module| module == "importlib.import_module")
            }
            _ => false,
        }
    }

    /// Resolve `importlib.import_module()` target module name, handling relative imports
    fn resolve_importlib_target(call: &ExprCall, bundler: &Bundler<'_>) -> Option<String> {
        if let Some(arg) = call.arguments.args.first()
            && let Expr::StringLiteral(lit) = arg
        {
            let module_name = lit.value.to_str();

            // Handle relative imports with package context
            let resolved_name = if module_name.starts_with('.') && call.arguments.args.len() >= 2 {
                // Get the package context from the second argument
                if let Expr::StringLiteral(package_lit) = &call.arguments.args[1] {
                    let package = package_lit.value.to_str();

                    // Resolve package to path, then use resolver
                    if let Ok(Some(package_path)) = bundler.resolver.resolve_module_path(package) {
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        bundler
                            .resolver
                            .resolve_relative_to_absolute_module_name(
                                level,
                                if name_part.is_empty() {
                                    None
                                } else {
                                    Some(name_part)
                                },
                                &package_path,
                            )
                            .unwrap_or_else(|| module_name.to_owned())
                    } else {
                        // Use resolver's method for package name resolution when path not found
                        let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                        let name_part = module_name.trim_start_matches('.');

                        bundler.resolver.resolve_relative_import_from_package_name(
                            level,
                            if name_part.is_empty() {
                                None
                            } else {
                                Some(name_part)
                            },
                            package,
                        )
                    }
                } else {
                    module_name.to_owned()
                }
            } else {
                module_name.to_owned()
            };

            Some(resolved_name)
        } else {
            None
        }
    }

    /// Transform importlib.import_module("module-name") to direct module reference
    pub(in crate::code_generator::import_transformer) fn transform_importlib_import_module(
        call: &ExprCall,
        bundler: &Bundler<'_>,
        created_namespace_objects: &mut bool,
        create_module_access_expr: impl Fn(&str) -> Expr,
    ) -> Option<Expr> {
        // Get the module name and resolve relative imports
        if let Some(resolved_name) = Self::resolve_importlib_target(call, bundler) {
            // Check if this module is part of the bundle (wrapper or inlined)
            if bundler.get_module_id(&resolved_name).is_some_and(|id| {
                bundler.bundled_modules.contains(&id) || bundler.inlined_modules.contains(&id)
            }) {
                log::debug!(
                    "Transforming importlib.import_module call to module access '{resolved_name}'"
                );

                // Check if this creates a namespace object
                if bundler
                    .get_module_id(&resolved_name)
                    .is_some_and(|id| bundler.inlined_modules.contains(&id))
                {
                    *created_namespace_objects = true;
                }

                // Use common logic for module access
                return Some(create_module_access_expr(&resolved_name));
            }
        }
        None
    }

    /// For importlib-imported module variables, rewrite `base.attr` to the inlined symbol
    pub(in crate::code_generator::import_transformer) fn rewrite_attr_for_importlib_var(
        attr_expr: &ExprAttribute,
        base: &str,
        module_name: &str,
        bundler: &Bundler<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    ) -> Expr {
        // Only rewrite attribute reads; preserve writes to module attributes.
        if !matches!(attr_expr.ctx, ExprContext::Load) {
            return Expr::Attribute(attr_expr.clone());
        }
        let attr_name = attr_expr.attr.as_str();

        if let Some(module_id) = bundler.get_module_id(module_name)
            && let Some(module_renames) = symbol_renames.get(&module_id)
            && let Some(renamed) = module_renames.get(attr_name)
        {
            let renamed_str = renamed.clone();
            log::debug!(
                "Rewrote {base}.{attr_name} to {renamed_str} (renamed symbol from importlib \
                 inlined module)"
            );
            return Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                id: renamed_str.into(),
                ctx: attr_expr.ctx,
                range: attr_expr.range,
            });
        }
        // no rename: fallthrough below
        log::debug!(
            "Rewrote {base}.{attr_name} to {attr_name} (symbol from importlib inlined module)"
        );
        Expr::Name(ExprName {
            node_index: AtomicNodeIndex::NONE,
            id: attr_name.into(),
            ctx: attr_expr.ctx,
            range: attr_expr.range,
        })
    }

    /// Handle assignment from `importlib.import_module` call, tracking inlined modules
    pub(in crate::code_generator::import_transformer) fn handle_importlib_assignment(
        assigned_names: &FxIndexSet<String>,
        call: &ExprCall,
        bundler: &Bundler<'_>,
        importlib_inlined_modules: &mut FxIndexMap<String, String>,
    ) {
        // Get the module name and resolve relative imports
        if let Some(resolved_name) = Self::resolve_importlib_target(call, bundler)
            && bundler
                .get_module_id(&resolved_name)
                .is_some_and(|id| bundler.inlined_modules.contains(&id))
        {
            // Track all assigned names as importing this module
            for name in assigned_names {
                log::debug!(
                    "Tracking variable '{name}' as assigned from \
                     importlib.import_module('{resolved_name}')"
                );
                importlib_inlined_modules.insert(name.clone(), resolved_name.clone());
            }
        }
    }
}
