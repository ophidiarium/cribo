//! Module transformation logic for converting Python modules into init functions
//!
//! This module handles the complex transformation of Python module ASTs into
//! initialization functions that can be called to create module objects.

// Constant for the self parameter name used in init functions
const SELF_PARAM: &str = "self";

use log::debug;
use ruff_python_ast::{
    AtomicNodeIndex, ExceptHandler, Expr, ExprContext, Identifier, ModModule, Stmt, StmtAssign,
    StmtFunctionDef, StmtGlobal,
};
use ruff_text_size::TextRange;

use crate::{
    ast_builder,
    code_generator::{
        bundler::Bundler,
        context::ModuleTransformContext,
        expression_handlers,
        globals::{GlobalsLifter, transform_globals_in_stmt, transform_locals_in_stmt},
        import_deduplicator,
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
        module_registry::sanitize_module_name_for_identifier,
    },
    types::{FxIndexMap, FxIndexSet},
};

/// Transforms a module AST into an initialization function
pub fn transform_module_to_init_function<'a>(
    bundler: &'a Bundler<'a>,
    ctx: &ModuleTransformContext,
    mut ast: ModModule,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
) -> Stmt {
    let module_id = bundler
        .get_module_id(ctx.module_name)
        .expect("Module ID must exist for module being transformed");
    let init_func_name = bundler
        .module_init_functions
        .get(&module_id)
        .expect("Init function must exist for wrapper module");
    let mut body = Vec::new();

    // Get the global module variable name
    let module_var_name = sanitize_module_name_for_identifier(ctx.module_name);

    // Check if already fully initialized
    // if getattr(self, "__initialized__", False):
    //     return self
    let check_initialized = ast_builder::statements::if_stmt(
        ast_builder::expressions::call(
            ast_builder::expressions::name("getattr", ExprContext::Load),
            vec![
                ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
                ast_builder::expressions::string_literal("__initialized__"),
                ast_builder::expressions::bool_literal(false),
            ],
            vec![],
        ),
        vec![ast_builder::statements::return_stmt(Some(
            ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
        ))],
        vec![],
    );
    body.push(check_initialized);

    // Check if currently initializing (circular dependency)
    // if getattr(self, "__initializing__", False):
    //     return self  # Return partial module in partially-initialized state
    let check_initializing = ast_builder::statements::if_stmt(
        ast_builder::expressions::call(
            ast_builder::expressions::name("getattr", ExprContext::Load),
            vec![
                ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
                ast_builder::expressions::string_literal("__initializing__"),
                ast_builder::expressions::bool_literal(false),
            ],
            vec![],
        ),
        vec![ast_builder::statements::return_stmt(Some(
            ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
        ))],
        vec![],
    );
    body.push(check_initializing);

    // Mark as initializing at the start of init to emulate Python's partial module semantics
    body.push(ast_builder::statements::assign_attribute(
        SELF_PARAM,
        "__initializing__",
        ast_builder::expressions::bool_literal(true),
    ));

    // NOTE: We do NOT call parent init from child modules
    // In Python, the import machinery ensures parent is initialized before child,
    // but this happens OUTSIDE the child module's code.
    // Child modules don't explicitly call parent init - that would create
    // artificial circular dependencies.
    // The parent will be initialized by whoever imports the child module.

    // Apply globals lifting if needed
    let lifted_names = if let Some(ref global_info) = ctx.global_info {
        if global_info.global_declarations.is_empty() {
            None
        } else {
            let globals_lifter = GlobalsLifter::new(global_info);
            let lifted_names = globals_lifter.get_lifted_names().clone();

            // Transform the AST to use lifted globals
            transform_ast_with_lifted_globals(bundler, &mut ast, &lifted_names, global_info);

            Some(lifted_names)
        }
    } else {
        None
    };

    // First, recursively transform all imports in the AST
    // For wrapper modules, we don't need to defer imports since they run in their own scope
    let mut transformer = RecursiveImportTransformer::new(&RecursiveImportTransformerParams {
        bundler,
        module_id,
        symbol_renames,
        is_wrapper_init: true,         // This IS a wrapper init function
        global_deferred_imports: None, // No need for global deferred imports in wrapper modules
        python_version: ctx.python_version,
    });

    // Track imports from inlined modules before transformation
    // - imports_from_inlined: symbols that exist in global scope (primarily for wildcard imports)
    //   Format: (exported_name, value_name, source_module)
    // - inlined_import_bindings: local binding names created by explicit from-imports (asname if
    //   present)
    let mut imports_from_inlined: Vec<(String, String, Option<String>)> = Vec::new();
    let mut inlined_import_bindings = Vec::new();
    // Track wrapper module symbols that need placeholders (symbol_name, value_name)
    let mut wrapper_module_symbols_global_only: Vec<(String, String)> = Vec::new();

    // Track ALL imported symbols to avoid overwriting them with submodule namespaces
    let mut imported_symbols = FxIndexSet::default();

    // Track stdlib symbols that need to be added to the module namespace
    // Use a stable set to dedup and preserve insertion order
    let mut stdlib_reexports: FxIndexSet<(String, String)> = FxIndexSet::default();

    // Do not reorder statements in wrapper modules. Some libraries (e.g., httpx)
    // define constants used by function default arguments; hoisting functions would
    // break evaluation order of those defaults.

    for stmt in &ast.body {
        if let Stmt::ImportFrom(import_from) = stmt {
            // Collect ALL imported symbols (not just from inlined modules)
            for alias in &import_from.names {
                let imported_name = alias.name.as_str();
                if imported_name != "*" {
                    // Use the local binding name (asname if present, otherwise the imported name)
                    let local_name = alias
                        .asname
                        .as_ref()
                        .map_or(imported_name, ruff_python_ast::Identifier::as_str);
                    imported_symbols.insert(local_name.to_string());
                    debug!(
                        "Collected imported symbol '{}' in module '{}'",
                        local_name, ctx.module_name
                    );
                }
                // Note: Wildcard imports aren't expanded here to avoid false positives.
                // Resolution is handled elsewhere (module export analysis); we intentionally skip
                // adding names from '*' here.
            }

            // Resolve the module to check if it's inlined
            let resolved_module = if import_from.level > 0 {
                bundler.resolver.resolve_relative_to_absolute_module_name(
                    import_from.level,
                    import_from
                        .module
                        .as_ref()
                        .map(ruff_python_ast::Identifier::as_str),
                    ctx.module_path,
                )
            } else {
                import_from
                    .module
                    .as_ref()
                    .map(std::string::ToString::to_string)
            };

            if let Some(ref module) = resolved_module {
                // Check if this is a stdlib module
                let root_module = module.split('.').next().unwrap_or(module);
                let is_stdlib = ruff_python_stdlib::sys::is_known_standard_library(
                    ctx.python_version,
                    root_module,
                );

                if is_stdlib && import_from.level == 0 {
                    // Track stdlib imports for re-export
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        if imported_name != "*" {
                            let local_name = alias
                                .asname
                                .as_ref()
                                .map_or(imported_name, ruff_python_ast::Identifier::as_str);

                            // Check if this symbol should be re-exported (in __all__ or no __all__)
                            let should_reexport = if let Some(Some(export_list)) = bundler
                                .get_module_id(ctx.module_name)
                                .and_then(|id| bundler.module_exports.get(&id))
                            {
                                export_list.contains(&local_name.to_string())
                            } else {
                                // No explicit __all__, re-export all public symbols
                                !local_name.starts_with('_')
                            };

                            if should_reexport {
                                let proxy_path = format!(
                                    "{}.{module}.{imported_name}",
                                    crate::ast_builder::CRIBO_PREFIX
                                );
                                debug!(
                                    "Tracking stdlib re-export in wrapper module '{}': {} -> {}",
                                    ctx.module_name, local_name, &proxy_path
                                );
                                stdlib_reexports.insert((local_name.to_string(), proxy_path));
                            }
                        }
                    }
                }

                // Check if the module is inlined (NOT wrapper modules)
                // Only inlined modules have their symbols in global scope
                let is_inlined = bundler
                    .get_module_id(module)
                    .is_some_and(|id| bundler.inlined_modules.contains(&id));

                debug!("Checking if resolved module '{module}' is inlined: {is_inlined}");

                if is_inlined {
                    // Track all imported names from this inlined module
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        // For wildcard imports, we need to track all symbols that will be imported
                        if imported_name == "*" {
                            let wrapper_symbols = process_wildcard_import(
                                bundler,
                                module,
                                symbol_renames,
                                &mut imports_from_inlined,
                                ctx.module_name,
                            );

                            // Collect wrapper module symbols that need special handling
                            // We need to track them separately to create placeholder assignments
                            for (symbol_name, value_name) in wrapper_symbols {
                                debug!(
                                    "Collecting wrapper module symbol '{symbol_name}' for special \
                                     handling"
                                );
                                wrapper_module_symbols_global_only.push((symbol_name, value_name));
                            }
                        } else {
                            let local_binding_name = alias
                                .asname
                                .as_ref()
                                .map_or(imported_name, ruff_python_ast::Identifier::as_str);
                            debug!(
                                "Tracking imported name '{imported_name}' as local binding \
                                 '{local_binding_name}' from inlined module '{module}'"
                            );
                            inlined_import_bindings.push(local_binding_name.to_string());
                        }
                    }
                }
            }
        }

        // Also handle plain import statements to avoid name collisions
        if let Stmt::Import(import_stmt) = stmt {
            for alias in &import_stmt.names {
                // Local binding is either `asname` or the top-level package segment (`pkg` in
                // `pkg.sub`)
                let local_name = alias
                    .asname
                    .as_ref()
                    .map(Identifier::as_str)
                    .unwrap_or_else(|| {
                        let full = alias.name.as_str();
                        full.split('.').next().unwrap_or(full)
                    });
                imported_symbols.insert(local_name.to_string());
                debug!(
                    "Collected imported symbol '{}' via 'import' in module '{}'",
                    local_name, ctx.module_name
                );
            }
        }
    }

    transformer.transform_module(&mut ast);

    // If namespace objects were created, we need types import
    // (though wrapper modules already have types import)
    if transformer.created_namespace_objects() {
        debug!("Namespace objects were created in wrapper module, types import already present");
    }

    // Add global declarations for symbols imported from inlined modules
    // This is necessary because the symbols are defined in the global scope
    // but we need to access them inside the init function
    // Also include wrapper module symbols that will be defined later
    if !imports_from_inlined.is_empty() || !wrapper_module_symbols_global_only.is_empty() {
        // Deduplicate by value name (what's actually in global scope) and sort for deterministic
        // output
        // Only add symbols from NON-inlined modules to globals (they exist as bare symbols)
        // Symbols from inlined modules will be accessed through their namespace
        let mut unique_imports: Vec<String> = imports_from_inlined
            .iter()
            .filter(|(_, _, source_module)| source_module.is_none())
            .map(|(_, value_name, _)| value_name.clone())
            .chain(
                wrapper_module_symbols_global_only
                    .iter()
                    .map(|(_, value_name)| value_name.clone()),
            )
            .collect::<FxIndexSet<_>>()
            .into_iter()
            .collect();
        unique_imports.sort();

        // Filter out tree-shaken symbols
        if let Some(ref tree_shaking_keep) = bundler.tree_shaking_keep_symbols {
            // Use the pre-computed global set of kept symbols for efficient lookup.
            if let Some(ref all_kept_symbols) = bundler.kept_symbols_global {
                unique_imports.retain(|symbol| {
                    if all_kept_symbols.contains(symbol) {
                        if log::log_enabled!(log::Level::Debug) {
                            // This find is only executed when debug logging is enabled.
                            let module_name = tree_shaking_keep
                                .iter()
                                .find(|(_, symbols)| symbols.contains(symbol))
                                .and_then(|(id, _)| bundler.resolver.get_module_name(*id))
                                .unwrap_or_else(|| "unknown".to_string());
                            debug!(
                                "Symbol '{symbol}' kept by tree-shaking from module \
                                 '{module_name}'"
                            );
                        }
                        true
                    } else {
                        debug!(
                            "Symbol '{symbol}' was removed by tree-shaking, excluding from global \
                             declaration"
                        );
                        false
                    }
                });
            }
        }

        if !unique_imports.is_empty() {
            debug!(
                "Adding global declaration for imported symbols from inlined modules: \
                 {unique_imports:?}"
            );
            body.push(Stmt::Global(StmtGlobal {
                node_index: AtomicNodeIndex::dummy(),
                names: unique_imports
                    .iter()
                    .map(|name| Identifier::new(name, TextRange::default()))
                    .collect(),
                range: TextRange::default(),
            }));
        }
    }

    // Note: deferred imports functionality has been removed
    // Import alias assignments were previously added here

    // Add placeholder assignments for wrapper module symbols
    // These symbols will be properly assigned later when wrapper modules are initialized,
    // but we need them to exist in the local scope (not as module attributes yet)
    // We use a sentinel object that can have attributes set on it
    for (symbol_name, _value_name) in &wrapper_module_symbols_global_only {
        debug!("Adding placeholder assignment for wrapper module symbol '{symbol_name}'");
        // Create assignment: symbol_name = types.SimpleNamespace()
        // This creates a placeholder that can have attributes set on it
        body.push(ast_builder::statements::simple_assign(
            symbol_name,
            ast_builder::expressions::call(
                ast_builder::expressions::simple_namespace_ctor(),
                vec![],
                vec![],
            ),
        ));
        // Also add as module attribute so it's visible in vars(__cribo_module)
        body.push(
            crate::code_generator::module_registry::create_module_attr_assignment(
                SELF_PARAM,
                symbol_name,
            ),
        );
    }

    // CRITICAL: Add wildcard-imported symbols as module attributes NOW
    // This must happen BEFORE processing the body, as the body may contain code that
    // accesses these symbols via vars(__cribo_module) or locals()
    // (e.g., the setattr pattern used by httpx and similar libraries)

    // Dedup and sort wildcard imports for deterministic output
    let mut wildcard_attrs: Vec<(String, String, Option<String>)> = imports_from_inlined
        .iter()
        .cloned()
        .collect::<FxIndexSet<_>>()
        .into_iter()
        .collect();
    wildcard_attrs.sort_by(|a, b| a.0.cmp(&b.0)); // Sort by exported name

    for (exported_name, value_name, source_module) in wildcard_attrs {
        if bundler.should_export_symbol(&exported_name, ctx.module_name) {
            // If the symbol comes from an inlined module, access it through the module's namespace
            let value_expr = if let Some(ref module) = source_module {
                // Access through the inlined module's namespace
                let sanitized =
                    crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                        module,
                    );
                format!("{sanitized}.{value_name}")
            } else {
                value_name.clone()
            };

            body.push(
                crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                    SELF_PARAM,
                    &exported_name,
                    &value_expr,
                ),
            );
        }
    }

    // Check if __all__ is referenced in the module body
    let mut all_is_referenced = false;
    for stmt in &ast.body {
        // Skip checking __all__ assignment itself
        if let Stmt::Assign(assign) = stmt
            && let Some(name) = expression_handlers::extract_simple_assign_target(assign)
            && name == "__all__"
        {
            continue;
        } else if let Stmt::AnnAssign(ann_assign) = stmt
            && let Expr::Name(target) = ann_assign.target.as_ref()
            && target.id.as_str() == "__all__"
        {
            // Skip annotated assignments to __all__ as a "reference"
            continue;
        }
        // Check if __all__ is referenced in this statement
        if crate::visitors::VariableCollector::statement_references_variable(stmt, "__all__") {
            all_is_referenced = true;
            break;
        }
    }

    // Collect all variables that are referenced by exported functions
    let mut vars_used_by_exported_functions: FxIndexSet<String> = FxIndexSet::default();
    for stmt in &ast.body {
        if let Stmt::FunctionDef(func_def) = stmt
            && bundler.should_export_symbol(func_def.name.as_ref(), ctx.module_name)
        {
            // This function will be exported, collect variables it references
            crate::visitors::VariableCollector::collect_referenced_vars(
                &func_def.body,
                &mut vars_used_by_exported_functions,
            );
        }
    }

    // Now process the transformed module
    // We'll do the in-place symbol export as we process each statement
    let module_scope_symbols = if let Some(semantic_bundler) = ctx.semantic_bundler {
        debug!(
            "Looking up module ID for '{}' in semantic bundler",
            ctx.module_name
        );
        // Use the central module registry for fast, reliable lookup
        let module_id = if let Some(registry) = bundler.module_info_registry {
            let id = registry.get_id_by_name(ctx.module_name);
            if id.is_some() {
                debug!(
                    "Found module ID for '{}' using module registry",
                    ctx.module_name
                );
            } else {
                debug!("Module '{}' not found in module registry", ctx.module_name);
            }
            id
        } else {
            log::warn!("No module registry available for module ID lookup");
            None
        };

        if let Some(module_id) = module_id {
            if let Some(module_info) = semantic_bundler.get_module_info(module_id) {
                debug!(
                    "Found module-scope symbols for '{}': {:?}",
                    ctx.module_name, module_info.module_scope_symbols
                );
                Some(&module_info.module_scope_symbols)
            } else {
                log::warn!(
                    "No semantic info found for module '{}' (module_id: {:?})",
                    ctx.module_name,
                    module_id
                );
                None
            }
        } else {
            log::warn!(
                "Could not find module ID for '{}' in semantic bundler",
                ctx.module_name
            );
            None
        }
    } else {
        debug!(
            "No semantic bundler provided for module '{}'",
            ctx.module_name
        );
        None
    };

    // First, scan the body to find all built-in names that will be assigned as local variables
    let mut builtin_locals = FxIndexSet::default();
    for stmt in &ast.body {
        let target_opt = match stmt {
            Stmt::Assign(assign) if assign.targets.len() == 1 => {
                if let Expr::Name(target) = &assign.targets[0] {
                    Some(target)
                } else {
                    None
                }
            }
            Stmt::AnnAssign(ann_assign) if ann_assign.value.is_some() => {
                if let Expr::Name(target) = ann_assign.target.as_ref() {
                    Some(target)
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(target) = target_opt
            && ruff_python_stdlib::builtins::is_python_builtin(
                target.id.as_str(),
                ctx.python_version,
                false,
            )
        {
            debug!(
                "Found built-in type '{}' that will be assigned as local variable in init function",
                target.id
            );
            builtin_locals.insert(target.id.to_string());
        }
    }

    // Helper function to get exported module-level variables
    let get_exported_module_vars =
        |bundler: &Bundler, ctx: &ModuleTransformContext| -> FxIndexSet<String> {
            if let Some(ref global_info) = ctx.global_info {
                let all_vars = &global_info.module_level_vars;
                let mut exported_vars = FxIndexSet::default();
                for var in all_vars {
                    if bundler.should_export_symbol(var, ctx.module_name) {
                        exported_vars.insert(var.clone());
                    }
                }
                exported_vars
            } else {
                FxIndexSet::default()
            }
        };

    // Process the body with a new recursive approach
    let processed_body_raw =
        bundler.process_body_recursive(ast.body, ctx.module_name, module_scope_symbols);

    // Filter out accidental attempts to (re)initialize the entry package (__init__) from
    // within submodule init functions, which can create circular initialization.
    let processed_body: Vec<Stmt> = processed_body_raw
        .into_iter()
        .filter(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && assign.targets.len() == 1
                && let Expr::Name(target) = &assign.targets[0]
                && target.id.as_str() == crate::python::constants::INIT_STEM
                && let Expr::Call(call) = assign.value.as_ref()
                && let Expr::Name(func_name) = call.func.as_ref()
                && crate::code_generator::module_registry::is_init_function(func_name.id.as_str())
            {
                debug!(
                    "Skipping entry package __init__ re-initialization inside wrapper init to \
                     avoid circular init"
                );
                return false;
            }
            true
        })
        .collect();

    debug!(
        "Processing init function for module '{}', inlined_import_bindings: {:?}",
        ctx.module_name, inlined_import_bindings
    );
    debug!("Processed body has {} statements", processed_body.len());

    // Declare lifted globals FIRST if any - they need to be declared before any usage
    // But we'll initialize them later after the original variables are defined
    if let Some(ref lifted_names) = lifted_names
        && !lifted_names.is_empty()
    {
        // Declare all lifted globals once (sorted) for deterministic output
        let mut lifted: Vec<&str> = lifted_names
            .values()
            .map(std::string::String::as_str)
            .collect();
        lifted.sort_unstable();
        body.push(ast_builder::statements::global(lifted));
    }

    // Track which lifted globals we've already initialized to avoid duplicates
    let mut initialized_lifted_globals = FxIndexSet::default();

    // First pass: collect all wrapper module namespace variables that need global declarations
    // Use a visitor to properly traverse the AST
    let wrapper_globals_needed = {
        use ruff_python_ast::visitor::source_order::{self, SourceOrderVisitor};

        struct WrapperGlobalCollector {
            globals_needed: FxIndexSet<String>,
        }

        impl WrapperGlobalCollector {
            fn new() -> Self {
                Self {
                    globals_needed: FxIndexSet::default(),
                }
            }

            fn collect(processed_body: &[Stmt]) -> FxIndexSet<String> {
                let mut collector = Self::new();
                for stmt in processed_body {
                    collector.visit_stmt(stmt);
                }
                collector.globals_needed
            }
        }

        impl<'a> SourceOrderVisitor<'a> for WrapperGlobalCollector {
            fn visit_stmt(&mut self, stmt: &'a Stmt) {
                if let Stmt::Assign(assign) = stmt {
                    // Check if the value is a call to an init function
                    if let Expr::Call(call) = assign.value.as_ref()
                        && let Expr::Name(name) = call.func.as_ref()
                        && crate::code_generator::module_registry::is_init_function(
                            name.id.as_str(),
                        )
                    {
                        // Check if the assignment target is also used as an argument
                        if assign.targets.len() == 1
                            && let Expr::Name(target) = &assign.targets[0]
                        {
                            // Check if the target is also passed as an argument
                            let needs_global = call.arguments.args.iter().any(|arg| {
                                matches!(arg, Expr::Name(arg_name) if arg_name.id.as_str() == target.id.as_str())
                            });
                            if needs_global {
                                self.globals_needed.insert(target.id.to_string());
                            }
                        }
                    }
                }
                // Continue traversing the statement tree
                source_order::walk_stmt(self, stmt);
            }
        }

        WrapperGlobalCollector::collect(&processed_body)
    };

    // Add global declarations for wrapper module namespace variables at the beginning
    if !wrapper_globals_needed.is_empty() {
        let mut globals: Vec<&str> = wrapper_globals_needed
            .iter()
            .map(std::string::String::as_str)
            .collect();
        globals.sort_unstable();
        body.push(ast_builder::statements::global(globals));
    }

    // Process each statement from the transformed module body
    for (idx, stmt) in processed_body.into_iter().enumerate() {
        match &stmt {
            Stmt::Assign(_) => debug!("Processing statement {idx} in init function: Assign"),
            Stmt::ImportFrom(_) => {
                debug!("Processing statement {idx} in init function: ImportFrom");
            }
            Stmt::Expr(_) => debug!("Processing statement {idx} in init function: Expr"),
            Stmt::For(_) => debug!("Processing statement {idx} in init function: For"),
            _ => debug!("Processing statement {idx} in init function: Other"),
        }
        match &stmt {
            Stmt::Import(_import_stmt) => {
                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    body.push(stmt.clone());
                }
            }
            Stmt::ImportFrom(import_from) => {
                // Skip __future__ imports - they cannot appear inside functions
                if import_from
                    .module
                    .as_ref()
                    .map(ruff_python_ast::Identifier::as_str)
                    == Some("__future__")
                {
                    continue;
                }

                // Skip imports that are already hoisted
                if !import_deduplicator::is_hoisted_import(bundler, &stmt) {
                    body.push(stmt.clone());
                }

                // Module attribute assignments for imported names are already handled by
                // process_body_recursive in the bundler, so we don't need to add them here
            }
            Stmt::ClassDef(class_def) => {
                // Add class definition
                body.push(stmt.clone());

                let symbol_name = class_def.name.to_string();

                // Note: We set __module__ for the class, but Python still shows the full scope path
                // in the class repr when it's defined inside a function. This is expected behavior.
                // Setting __module__ helps with introspection but doesn't change the repr.
                body.push(ast_builder::statements::assign_attribute(
                    &symbol_name,
                    "__module__",
                    ast_builder::expressions::string_literal(ctx.module_name),
                ));

                // Set as module attribute via centralized helper
                emit_module_attr_if_exportable(
                    bundler,
                    &symbol_name,
                    ctx.module_name,
                    &mut body,
                    module_scope_symbols,
                    None, // not a lifted var
                );
            }
            Stmt::FunctionDef(func_def) => {
                // Clone the function for transformation
                let mut func_def_clone = func_def.clone();

                // Transform nested functions to use module attributes for module-level vars
                if let Some(ref global_info) = ctx.global_info {
                    bundler.transform_nested_function_for_module_vars_with_global_info(
                        &mut func_def_clone,
                        &global_info.module_level_vars,
                        &global_info.global_declarations,
                        lifted_names.as_ref(),
                        SELF_PARAM,
                    );
                }

                // Add transformed function definition
                body.push(Stmt::FunctionDef(func_def_clone));

                // Set as module attribute via centralized helper
                let symbol_name = func_def.name.to_string();
                emit_module_attr_if_exportable(
                    bundler,
                    &symbol_name,
                    ctx.module_name,
                    &mut body,
                    module_scope_symbols,
                    None, // not a lifted var
                );
            }
            Stmt::Assign(assign) => {
                // Handle __all__ assignments - skip unless it's referenced elsewhere
                if let Some(name) = expression_handlers::extract_simple_assign_target(assign)
                    && name == "__all__"
                {
                    if all_is_referenced {
                        // __all__ is referenced elsewhere, include the assignment
                        body.push(stmt.clone());
                    }
                    // Skip further processing for __all__ assignments
                    continue;
                }

                // Skip self-referential assignments like `process = process`
                // These are meaningless in the init function context and cause errors
                if expression_handlers::is_self_referential_assignment(assign, ctx.python_version) {
                    debug!(
                        "Skipping self-referential assignment in module '{}': {:?}",
                        ctx.module_name,
                        assign.targets.first().and_then(|t| match t {
                            Expr::Name(name) => Some(name.id.as_str()),
                            _ => None,
                        })
                    );
                } else {
                    // Clone and transform the assignment to handle __name__ references
                    let mut assign_clone = assign.clone();

                    // Use actual module-level variables if available, but filter to only
                    // exported ones
                    let module_level_vars = get_exported_module_vars(bundler, ctx);

                    // Special handling for assignments involving built-in types
                    // We need to transform any reference to a built-in that will be assigned
                    // as a local variable later in this function
                    transform_expr_for_builtin_shadowing(&mut assign_clone.value, &builtin_locals);

                    // Also transform module-level variable references
                    // Inside the init function, use "self" to refer to the module
                    transform_expr_for_module_vars(
                        &mut assign_clone.value,
                        &module_level_vars,
                        SELF_PARAM, // Use "self" instead of module_var_name inside init function
                        ctx.python_version,
                    );

                    // For simple assignments, also set as module attribute if it should be
                    // exported
                    body.push(Stmt::Assign(assign_clone.clone()));

                    // If this variable is being lifted to a global, update the global
                    let mut lifted_var_handled = false;
                    if let Some(ref lifted_names) = lifted_names
                        && let Some(name) =
                            expression_handlers::extract_simple_assign_target(&assign_clone)
                        && let Some(lifted_name) = lifted_names.get(&name)
                    {
                        // Always propagate to the lifted binding to keep it in sync
                        body.push(ast_builder::statements::assign(
                            vec![ast_builder::expressions::name(
                                lifted_name,
                                ExprContext::Store,
                            )],
                            ast_builder::expressions::name(&name, ExprContext::Load),
                        ));

                        // Keep the module attribute consistent with the current value
                        body.push(
                            crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                                SELF_PARAM,
                                &name,
                                lifted_name,
                            ),
                        );

                        if initialized_lifted_globals.insert(name.clone()) {
                            debug!("Initialized lifted global '{lifted_name}' from '{name}'");
                        } else {
                            debug!(
                                "Refreshed lifted global '{lifted_name}' after reassignment of \
                                 '{name}'"
                            );
                        }
                        lifted_var_handled = true;
                    }

                    // Skip further module attribute handling if this was a lifted variable
                    if lifted_var_handled {
                        // Already handled as a lifted variable
                    } else if let Some(name) =
                        expression_handlers::extract_simple_assign_target(assign)
                    {
                        debug!(
                            "Checking assignment '{}' in module '{}' (inlined_import_bindings: \
                             {:?})",
                            name, ctx.module_name, inlined_import_bindings
                        );

                        if inlined_import_bindings.contains(&name) {
                            // This was imported from an inlined module
                            // Module attributes for imports are now handled by import_transformer
                            // to ensure correct value assignment (original_name vs local_name)
                            debug!(
                                "Skipping module attribute for imported symbol '{name}' - handled \
                                 by import_transformer"
                            );
                        } else if vars_used_by_exported_functions.contains(&name) {
                            // Check if this variable is used by exported functions
                            // Use a special case: if no scope info available, include vars used
                            // by exported functions
                            let should_include =
                                module_scope_symbols.is_none_or(|symbols| symbols.contains(&name));

                            if should_include {
                                debug!("Exporting '{name}' as it's used by exported functions");
                                body.push(crate::code_generator::module_registry::create_module_attr_assignment(
                                    SELF_PARAM,
                                    &name,
                                ));
                            }
                        } else {
                            // Regular assignment, use the normal export logic
                            add_module_attr_if_exported(
                                bundler,
                                assign,
                                ctx.module_name,
                                &mut body,
                                module_scope_symbols,
                            );
                        }
                    } else {
                        // Not a simple assignment
                        add_module_attr_if_exported(
                            bundler,
                            assign,
                            ctx.module_name,
                            &mut body,
                            module_scope_symbols,
                        );
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Handle annotated assignments similar to regular assignments
                if ann_assign.value.is_some() {
                    // Skip __all__ annotated assignments unless it's referenced elsewhere
                    if let Expr::Name(target) = ann_assign.target.as_ref()
                        && target.id.as_str() == "__all__"
                        && !all_is_referenced
                    {
                        continue;
                    }

                    let mut ann_assign_clone = ann_assign.clone();

                    // Use actual module-level variables if available, but filter to only exported
                    // ones
                    let module_level_vars = get_exported_module_vars(bundler, ctx);

                    // Transform references to built-ins that will be shadowed
                    if let Some(ref mut value) = ann_assign_clone.value {
                        transform_expr_for_builtin_shadowing(value, &builtin_locals);

                        // Also transform module-level variable references
                        // Inside the init function, use "self" to refer to the module
                        transform_expr_for_module_vars(
                            value,
                            &module_level_vars,
                            SELF_PARAM, /* Use "self" instead of module_var_name inside init
                                         * function */
                            ctx.python_version,
                        );
                    }

                    // Transform the annotation expression as well
                    transform_expr_for_builtin_shadowing(
                        &mut ann_assign_clone.annotation,
                        &builtin_locals,
                    );
                    transform_expr_for_module_vars(
                        &mut ann_assign_clone.annotation,
                        &module_level_vars,
                        SELF_PARAM, // Use "self" instead of module_var_name inside init function
                        ctx.python_version,
                    );

                    body.push(Stmt::AnnAssign(ann_assign_clone.clone()));

                    // If this variable is being lifted to a global, handle it
                    if let Some(ref lifted_names) = lifted_names
                        && let Expr::Name(target) = ann_assign_clone.target.as_ref()
                        && let Some(lifted_name) = lifted_names.get(target.id.as_str())
                    {
                        // Always propagate to the lifted binding to keep it in sync
                        body.push(ast_builder::statements::assign(
                            vec![ast_builder::expressions::name(
                                lifted_name,
                                ExprContext::Store,
                            )],
                            ast_builder::expressions::name(&target.id, ExprContext::Load),
                        ));

                        // Keep the module attribute consistent with the current value
                        body.push(crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                            SELF_PARAM,
                            target.id.as_str(),
                            lifted_name,
                        ));

                        if initialized_lifted_globals.insert(target.id.to_string()) {
                            debug!(
                                "Initialized lifted global '{lifted_name}' from annotated \
                                 assignment '{}'",
                                target.id
                            );
                        } else {
                            debug!(
                                "Refreshed lifted global '{lifted_name}' after annotated \
                                 reassignment '{}'",
                                target.id
                            );
                        }
                    }

                    // Also set as module attribute if it should be exported (for non-lifted vars)
                    if let Expr::Name(target) = ann_assign.target.as_ref() {
                        emit_module_attr_if_exportable(
                            bundler,
                            &target.id,
                            ctx.module_name,
                            &mut body,
                            module_scope_symbols,
                            lifted_names.as_ref(),
                        );
                    }
                } else {
                    // Type annotation without value, just add it
                    body.push(stmt.clone());
                }
            }
            Stmt::Try(_try_stmt) => {
                // Let the new conditional logic in bundler.rs handle try/except processing
                // This avoids duplicate module attribute assignments
                body.push(stmt.clone());
            }
            _ => {
                // Clone and transform other statements to handle __name__ references
                let mut stmt_clone = stmt.clone();
                // Use actual module-level variables if available, but filter to only exported
                // ones
                let module_level_vars = if let Some(ref global_info) = ctx.global_info {
                    let all_vars = &global_info.module_level_vars;
                    let mut exported_vars = FxIndexSet::default();
                    for var in all_vars {
                        if bundler.should_export_symbol(var, ctx.module_name) {
                            exported_vars.insert(var.clone());
                        }
                    }
                    exported_vars
                } else {
                    FxIndexSet::default()
                };
                let transform_ctx = ModuleVarTransformContext {
                    bundler,
                    module_level_vars: &module_level_vars,
                    module_var_name: SELF_PARAM, /* Use "self" instead of module_var_name inside
                                                  * init function */
                    global_declarations: ctx.global_info.as_ref().map(|g| &g.global_declarations),
                    lifted_names: lifted_names.as_ref(),
                    python_version: ctx.python_version,
                };
                transform_stmt_for_module_vars_with_bundler(&mut stmt_clone, &transform_ctx);
                body.push(stmt_clone);
            }
        }
    }

    // Set submodules as attributes on this module BEFORE processing deferred imports
    // This is needed because deferred imports may reference these submodules
    let current_module_prefix = format!("{}.", ctx.module_name);
    let mut submodules_to_add = Vec::new();

    // Collect all direct submodules
    for module_id in &bundler.bundled_modules {
        let module_name = bundler
            .resolver
            .get_module_name(*module_id)
            .expect("Module name should exist");
        if module_name.starts_with(&current_module_prefix) {
            let relative_name = &module_name[current_module_prefix.len()..];
            // Only handle direct children, not nested submodules
            if !relative_name.contains('.') {
                submodules_to_add.push((module_name.clone(), relative_name.to_string()));
            }
        }
    }

    // Also check inlined modules
    for module_id in &bundler.inlined_modules {
        let module_name = bundler
            .resolver
            .get_module_name(*module_id)
            .expect("Module name should exist");
        if module_name.starts_with(&current_module_prefix) {
            let relative_name = &module_name[current_module_prefix.len()..];
            // Only handle direct children, not nested submodules
            if !relative_name.contains('.') {
                submodules_to_add.push((module_name.clone(), relative_name.to_string()));
            }
        }
    }

    // Now add the submodules as attributes
    debug!(
        "Submodules to add for {}: {:?}",
        ctx.module_name, submodules_to_add
    );
    for (full_name, relative_name) in submodules_to_add {
        // CRITICAL: Check if this wrapper module already imports a symbol with the same name
        // as the submodule. If it does, skip setting the submodule namespace to avoid overwriting
        // the imported symbol. For example, if package.__init__ imports `__version__` from
        // `.__version__`, we should NOT overwrite that with the namespace object for
        // package.__version__.
        let symbol_already_imported = imported_symbols.contains(&relative_name);

        if symbol_already_imported {
            debug!(
                "Skipping submodule namespace assignment for {full_name} because symbol \
                 '{relative_name}' is already imported"
            );
            continue;
        }

        debug!(
            "Setting submodule {} as attribute {} on {}",
            full_name, relative_name, ctx.module_name
        );

        if bundler
            .get_module_id(&full_name)
            .is_some_and(|id| bundler.inlined_modules.contains(&id))
        {
            // Check if we're inside a wrapper function context
            // If we are, skip creating namespace for inlined submodules because
            // their symbols are at global scope and can't be referenced from
            // inside the wrapper function at definition time
            if ctx.is_wrapper_body {
                debug!(
                    "Skipping namespace creation for inlined submodule '{}' inside wrapper module \
                     '{}'",
                    full_name, ctx.module_name
                );
                // Bind existing global namespace object to module.<relative_name>
                // Example: module.submodule = pkg_submodule
                // IMPORTANT: This references a namespace variable (e.g., package___version__) that
                // MUST already exist at the global scope. These namespace objects
                // are pre-created earlier in the bundling pipeline via
                // namespace_manager::generate_submodule_attributes_with_exclusions()
                // (invoked from bundler.rs). If you get "NameError: name 'package___version__' is
                // not defined", the pre-creation step likely ran too late.
                let namespace_var = sanitize_module_name_for_identifier(&full_name);
                body.push(
                    crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                        SELF_PARAM,
                        &relative_name,
                        &namespace_var,
                    ),
                );
            } else {
                // For non-wrapper contexts (like global inlined modules), create the namespace
                let create_namespace_stmts = create_namespace_for_inlined_submodule(
                    bundler,
                    &full_name,
                    &relative_name,
                    SELF_PARAM,
                    symbol_renames,
                );
                body.extend(create_namespace_stmts);
            }
        } else {
            // For wrapped submodules, we'll set them up later when they're initialized
            // For now, just skip - the parent module will get the submodule reference
            // when the submodule's init function is called
        }
    }

    // Note: deferred imports functionality has been removed
    // Remaining deferred imports were previously added here

    // Skip __all__ generation - it has no meaning for types.SimpleNamespace objects

    // Add stdlib re-exports to the module namespace
    for (local_name, proxy_path) in stdlib_reexports {
        // Create: _cribo_module.local_name = _cribo.module.symbol
        // Parse the proxy path to create the attribute access expression
        let parts: Vec<&str> = proxy_path.split('.').collect();
        let value_expr = ast_builder::expressions::dotted_name(&parts, ExprContext::Load);

        body.push(ast_builder::statements::assign(
            vec![ast_builder::expressions::attribute(
                ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
                &local_name,
                ExprContext::Store,
            )],
            value_expr,
        ));

        debug!(
            "Added stdlib re-export to wrapper module '{}': {} = {}",
            ctx.module_name, local_name, proxy_path
        );
    }

    // For explicit imports from inlined modules that don't create assignments,
    // we still need to set them as module attributes if they're exported
    // Note: wildcard imports (imports_from_inlined) were already handled earlier
    // before processing the body, so we only need to handle explicit imports here
    for imported_name in inlined_import_bindings {
        if bundler.should_export_symbol(&imported_name, ctx.module_name) {
            // Check if we already have a module attribute assignment for this
            let already_assigned = body.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                    && let Expr::Name(name) = &*attr.value
                {
                    return name.id.as_str() == SELF_PARAM && attr.attr == imported_name;
                }
                false
            });

            if !already_assigned {
                body.push(
                    crate::code_generator::module_registry::create_module_attr_assignment(
                        SELF_PARAM,
                        &imported_name,
                    ),
                );
            }
        }
    }

    // Transform globals() calls to module.__dict__ in the entire body
    for stmt in &mut body {
        transform_globals_in_stmt(stmt, &module_var_name);
        // Transform locals() calls to vars(module_var) in the entire body
        transform_locals_in_stmt(stmt, &module_var_name);
    }

    // Mark as fully initialized (module is now fully populated)
    // self.__initialized__ = True  (set this first!)
    // self.__initializing__ = False
    body.push(ast_builder::statements::assign_attribute(
        SELF_PARAM,
        "__initialized__",
        ast_builder::expressions::bool_literal(true),
    ));
    body.push(ast_builder::statements::assign_attribute(
        SELF_PARAM,
        "__initializing__",
        ast_builder::expressions::bool_literal(false),
    ));

    // Return the module object (self)
    body.push(ast_builder::statements::return_stmt(Some(
        ast_builder::expressions::name(SELF_PARAM, ExprContext::Load),
    )));

    // Create the init function parameters with 'self' parameter
    let self_param = ruff_python_ast::ParameterWithDefault {
        range: TextRange::default(),
        parameter: ruff_python_ast::Parameter {
            range: TextRange::default(),
            name: Identifier::new(SELF_PARAM, TextRange::default()),
            annotation: None,
            node_index: AtomicNodeIndex::dummy(),
        },
        default: None,
        node_index: AtomicNodeIndex::dummy(),
    };

    let parameters = ruff_python_ast::Parameters {
        node_index: AtomicNodeIndex::dummy(),
        posonlyargs: vec![],
        args: vec![self_param],
        vararg: None,
        kwonlyargs: vec![],
        kwarg: None,
        range: TextRange::default(),
    };

    // No decorator - we manage initialization ourselves
    ast_builder::statements::function_def(
        init_func_name,
        parameters,
        body,
        vec![], // No decorators
        None,   // No return type annotation
        false,  // Not async
    )
}

