use ruff_python_ast::{ExprContext, Stmt, StmtImportFrom};

use crate::ast_builder::{expressions, statements};

/// Handle submodule import transformations
pub(crate) struct SubmoduleHandler;

impl SubmoduleHandler {
    /// Handle from-import submodules
    pub(in crate::code_generator::import_transformer) fn handle_from_import_submodules(
        transformer: &mut crate::code_generator::import_transformer::RecursiveImportTransformer<'_>,
        import_from: &StmtImportFrom,
        resolved_base: &str,
    ) -> Option<Vec<Stmt>> {
        use crate::code_generator::module_registry::get_module_var_identifier;

        // Check if we're importing submodules that have been inlined
        // e.g., from utils import calculator where calculator is utils.calculator
        // This must be checked BEFORE checking if the parent module is inlined
        let mut result_stmts = Vec::new();
        let mut handled_any = false;

        log::debug!(
            "RecursiveImportTransformer: Checking import from '{}' in module '{}'",
            resolved_base,
            transformer
                .state
                .bundler
                .resolver
                .get_module_name(transformer.state.module_id)
                .unwrap_or_else(|| format!("module#{}", transformer.state.module_id))
        );

        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
            let full_module_path = format!("{resolved_base}.{imported_name}");

            log::debug!("  Checking if '{full_module_path}' is an inlined module");
            log::debug!(
                "  inlined_modules contains '{}': {}",
                full_module_path,
                transformer
                    .state
                    .bundler
                    .get_module_id(&full_module_path)
                    .is_some_and(|id| transformer.state.bundler.inlined_modules.contains(&id))
            );

            // Check if this is importing a submodule (like from . import config)
            // First check if it's a wrapper submodule, then check if it's inlined
            let is_wrapper_submodule = if let Some(submodule_id) =
                transformer.state.bundler.get_module_id(&full_module_path)
            {
                crate::code_generator::module_registry::is_wrapper_submodule(
                    submodule_id,
                    transformer.state.bundler.module_info_registry,
                    &transformer.state.bundler.inlined_modules,
                )
            } else {
                false
            };

            if is_wrapper_submodule {
                // This is a wrapper submodule
                log::debug!("  '{full_module_path}' is a wrapper submodule");

                // For wrapper modules importing wrapper submodules from the same package
                if transformer.state.is_wrapper_init {
                    // Initialize the wrapper submodule if needed
                    // Pass the current module context to avoid recursive initialization
                    if let Some(module_id) =
                        transformer.state.bundler.get_module_id(&full_module_path)
                    {
                        let current_module_id = transformer.state.bundler.get_module_id(
                            &transformer
                                .state
                                .bundler
                                .resolver
                                .get_module_name(transformer.state.module_id)
                                .unwrap_or_else(|| {
                                    format!("module#{}", transformer.state.module_id)
                                }),
                        );
                        result_stmts.extend(
                            transformer
                                .state
                                .bundler
                                .create_module_initialization_for_import_with_current_module(
                                    module_id,
                                    current_module_id,
                                    /* at_module_level */ true,
                                ),
                        );
                    }

                    // Create assignment: local_name = parent.submodule
                    let module_expr =
                        expressions::module_reference(&full_module_path, ExprContext::Load);

                    result_stmts.push(statements::simple_assign(local_name, module_expr));

                    // Track as local to avoid any accidental rewrites later in this transform
                    // pass
                    transformer
                        .state
                        .local_variables
                        .insert(local_name.to_owned());

                    log::debug!(
                        "  Created assignment for wrapper submodule: {local_name} = \
                         {full_module_path}"
                    );

                    // Note: The module attribute assignment (_cribo_module.<local_name> = ...)
                    // is handled later in create_assignments_for_inlined_imports to avoid
                    // duplication

                    handled_any = true;
                } else if !transformer.state.module_id.is_entry()
                    && transformer
                        .state
                        .bundler
                        .inlined_modules
                        .contains(&transformer.state.module_id)
                {
                    // This is an inlined module importing a wrapper submodule
                    // We need to defer this import because the wrapper module may not be
                    // initialized yet
                    log::debug!(
                        "  Inlined module '{}' importing wrapper submodule '{}' - deferring",
                        transformer
                            .state
                            .bundler
                            .resolver
                            .get_module_name(transformer.state.module_id)
                            .unwrap_or_else(|| format!("module#{}", transformer.state.module_id)),
                        full_module_path
                    );

                    // Note: deferred imports functionality has been removed
                    // The wrapper module assignment was previously deferred but is no longer
                    // needed

                    // Track as local to avoid any accidental rewrites later in this transform
                    // pass
                    transformer
                        .state
                        .local_variables
                        .insert(local_name.to_owned());

                    // Do not consume the import; let fallback keep it.
                    continue;
                }
            } else if let Some(module_id) =
                transformer.state.bundler.get_module_id(&full_module_path)
            {
                if transformer
                    .state
                    .bundler
                    .inlined_modules
                    .contains(&module_id)
                {
                    log::debug!("  '{full_module_path}' is an inlined module");

                    // Check if this module was namespace imported
                    if transformer
                        .state
                        .bundler
                        .namespace_imported_modules
                        .contains_key(&module_id)
                    {
                        // Create assignment: local_name = full_module_path_with_underscores
                        // But be careful about stdlib conflicts - only create in entry module
                        // if there's a conflict
                        // Use get_module_var_identifier to handle symlinks properly
                        use crate::code_generator::module_registry::get_module_var_identifier;
                        let namespace_var = get_module_var_identifier(
                            module_id,
                            transformer.state.bundler.resolver,
                        );

                        // Check if this would shadow a stdlib module
                        let shadows_stdlib = crate::resolver::is_stdlib_module(
                            local_name,
                            transformer.state.python_version,
                        );

                        // Only create the assignment if:
                        // 1. We're in the entry module (where user expects the shadowing), OR
                        // 2. The name doesn't conflict with stdlib
                        if transformer.state.module_id.is_entry() || !shadows_stdlib {
                            log::debug!(
                                "  Creating namespace assignment: {local_name} = {namespace_var}"
                            );
                            result_stmts.push(statements::simple_assign(
                                local_name,
                                expressions::name(&namespace_var, ExprContext::Load),
                            ));

                            // Track this as a local variable to prevent it from being
                            // transformed as a stdlib module
                            transformer
                                .state
                                .local_variables
                                .insert(local_name.to_owned());
                            log::debug!(
                                "  Tracked '{local_name}' as local variable to prevent stdlib \
                                 transformation"
                            );
                        } else {
                            log::debug!(
                                "  Skipping namespace assignment: {local_name} = {namespace_var} \
                                 - would shadow stdlib in non-entry module"
                            );
                        }
                        handled_any = true;
                    }
                } else {
                    // This is importing an inlined submodule
                    // We need to handle this specially when the current module is being inlined
                    // (i.e., not the entry module and not a wrapper module)
                    let current_module_is_inlined = transformer
                        .state
                        .bundler
                        .inlined_modules
                        .contains(&transformer.state.module_id);
                    let current_module_is_wrapper =
                        !current_module_is_inlined && !transformer.state.module_id.is_entry();

                    if !transformer.state.module_id.is_entry()
                        && (current_module_is_inlined || current_module_is_wrapper)
                    {
                        log::debug!(
                            "  Creating namespace for inlined submodule: {local_name} -> \
                             {full_module_path}"
                        );

                        if current_module_is_inlined {
                            // For inlined modules importing other inlined modules, we need to
                            // defer the namespace creation
                            // until after all modules are inlined
                            log::debug!("  Deferring namespace creation for inlined module import");

                            // Create the namespace and populate it as deferred imports
                            // For inlined modules, use the sanitized module name instead of
                            // local_name e.g., pkg_compat
                            // instead of compat
                            // Use get_module_var_identifier to handle symlinks properly
                            let namespace_var = get_module_var_identifier(
                                module_id,
                                transformer.state.bundler.resolver,
                            );

                            // Deferred namespace creation removed; skip no-op branch

                            // IMPORTANT: Create the local alias immediately, not deferred
                            // This ensures the alias is available in the current module's
                            // context For example, when `from .
                            // import messages` in greetings.greeting,
                            // we need `messages = greetings_messages` to be available
                            // immediately
                            crate::code_generator::import_transformer::handlers::inlined::InlinedHandler::alias_local_to_namespace_if_needed(
                                local_name,
                                &namespace_var,
                                &mut result_stmts,
                            );
                            transformer.state.created_namespace_objects = true;

                            // If this is a submodule being imported (from . import compat),
                            // and the parent module is also being used as a namespace
                            // externally, we need to create the
                            // parent.child assignment
                            transformer.maybe_log_parent_child_assignment(
                                Some(resolved_base),
                                imported_name,
                                local_name,
                            );

                            // Mark namespace populated if needed (keep deferred behavior)
                            transformer.mark_namespace_populated_if_needed(&full_module_path);
                        } else {
                            // For wrapper modules importing inlined modules, we need to create
                            // the namespace immediately since it's used in the module body
                            log::debug!("  Creating immediate namespace for wrapper module import");

                            // Create: local_name = types.SimpleNamespace()
                            result_stmts.push(statements::simple_assign(
                                local_name,
                                expressions::call(
                                    expressions::simple_namespace_ctor(),
                                    vec![],
                                    vec![],
                                ),
                            ));
                            transformer.state.created_namespace_objects = true;

                            transformer.emit_namespace_symbols_for_local_from_path(
                                local_name,
                                &full_module_path,
                                &mut result_stmts,
                            );
                        }

                        handled_any = true;
                    } else if !transformer.state.module_id.is_entry() {
                        // This is a wrapper module importing an inlined module
                        log::debug!(
                            "  Deferring inlined submodule import in wrapper module: {local_name} \
                             -> {full_module_path}"
                        );
                    } else {
                        // For entry module, create namespace object immediately

                        // Create the namespace object with symbols
                        // This mimics what happens in non-entry modules

                        // First create the empty namespace
                        result_stmts.push(statements::simple_assign(
                            local_name,
                            expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
                        ));

                        // Track this as a local variable, not an import alias
                        transformer
                            .state
                            .local_variables
                            .insert(local_name.to_owned());

                        handled_any = true;
                    }
                }
            }
        }

        if handled_any {
            // For deferred imports, we return empty to remove the original import
            if result_stmts.is_empty() {
                log::debug!("  Import handling deferred, returning empty");
                return Some(vec![]);
            }
            log::debug!(
                "  Returning {} transformed statements for import",
                result_stmts.len()
            );
            log::debug!("  Statements: {result_stmts:?}");
            // We've already handled the import completely, don't fall through to other handling
            return Some(result_stmts);
        }

        None
    }
}