/// Transform an expression to use module attributes for module-level variables
fn transform_expr_for_module_vars(
    expr: &mut Expr,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match expr {
        Expr::Name(name) if name.ctx == ExprContext::Load => {
            // Special case: transform __name__ to module.__name__
            if name.id.as_str() == "__name__" {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    "__name__",
                    ExprContext::Load,
                );
            }
            // Check if this is a reference to a module-level variable
            // BUT exclude Python builtins from transformation
            else if module_level_vars.contains(name.id.as_str())
                && !ruff_python_stdlib::builtins::is_python_builtin(
                    name.id.as_str(),
                    python_version,
                    false,
                )
            {
                // Transform to module.var
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    name.id.as_str(),
                    ExprContext::Load,
                );
            }
        }
        // Recursively handle other expressions
        Expr::Call(call) => {
            transform_expr_for_module_vars(
                &mut call.func,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars(
                    arg,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for kw in &mut call.arguments.keywords {
                transform_expr_for_module_vars(
                    &mut kw.value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars(
                &mut attr.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars(
                &mut binop.left,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut binop.right,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::UnaryOp(unop) => {
            transform_expr_for_module_vars(
                &mut unop.operand,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::If(if_expr) => {
            transform_expr_for_module_vars(
                &mut if_expr.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut if_expr.body,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut if_expr.orelse,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_module_vars(
                        key,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                transform_expr_for_module_vars(
                    &mut item.value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Subscript(sub) => {
            transform_expr_for_module_vars(
                &mut sub.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut sub.slice,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                transform_expr_for_module_vars(
                    elem,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Lambda(lambda) => {
            // Note: Lambda parameters create a new scope, so we don't transform them
            transform_expr_for_module_vars(
                &mut lambda.body,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Compare(cmp) => {
            transform_expr_for_module_vars(
                &mut cmp.left,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for comp in &mut cmp.comparators {
                transform_expr_for_module_vars(
                    comp,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::ListComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::SetComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::DictComp(comp) => {
            transform_expr_for_module_vars(
                &mut comp.key,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut comp.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut comp.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::Generator(r#gen) => {
            transform_expr_for_module_vars(
                &mut r#gen.elt,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for generator in &mut r#gen.generators {
                transform_expr_for_module_vars(
                    &mut generator.iter,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                for if_clause in &mut generator.ifs {
                    transform_expr_for_module_vars(
                        if_clause,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Expr::Await(await_expr) => {
            transform_expr_for_module_vars(
                &mut await_expr.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_expr_for_module_vars(
                &mut yield_from.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Starred(starred) => {
            transform_expr_for_module_vars(
                &mut starred.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Named(named) => {
            transform_expr_for_module_vars(
                &mut named.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                transform_expr_for_module_vars(
                    lower,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut upper) = slice.upper {
                transform_expr_for_module_vars(
                    upper,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut step) = slice.step {
                transform_expr_for_module_vars(
                    step,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::FString(_fstring) => {
            // F-strings require special handling due to their immutable structure
            // For now, we skip transforming f-strings as they would need to be rebuilt
            // TODO: Implement f-string transformation if needed
        }
        // Literals and name expressions that don't need transformation
        // - Literals don't contain variable references
        // - Name expressions that don't match the conditional pattern (e.g., Store context)
        Expr::StringLiteral(_)
        | Expr::BytesLiteral(_)
        | Expr::NumberLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::EllipsisLiteral(_)
        | Expr::TString(_)
        | Expr::IpyEscapeCommand(_)
        | Expr::Name(_) => {}
    }
}

/// Transform a statement to use module attributes for module-level variables
fn transform_stmt_for_module_vars(
    stmt: &mut Stmt,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Assign(assign) => {
            // Transform assignment targets and values
            for target in &mut assign.targets {
                transform_expr_for_module_vars(
                    target,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            transform_expr_for_module_vars(
                &mut assign.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars(
                &mut expr_stmt.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars(
                &mut if_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars(
                        condition,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_expr_for_module_vars(
                &mut for_stmt.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut for_stmt.iter,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut for_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars(
                &mut while_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut while_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::With(with_stmt) => {
            for item in &mut with_stmt.items {
                transform_expr_for_module_vars(
                    &mut item.context_expr,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
                if let Some(ref mut optional_vars) = item.optional_vars {
                    transform_expr_for_module_vars(
                        optional_vars,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            for stmt in &mut with_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Try(try_stmt) => {
            for stmt in &mut try_stmt.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for handler in &mut try_stmt.handlers {
                let ExceptHandler::ExceptHandler(except_handler) = handler;
                if let Some(ref mut type_) = except_handler.type_ {
                    transform_expr_for_module_vars(
                        type_,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut except_handler.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            for stmt in &mut try_stmt.orelse {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            for stmt in &mut try_stmt.finalbody {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Raise(raise_stmt) => {
            if let Some(ref mut exc) = raise_stmt.exc {
                transform_expr_for_module_vars(
                    exc,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            if let Some(ref mut cause) = raise_stmt.cause {
                transform_expr_for_module_vars(
                    cause,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::ClassDef(class_def) => {
            // Transform decorators
            for decorator in &mut class_def.decorator_list {
                transform_expr_for_module_vars(
                    &mut decorator.expression,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
            // Transform class arguments (base classes and keyword arguments)
            if let Some(ref mut arguments) = class_def.arguments {
                for arg in &mut arguments.args {
                    transform_expr_for_module_vars(
                        arg,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for keyword in &mut arguments.keywords {
                    transform_expr_for_module_vars(
                        &mut keyword.value,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
            // Transform class body
            for stmt in &mut class_def.body {
                transform_stmt_for_module_vars(
                    stmt,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::AugAssign(aug_assign) => {
            transform_expr_for_module_vars(
                &mut aug_assign.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut aug_assign.value,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::AnnAssign(ann_assign) => {
            transform_expr_for_module_vars(
                &mut ann_assign.target,
                module_level_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars(
                &mut ann_assign.annotation,
                module_level_vars,
                module_var_name,
                python_version,
            );
            if let Some(ref mut value) = ann_assign.value {
                transform_expr_for_module_vars(
                    value,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Delete(delete_stmt) => {
            for target in &mut delete_stmt.targets {
                transform_expr_for_module_vars(
                    target,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::Match(match_stmt) => {
            transform_expr_for_module_vars(
                &mut match_stmt.subject,
                module_level_vars,
                module_var_name,
                python_version,
            );
            // Match cases have complex patterns that may need specialized handling
            // For now, we'll focus on transforming the guard expressions and bodies
            for case in &mut match_stmt.cases {
                if let Some(ref mut guard) = case.guard {
                    transform_expr_for_module_vars(
                        guard,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut case.body {
                    transform_stmt_for_module_vars(
                        stmt,
                        module_level_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::Assert(assert_stmt) => {
            transform_expr_for_module_vars(
                &mut assert_stmt.test,
                module_level_vars,
                module_var_name,
                python_version,
            );
            if let Some(ref mut msg) = assert_stmt.msg {
                transform_expr_for_module_vars(
                    msg,
                    module_level_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::TypeAlias(_)
        | Stmt::Import(_)
        | Stmt::ImportFrom(_)
        | Stmt::Global(_)
        | Stmt::Nonlocal(_)
        | Stmt::Pass(_)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::IpyEscapeCommand(_) => {
            // These statement types don't contain expressions that need transformation
        }
    }
}

/// Context for transforming statements with module variable awareness
struct ModuleVarTransformContext<'a> {
    bundler: &'a Bundler<'a>,
    module_level_vars: &'a FxIndexSet<String>,
    module_var_name: &'a str,
    global_declarations: Option<&'a FxIndexMap<String, Vec<ruff_text_size::TextRange>>>,
    lifted_names: Option<&'a FxIndexMap<String, String>>,
    python_version: u8,
}

/// Transform a statement to use module attributes for module-level variables,
/// with awareness of lifted globals for nested functions
fn transform_stmt_for_module_vars_with_bundler(stmt: &mut Stmt, ctx: &ModuleVarTransformContext) {
    if let Stmt::FunctionDef(nested_func) = stmt {
        // For function definitions, use the global-aware transformation
        if let Some(globals_map) = ctx.global_declarations {
            ctx.bundler
                .transform_nested_function_for_module_vars_with_global_info(
                    nested_func,
                    ctx.module_level_vars,
                    globals_map,
                    ctx.lifted_names,
                    ctx.module_var_name,
                );
        } else {
            // Fallback to legacy path when no global info is available
            transform_nested_function_for_module_vars(
                nested_func,
                ctx.module_level_vars,
                ctx.module_var_name,
                ctx.python_version,
            );
        }
        return;
    }
    // Non-function statements: reuse the existing traversal
    transform_stmt_for_module_vars(
        stmt,
        ctx.module_level_vars,
        ctx.module_var_name,
        ctx.python_version,
    );
}

/// Transform nested function to use module attributes for module-level variables
fn transform_nested_function_for_module_vars(
    func_def: &mut StmtFunctionDef,
    module_level_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    // Collect local variables defined in this function
    let mut local_vars = FxIndexSet::default();

    // Add function parameters to local variables
    for param in &func_def.parameters.args {
        local_vars.insert(param.parameter.name.to_string());
    }
    for param in &func_def.parameters.posonlyargs {
        local_vars.insert(param.parameter.name.to_string());
    }
    for param in &func_def.parameters.kwonlyargs {
        local_vars.insert(param.parameter.name.to_string());
    }
    if let Some(ref vararg) = func_def.parameters.vararg {
        local_vars.insert(vararg.name.to_string());
    }
    if let Some(ref kwarg) = func_def.parameters.kwarg {
        local_vars.insert(kwarg.name.to_string());
    }

    // Collect all local variables assigned in the function body
    collect_local_vars(&func_def.body, &mut local_vars);

    // Transform the function body, excluding local variables
    for stmt in &mut func_def.body {
        transform_stmt_for_module_vars_with_locals(
            stmt,
            module_level_vars,
            &local_vars,
            module_var_name,
            python_version,
        );
    }
}

/// Collect local variables defined in a list of statements
fn collect_local_vars(stmts: &[Stmt], local_vars: &mut FxIndexSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::Assign(assign) => {
                // Collect assignment targets as local variables
                for target in &assign.targets {
                    if let Expr::Name(name) = target {
                        local_vars.insert(name.id.to_string());
                    }
                }
            }
            Stmt::AnnAssign(ann_assign) => {
                // Collect annotated assignment targets
                if let Expr::Name(name) = ann_assign.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
            }
            Stmt::For(for_stmt) => {
                // Collect for loop targets
                if let Expr::Name(name) = for_stmt.target.as_ref() {
                    local_vars.insert(name.id.to_string());
                }
                // Recursively collect from body
                collect_local_vars(&for_stmt.body, local_vars);
                collect_local_vars(&for_stmt.orelse, local_vars);
            }
            Stmt::If(if_stmt) => {
                // Recursively collect from branches
                collect_local_vars(&if_stmt.body, local_vars);
                for clause in &if_stmt.elif_else_clauses {
                    collect_local_vars(&clause.body, local_vars);
                }
            }
            Stmt::While(while_stmt) => {
                collect_local_vars(&while_stmt.body, local_vars);
                collect_local_vars(&while_stmt.orelse, local_vars);
            }
            Stmt::With(with_stmt) => {
                // Collect with statement targets
                for item in &with_stmt.items {
                    if let Some(ref optional_vars) = item.optional_vars
                        && let Expr::Name(name) = optional_vars.as_ref()
                    {
                        local_vars.insert(name.id.to_string());
                    }
                }
                collect_local_vars(&with_stmt.body, local_vars);
            }
            Stmt::Try(try_stmt) => {
                collect_local_vars(&try_stmt.body, local_vars);
                for handler in &try_stmt.handlers {
                    let ExceptHandler::ExceptHandler(eh) = handler;
                    // Collect exception name if present
                    if let Some(ref name) = eh.name {
                        local_vars.insert(name.to_string());
                    }
                    collect_local_vars(&eh.body, local_vars);
                }
                collect_local_vars(&try_stmt.orelse, local_vars);
                collect_local_vars(&try_stmt.finalbody, local_vars);
            }
            Stmt::FunctionDef(func_def) => {
                // Function definitions create local names
                local_vars.insert(func_def.name.to_string());
            }
            Stmt::ClassDef(class_def) => {
                // Class definitions create local names
                local_vars.insert(class_def.name.to_string());
            }
            _ => {
                // Other statements don't introduce new local variables
            }
        }
    }
}

/// Transform a statement with awareness of local variables
fn transform_stmt_for_module_vars_with_locals(
    stmt: &mut Stmt,
    module_level_vars: &FxIndexSet<String>,
    local_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match stmt {
        Stmt::FunctionDef(nested_func) => {
            // Recursively transform nested functions
            transform_nested_function_for_module_vars(
                nested_func,
                module_level_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Assign(assign) => {
            // Transform assignment targets and values
            for target in &mut assign.targets {
                transform_expr_for_module_vars_with_locals(
                    target,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            transform_expr_for_module_vars_with_locals(
                &mut assign.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Expr(expr_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut expr_stmt.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Stmt::Return(return_stmt) => {
            if let Some(value) = &mut return_stmt.value {
                transform_expr_for_module_vars_with_locals(
                    value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::If(if_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut if_stmt.test,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut if_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            for clause in &mut if_stmt.elif_else_clauses {
                if let Some(condition) = &mut clause.test {
                    transform_expr_for_module_vars_with_locals(
                        condition,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
                for stmt in &mut clause.body {
                    transform_stmt_for_module_vars_with_locals(
                        stmt,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
            }
        }
        Stmt::For(for_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut for_stmt.target,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut for_stmt.iter,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut for_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Stmt::While(while_stmt) => {
            transform_expr_for_module_vars_with_locals(
                &mut while_stmt.test,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for stmt in &mut while_stmt.body {
                transform_stmt_for_module_vars_with_locals(
                    stmt,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        _ => {
            // Handle other statement types as needed
        }
    }
}

/// Transform an expression with awareness of local variables
fn transform_expr_for_module_vars_with_locals(
    expr: &mut Expr,
    module_level_vars: &FxIndexSet<String>,
    local_vars: &FxIndexSet<String>,
    module_var_name: &str,
    python_version: u8,
) {
    match expr {
        Expr::Name(name_expr) => {
            let name_str = name_expr.id.as_str();

            // Special case: transform __name__ to module.__name__
            if name_str == "__name__" && matches!(name_expr.ctx, ExprContext::Load) {
                // Transform __name__ -> module.__name__
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    "__name__",
                    ExprContext::Load,
                );
            }
            // If this is a module-level variable being read AND NOT a local variable AND NOT a
            // builtin, transform to module.var
            else if module_level_vars.contains(name_str)
                && !local_vars.contains(name_str)
                && !ruff_python_stdlib::builtins::is_python_builtin(name_str, python_version, false)
                && matches!(name_expr.ctx, ExprContext::Load)
            {
                // Transform foo -> module.foo
                *expr = ast_builder::expressions::attribute(
                    ast_builder::expressions::name(module_var_name, ExprContext::Load),
                    name_str,
                    ExprContext::Load,
                );
            }
        }
        Expr::Call(call) => {
            transform_expr_for_module_vars_with_locals(
                &mut call.func,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            for arg in &mut call.arguments.args {
                transform_expr_for_module_vars_with_locals(
                    arg,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
            for keyword in &mut call.arguments.keywords {
                transform_expr_for_module_vars_with_locals(
                    &mut keyword.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::BinOp(binop) => {
            transform_expr_for_module_vars_with_locals(
                &mut binop.left,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut binop.right,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(key) = &mut item.key {
                    transform_expr_for_module_vars_with_locals(
                        key,
                        module_level_vars,
                        local_vars,
                        module_var_name,
                        python_version,
                    );
                }
                transform_expr_for_module_vars_with_locals(
                    &mut item.value,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::List(list_expr) => {
            for elem in &mut list_expr.elts {
                transform_expr_for_module_vars_with_locals(
                    elem,
                    module_level_vars,
                    local_vars,
                    module_var_name,
                    python_version,
                );
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_module_vars_with_locals(
                &mut attr.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_module_vars_with_locals(
                &mut subscript.value,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
            transform_expr_for_module_vars_with_locals(
                &mut subscript.slice,
                module_level_vars,
                local_vars,
                module_var_name,
                python_version,
            );
        }
        _ => {
            // Handle other expression types as needed
        }
    }
}

// pub fn create_module_object_stmt(module_name: &str, _module_path: &Path) -> Vec<Stmt> {
//     let module_call = ast_builder::expressions::call(
//         ast_builder::expressions::simple_namespace_ctor(),
//         vec![],
//         vec![],
//     );
//
//     vec![
//         // self = types.SimpleNamespace()
//         ast_builder::statements::simple_assign("self", module_call),
//         // self.__name__ = "module_name"
//         ast_builder::statements::assign_attribute(
//             "self",
//             "__name__",
//             ast_builder::expressions::string_literal(module_name),
//         ),
//     ]
// }

/// Transform AST to use lifted globals
/// This is a thin wrapper around the bundler method to maintain module boundaries
pub fn transform_ast_with_lifted_globals(
    bundler: &Bundler,
    ast: &mut ModModule,
    lifted_names: &FxIndexMap<String, String>,
    global_info: &crate::semantic_bundler::ModuleGlobalInfo,
) {
    bundler.transform_ast_with_lifted_globals(ast, lifted_names, global_info);
}

/// Transform expressions to handle built-in name shadowing in init functions
/// When a built-in name like 'str' is assigned as a local variable in the function,
/// any reference to that built-in before the assignment needs to use __builtins__.name
fn transform_expr_for_builtin_shadowing(expr: &mut Expr, builtin_locals: &FxIndexSet<String>) {
    match expr {
        Expr::Name(name) if name.ctx == ExprContext::Load => {
            // If this name refers to a built-in that will be shadowed by a local assignment,
            // transform it to use __builtins__.name
            if builtin_locals.contains(name.id.as_str()) {
                debug!(
                    "Transforming built-in reference '{}' to avoid UnboundLocalError",
                    name.id
                );
                // Use builtins module which is more reliable than __builtins__
                // Generate: __import__('builtins').name
                let import_call = ast_builder::expressions::call(
                    ast_builder::expressions::name("__import__", ExprContext::Load),
                    vec![ast_builder::expressions::string_literal("builtins")],
                    vec![],
                );
                *expr = ast_builder::expressions::attribute(
                    import_call,
                    name.id.as_str(),
                    ExprContext::Load,
                );
            }
        }
        // Recursively handle other expressions
        Expr::Call(call) => {
            transform_expr_for_builtin_shadowing(&mut call.func, builtin_locals);
            for arg in &mut call.arguments.args {
                transform_expr_for_builtin_shadowing(arg, builtin_locals);
            }
            for kw in &mut call.arguments.keywords {
                transform_expr_for_builtin_shadowing(&mut kw.value, builtin_locals);
            }
        }
        Expr::Attribute(attr) => {
            transform_expr_for_builtin_shadowing(&mut attr.value, builtin_locals);
        }
        Expr::Tuple(tuple) => {
            for elem in &mut tuple.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::List(list) => {
            for elem in &mut list.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::BinOp(binop) => {
            transform_expr_for_builtin_shadowing(&mut binop.left, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut binop.right, builtin_locals);
        }
        Expr::UnaryOp(unaryop) => {
            transform_expr_for_builtin_shadowing(&mut unaryop.operand, builtin_locals);
        }
        Expr::If(if_expr) => {
            transform_expr_for_builtin_shadowing(&mut if_expr.test, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut if_expr.body, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut if_expr.orelse, builtin_locals);
        }
        Expr::Lambda(lambda) => {
            // Don't transform inside lambda bodies as they have their own scope
            // Only transform default arguments
            if let Some(ref mut params) = lambda.parameters {
                for arg in &mut params.args {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
                for arg in &mut params.posonlyargs {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
                for arg in &mut params.kwonlyargs {
                    if let Some(ref mut default) = arg.default {
                        transform_expr_for_builtin_shadowing(default, builtin_locals);
                    }
                }
            }
        }
        Expr::Compare(compare) => {
            transform_expr_for_builtin_shadowing(&mut compare.left, builtin_locals);
            for comparator in &mut compare.comparators {
                transform_expr_for_builtin_shadowing(comparator, builtin_locals);
            }
        }
        Expr::Subscript(subscript) => {
            transform_expr_for_builtin_shadowing(&mut subscript.value, builtin_locals);
            transform_expr_for_builtin_shadowing(&mut subscript.slice, builtin_locals);
        }
        Expr::Dict(dict) => {
            for item in &mut dict.items {
                if let Some(ref mut key) = item.key {
                    transform_expr_for_builtin_shadowing(key, builtin_locals);
                }
                transform_expr_for_builtin_shadowing(&mut item.value, builtin_locals);
            }
        }
        Expr::Set(set) => {
            for elem in &mut set.elts {
                transform_expr_for_builtin_shadowing(elem, builtin_locals);
            }
        }
        Expr::ListComp(comp) => {
            // Only transform the iterator - the comprehension body has its own scope
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::SetComp(comp) => {
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::DictComp(comp) => {
            for generator in &mut comp.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::Generator(gen_expr) => {
            for generator in &mut gen_expr.generators {
                transform_expr_for_builtin_shadowing(&mut generator.iter, builtin_locals);
            }
        }
        Expr::BoolOp(boolop) => {
            for value in &mut boolop.values {
                transform_expr_for_builtin_shadowing(value, builtin_locals);
            }
        }
        Expr::Await(await_expr) => {
            transform_expr_for_builtin_shadowing(&mut await_expr.value, builtin_locals);
        }
        Expr::Yield(yield_expr) => {
            if let Some(ref mut value) = yield_expr.value {
                transform_expr_for_builtin_shadowing(value, builtin_locals);
            }
        }
        Expr::YieldFrom(yield_from) => {
            transform_expr_for_builtin_shadowing(&mut yield_from.value, builtin_locals);
        }
        Expr::Starred(starred) => {
            transform_expr_for_builtin_shadowing(&mut starred.value, builtin_locals);
        }
        Expr::Named(named) => {
            transform_expr_for_builtin_shadowing(&mut named.value, builtin_locals);
        }
        Expr::Slice(slice) => {
            if let Some(ref mut lower) = slice.lower {
                transform_expr_for_builtin_shadowing(lower, builtin_locals);
            }
            if let Some(ref mut upper) = slice.upper {
                transform_expr_for_builtin_shadowing(upper, builtin_locals);
            }
            if let Some(ref mut step) = slice.step {
                transform_expr_for_builtin_shadowing(step, builtin_locals);
            }
        }
        Expr::FString(_fstring) => {
            // F-strings are immutable in ruff AST and require special handling
            // We would need to rebuild the entire f-string structure to transform expressions
            // inside interpolations. For now, we skip transforming f-strings as they are less
            // likely to reference shadowed built-ins before assignment.
            // TODO: Implement f-string transformation if needed by reconstructing the f-string
        }
        _ => {
            // Other expression types don't need transformation
        }
    }
}

/// Helper function to determine if a symbol should be included in the module namespace
fn should_include_symbol(
    bundler: &Bundler,
    symbol_name: &str,
    module_name: &str,
    module_scope_symbols: Option<&FxIndexSet<String>>,
) -> bool {
    // If we have module_scope_symbols, check if the symbol is in that set
    // But also check special cases
    if let Some(symbols) = module_scope_symbols {
        if symbols.contains(symbol_name) {
            return true;
        }
        // Even if not in module_scope_symbols, check if it's a private symbol imported by others
        if symbol_name.starts_with('_')
            && let Some(module_asts) = &bundler.module_asts
            && let Some(module_id) = bundler.resolver.get_module_id_by_name(module_name)
            && crate::analyzers::ImportAnalyzer::is_symbol_imported_by_other_modules(
                module_asts,
                module_id,
                symbol_name,
                Some(&bundler.module_exports),
                bundler.resolver,
            )
        {
            log::debug!(
                "Private symbol '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but is imported by other modules, so including it"
            );
            return true;
        }
        // Also include all-caps constants as they're often used internally in comprehensions
        // and other module-level code. Include digits to handle constants like HTTP2, TLS1_3, etc.
        let is_constant_like = symbol_name.len() > 1
            && symbol_name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && symbol_name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if is_constant_like {
            log::debug!(
                "Constant '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but including as it appears to be a constant"
            );
            return true;
        }

        // Include common dunder variables that are often expected to be visible on modules
        const COMMON_DUNDERS: &[&str] = &[
            "__version__",
            "__author__",
            "__license__",
            "__description__",
            "__doc__",
            "__all__",
        ];
        if COMMON_DUNDERS.contains(&symbol_name) {
            log::debug!(
                "Common dunder '{symbol_name}' from module '{module_name}' is not in \
                 module_scope_symbols but including as it's a standard module attribute"
            );
            return true;
        }

        false
    } else {
        // No module_scope_symbols provided, use bundler's should_export_symbol
        bundler.should_export_symbol(symbol_name, module_name)
    }
}

/// Add module attribute assignment if the symbol should be exported
fn add_module_attr_if_exported(
    bundler: &Bundler,
    assign: &StmtAssign,
    module_name: &str,
    body: &mut Vec<Stmt>,
    module_scope_symbols: Option<&FxIndexSet<String>>,
) {
    if let Some(name) = expression_handlers::extract_simple_assign_target(assign) {
        emit_module_attr_if_exportable(
            bundler,
            &name,
            module_name,
            body,
            module_scope_symbols,
            None, // No lifted_names check needed for regular assigns
        );
    }
}

/// Helper to emit module attribute if a symbol should be exported
/// This centralizes the logic for both Assign and `AnnAssign` paths
fn emit_module_attr_if_exportable(
    bundler: &Bundler,
    symbol_name: &str,
    module_name: &str,
    body: &mut Vec<Stmt>,
    module_scope_symbols: Option<&FxIndexSet<String>>,
    lifted_names: Option<&FxIndexMap<String, String>>,
) {
    // Check if this is a lifted variable (only relevant for AnnAssign)
    if let Some(names) = lifted_names
        && names.contains_key(symbol_name)
    {
        debug!("Symbol '{symbol_name}' is a lifted variable, skipping module attribute");
        return;
    }

    let should_export =
        should_include_symbol(bundler, symbol_name, module_name, module_scope_symbols);
    debug!(
        "Symbol '{symbol_name}' in module '{module_name}' should_include_symbol returned: \
         {should_export}"
    );

    if should_export {
        debug!("Adding module attribute for symbol '{symbol_name}'");
        body.push(
            crate::code_generator::module_registry::create_module_attr_assignment(
                SELF_PARAM,
                symbol_name,
            ),
        );
    }
}

/// Create namespace for inlined submodule
fn create_namespace_for_inlined_submodule(
    bundler: &Bundler,
    full_module_name: &str,
    attr_name: &str,
    parent_module_var: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut stmts = Vec::new();

    // Use the sanitized module name for inlined modules to match the global namespace object
    let namespace_var = sanitize_module_name_for_identifier(full_module_name);

    log::debug!(
        "create_namespace_for_inlined_submodule: full_module_name='{full_module_name}', \
         attr_name='{attr_name}', namespace_var='{namespace_var}'"
    );

    // Create a types.SimpleNamespace() for the inlined module
    stmts.push(ast_builder::statements::assign(
        vec![ast_builder::expressions::name(
            &namespace_var,
            ExprContext::Store,
        )],
        ast_builder::expressions::call(
            ast_builder::expressions::simple_namespace_ctor(),
            vec![],
            vec![],
        ),
    ));

    // Get the module ID for this module
    let module_id = bundler
        .resolver
        .get_module_id_by_name(full_module_name)
        .expect("Module should exist in resolver");

    // Get the module exports for this inlined module
    let exported_symbols = bundler.module_exports.get(&module_id).cloned().flatten();

    // Add all exported symbols from the inlined module to the namespace
    if let Some(exports) = exported_symbols {
        for symbol in exports {
            // For re-exported symbols, check if the original symbol is kept by tree-shaking
            let should_include = if bundler.tree_shaking_keep_symbols.is_some() {
                // First check if this symbol is directly defined in this module
                if bundler.is_symbol_kept_by_tree_shaking(module_id, &symbol) {
                    true
                } else {
                    // If not, check if this is a re-exported symbol from another module
                    // For modules with __all__, we always include symbols that are re-exported
                    // even if they're not directly defined in the module
                    let module_has_all_export = bundler
                        .module_exports
                        .get(&module_id)
                        .and_then(|exports| exports.as_ref())
                        .is_some_and(|exports| exports.contains(&symbol));

                    if module_has_all_export {
                        log::debug!(
                            "Including re-exported symbol {symbol} from module {full_module_name} \
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
                log::debug!(
                    "Skipping namespace assignment for {full_module_name}.{symbol} - removed by \
                     tree-shaking"
                );
                continue;
            }

            // Get the renamed version of this symbol
            let renamed_symbol = if let Some(module_id) = bundler.get_module_id(full_module_name)
                && let Some(module_renames) = symbol_renames.get(&module_id)
            {
                module_renames
                    .get(&symbol)
                    .cloned()
                    .unwrap_or_else(|| symbol.clone())
            } else {
                symbol.clone()
            };

            // Before creating the assignment, check if the renamed symbol exists after
            // tree-shaking
            if !renamed_symbol_exists(bundler, &renamed_symbol, symbol_renames) {
                log::warn!(
                    "Skipping namespace assignment {namespace_var}.{symbol} = {renamed_symbol} - \
                     renamed symbol doesn't exist after tree-shaking"
                );
                continue;
            }

            // namespace_var.symbol = renamed_symbol
            log::debug!(
                "Creating namespace assignment: {namespace_var}.{symbol} = {renamed_symbol}"
            );
            stmts.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::name(&namespace_var, ExprContext::Load),
                    &symbol,
                    ExprContext::Store,
                )],
                ast_builder::expressions::name(&renamed_symbol, ExprContext::Load),
            ));
        }
    } else {
        // If no explicit exports, we still need to check if this module defines symbols
        // This is a fallback for modules that don't have __all__ defined
        // For now, log a warning since we can't determine exports without module analysis
        log::warn!(
            "Inlined module '{full_module_name}' has no explicit exports (__all__). Namespace \
             will be empty unless symbols are added elsewhere."
        );
    }

    // Finally, set module.attr_name = namespace_var (e.g., module.compat = pkg_compat)
    // This allows the parent module to access the submodule via the expected attribute name
    stmts.push(
        crate::code_generator::module_registry::create_module_attr_assignment_with_value(
            parent_module_var,
            attr_name,
            &namespace_var,
        ),
    );

    stmts
}

/// Check if a renamed symbol exists after tree-shaking
fn renamed_symbol_exists(
    bundler: &Bundler,
    renamed_symbol: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
) -> bool {
    // If not using tree-shaking, all symbols exist
    if bundler.tree_shaking_keep_symbols.is_none() {
        return true;
    }

    // Check all modules to see if any have this renamed symbol
    for (module_id, renames) in symbol_renames {
        for (original, renamed) in renames {
            if renamed == renamed_symbol {
                // Found the renamed symbol, check if it's kept
                if bundler.is_symbol_kept_by_tree_shaking(*module_id, original) {
                    return true;
                }
            }
        }
    }

    false
}

/// Process wildcard import from an inlined module
/// Returns a list of symbols from wrapper modules that need deferred assignment
fn process_wildcard_import(
    bundler: &Bundler,
    module: &str,
    symbol_renames: &FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    imports_from_inlined: &mut Vec<(String, String, Option<String>)>,
    current_module: &str,
) -> Vec<(String, String)> {
    debug!("Processing wildcard import from inlined module '{module}'");

    // Track symbols from wrapper modules that need deferred handling
    let mut wrapper_module_symbols = Vec::new();

    // Get all exported symbols from this module
    let module_id = bundler.get_module_id(module);
    let exports = module_id.and_then(|id| bundler.module_exports.get(&id));

    if let Some(Some(export_list)) = exports {
        let module_id = module_id.expect("Module ID should exist if exports found");

        // Module has explicit __all__, use it
        // Determine if the importing module accesses __all__ dynamically
        let importer_accesses_all = bundler.get_module_id(current_module).is_some_and(|id| {
            bundler
                .modules_with_accessed_all
                .iter()
                .any(|(mid, _)| *mid == id)
        });

        // Detect if importing module is a package (__init__.py)
        let importer_is_package = bundler
            .get_module_id(current_module)
            .and_then(|id| bundler.resolver.get_module(id))
            .is_some_and(|m| m.is_package);

        for symbol in export_list {
            if symbol != "*" {
                // A symbol is kept if it's kept in the re-exporting module itself,
                // or if it's re-exported from a submodule and kept in that source module.
                let is_kept_final = importer_accesses_all
                    || importer_is_package
                    || bundler.is_symbol_kept_by_tree_shaking(module_id, symbol)
                    || {
                        let mut found_in_submodule = false;
                        for (potential_module_id, module_exports) in &bundler.module_exports {
                            // Check if this is a submodule by comparing names
                            let potential_module_name = bundler
                                .resolver
                                .get_module_name(*potential_module_id)
                                .expect("Module name must exist");
                            if potential_module_name.starts_with(&format!("{module}."))
                                && let Some(exports) = module_exports
                                && exports.contains(symbol)
                                && bundler
                                    .is_symbol_kept_by_tree_shaking(*potential_module_id, symbol)
                            {
                                debug!(
                                    "Symbol '{symbol}' is kept in source module \
                                     '{potential_module_name}'"
                                );
                                found_in_submodule = true;
                                break;
                            }
                        }
                        found_in_submodule
                    };

                if is_kept_final {
                    // Check if this symbol comes from a wrapper module
                    // If it does, we should NOT add it as a module attribute immediately
                    // because the wrapper module hasn't been initialized yet
                    if symbol_comes_from_wrapper_module(bundler, module, symbol) {
                        debug!(
                            "Symbol '{symbol}' from inlined module '{module}' comes from a \
                             wrapper module - deferring assignment"
                        );
                        // Track for deferred assignment after wrapper module initialization
                        let value_name = bundler
                            .get_module_id(module)
                            .and_then(|id| symbol_renames.get(&id))
                            .and_then(|m| m.get(symbol))
                            .cloned()
                            .unwrap_or_else(|| symbol.clone());
                        wrapper_module_symbols.push((symbol.clone(), value_name));
                        continue;
                    }

                    // Get the actual value name (might be renamed to avoid collisions)
                    let value_name = bundler
                        .get_module_id(module)
                        .and_then(|id| symbol_renames.get(&id))
                        .and_then(|m| m.get(symbol))
                        .cloned()
                        .unwrap_or_else(|| symbol.clone());

                    debug!(
                        "Tracking wildcard-imported symbol '{symbol}' (value: '{value_name}') \
                         from inlined module '{module}'"
                    );
                    // Track the source module for proper namespace access
                    imports_from_inlined.push((
                        symbol.clone(),
                        value_name,
                        Some(module.to_string()),
                    ));
                } else {
                    debug!(
                        "Skipping wildcard-imported symbol '{symbol}' from inlined module \
                         '{module}' - removed by tree-shaking"
                    );
                }
            }
        }
        return wrapper_module_symbols;
    }

    if exports.is_some() {
        // Module exists but has no explicit __all__
        // Look at the symbol renames which contains all symbols from the module
        if let Some(module_id) = bundler.get_module_id(module)
            && let Some(renames) = symbol_renames.get(&module_id)
        {
            for (original_name, renamed_name) in renames {
                // Track the renamed symbol (which is what will be in the global scope)
                if !renamed_name.starts_with('_') {
                    // Check if the original symbol was kept by tree-shaking
                    if bundler.is_symbol_kept_by_tree_shaking(module_id, original_name) {
                        // Check if this symbol comes from a wrapper module
                        if symbol_comes_from_wrapper_module(bundler, module, original_name) {
                            debug!(
                                "Symbol '{original_name}' from inlined module '{module}' comes \
                                 from a wrapper module - deferring assignment"
                            );
                            // Track for deferred assignment
                            wrapper_module_symbols
                                .push((original_name.clone(), renamed_name.clone()));
                            continue;
                        }

                        debug!(
                            "Tracking wildcard-imported symbol '{renamed_name}' (renamed from \
                             '{original_name}') from inlined module '{module}'"
                        );
                        // For renamed symbols, use original as exported name, renamed as value
                        // Track the source module for proper namespace access
                        imports_from_inlined.push((
                            original_name.clone(),
                            renamed_name.clone(),
                            Some(module.to_string()),
                        ));
                    } else {
                        debug!(
                            "Skipping wildcard-imported symbol '{renamed_name}' (renamed from \
                             '{original_name}') from inlined module '{module}' - removed by \
                             tree-shaking"
                        );
                    }
                }
            }
            return wrapper_module_symbols;
        }

        // Fallback to semantic exports when no renames are available
        if let Some(module_id) = bundler.get_module_id(module)
            && let Some(semantic) = bundler.semantic_exports.get(&module_id)
        {
            for symbol in semantic {
                if !symbol.starts_with('_') {
                    // Check if the symbol was kept by tree-shaking
                    if bundler.is_symbol_kept_by_tree_shaking(module_id, symbol) {
                        // Check if this symbol comes from a wrapper module
                        if symbol_comes_from_wrapper_module(bundler, module, symbol) {
                            debug!(
                                "Symbol '{symbol}' from inlined module '{module}' comes from a \
                                 wrapper module - deferring assignment"
                            );
                            // Track for deferred assignment
                            wrapper_module_symbols.push((symbol.clone(), symbol.clone()));
                            continue;
                        }

                        debug!(
                            "Tracking wildcard-imported symbol '{symbol}' (from semantic exports) \
                             from inlined module '{module}'"
                        );
                        // No rename, so exported name and value are the same
                        // Track the source module for proper namespace access
                        imports_from_inlined.push((
                            symbol.clone(),
                            symbol.clone(),
                            Some(module.to_string()),
                        ));
                    } else {
                        debug!(
                            "Skipping wildcard-imported symbol '{symbol}' (from semantic exports) \
                             from inlined module '{module}' - removed by tree-shaking"
                        );
                    }
                }
            }
            return wrapper_module_symbols;
        }

        log::warn!(
            "No symbol renames or semantic exports found for inlined module '{module}' — wildcard \
             import may miss symbols"
        );
    } else {
        log::warn!("Could not find exports for inlined module '{module}'");
    }

    wrapper_module_symbols
}

/// Check if a symbol from an inlined module actually comes from a wrapper module
fn symbol_comes_from_wrapper_module(
    bundler: &Bundler,
    inlined_module: &str,
    symbol_name: &str,
) -> bool {
    // Find the module's AST in the module_asts if available
    let module_id = bundler.get_module_id(inlined_module);
    let module_data = module_id.and_then(|id| bundler.module_asts.as_ref()?.get(&id));

    if let Some((ast, module_path, _)) = module_data {
        // Check all import statements in the module
        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Check if this import includes our symbol
                for alias in &import_from.names {
                    let is_wildcard = alias.name.as_str() == "*";
                    let is_direct_import = alias.name.as_str() == symbol_name;

                    if is_wildcard || is_direct_import {
                        // Resolve the module this import is from
                        let resolved_module = if import_from.level > 0 {
                            // Relative import - need to resolve it
                            bundler.resolver.resolve_relative_to_absolute_module_name(
                                import_from.level,
                                import_from
                                    .module
                                    .as_ref()
                                    .map(ruff_python_ast::Identifier::as_str),
                                module_path,
                            )
                        } else {
                            import_from
                                .module
                                .as_ref()
                                .map(std::string::ToString::to_string)
                        };

                        let Some(ref source_module) = resolved_module else {
                            continue;
                        };

                        // Check if the source module is a wrapper module
                        let source_module_id = match bundler.get_module_id(source_module) {
                            Some(id) => id,
                            None => continue,
                        };

                        if !bundler.bundled_modules.contains(&source_module_id)
                            || bundler.inlined_modules.contains(&source_module_id)
                        {
                            continue;
                        }

                        // For wildcard imports, verify the symbol is actually exported
                        if is_wildcard {
                            if let Some(Some(exports)) =
                                bundler.module_exports.get(&source_module_id)
                                && exports.iter().any(|s| s == symbol_name)
                            {
                                debug!(
                                    "Symbol '{symbol_name}' in inlined module '{inlined_module}' \
                                     comes from wrapper module '{source_module}' via wildcard \
                                     import"
                                );
                                return true;
                            }
                        } else {
                            // Direct import - we know this symbol comes from the wrapper module
                            debug!(
                                "Symbol '{symbol_name}' in inlined module '{inlined_module}' \
                                 comes from wrapper module '{source_module}'"
                            );
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}
