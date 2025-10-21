//! Import analysis phase for init function transformation
//!
//! This phase analyzes all imports in the module and populates tracking state
//! without modifying the AST.

use log::debug;
use ruff_python_ast::{Identifier, ModModule, Stmt};

use super::state::InitFunctionState;
use crate::{
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
    resolver::ModuleId,
    types::FxIndexMap,
};

/// Phase responsible for analyzing imports in the module
pub(crate) struct ImportAnalysisPhase;

impl ImportAnalysisPhase {
    /// Analyze all imports in the module and populate tracking state
    ///
    /// This phase:
    /// 1. Loops through all import statements in the AST
    /// 2. Collects imported symbols to avoid name collisions
    /// 3. Resolves relative imports to absolute module names
    /// 4. Tracks stdlib imports that need re-export
    /// 5. Identifies symbols from inlined modules
    /// 6. Processes wildcard imports to track all imported symbols
    ///
    /// Note: This phase only analyzes imports - it doesn't transform the AST.
    /// The actual AST transformation happens in the Import Transformation phase.
    pub(crate) fn execute(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        ast: &ModModule,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) {
        // Track imports from inlined modules before transformation
        // - imports_from_inlined: symbols that exist in global scope (primarily for wildcard
        //   imports) Format: (exported_name, value_name, source_module)
        // - inlined_import_bindings: local binding names created by explicit from-imports (asname
        //   if present)
        // - wrapper_module_symbols_global_only: Track wrapper module symbols that need placeholders
        //   (symbol_name, value_name)

        // Track ALL imported symbols to avoid overwriting them with submodule namespaces
        // Track stdlib symbols that need to be added to the module namespace
        // Use a stable set to dedup and preserve insertion order

        // Note: Do not reorder statements in wrapper modules. Some libraries (e.g., httpx)
        // define constants used by function default arguments; hoisting functions would
        // break evaluation order of those defaults.

        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt {
                // Collect ALL imported symbols (not just from inlined modules)
                for alias in &import_from.names {
                    let imported_name = alias.name.as_str();
                    if imported_name != "*" {
                        // Use the local binding name (asname if present, otherwise the imported
                        // name)
                        let local_name = alias
                            .asname
                            .as_ref()
                            .map_or(imported_name, Identifier::as_str);
                        state.imported_symbols.insert(local_name.to_owned());
                        debug!(
                            "Collected imported symbol '{}' in module '{}'",
                            local_name, ctx.module_name
                        );
                    }
                    // Note: Wildcard imports aren't expanded here to avoid false positives.
                    // Resolution is handled elsewhere (module export analysis); we intentionally
                    // skip adding names from '*' here.
                }

                // Resolve the module to check if it's inlined
                let resolved_module = if import_from.level > 0 {
                    bundler.resolver.resolve_relative_to_absolute_module_name(
                        import_from.level,
                        import_from.module.as_ref().map(Identifier::as_str),
                        ctx.module_path,
                    )
                } else {
                    import_from.module.as_ref().map(ToString::to_string)
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
                        Self::process_stdlib_import_reexports(
                            bundler,
                            ctx,
                            import_from,
                            module,
                            state,
                        );
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
                            // For wildcard imports, we need to track all symbols that will be
                            // imported
                            if imported_name == "*" {
                                let wrapper_symbols = Self::process_wildcard_import(
                                    bundler,
                                    module,
                                    symbol_renames,
                                    &mut state.imports_from_inlined,
                                    ctx.module_name,
                                );

                                // Collect wrapper module symbols that need special handling
                                // We need to track them separately to create placeholder
                                // assignments
                                Self::collect_wrapper_symbols(wrapper_symbols, state);
                            } else {
                                let local_binding_name = alias
                                    .asname
                                    .as_ref()
                                    .map_or(imported_name, Identifier::as_str);
                                debug!(
                                    "Tracking imported name '{imported_name}' as local binding \
                                     '{local_binding_name}' from inlined module '{module}'"
                                );
                                state
                                    .inlined_import_bindings
                                    .push(local_binding_name.to_owned());
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
                    let local_name = alias.asname.as_ref().map_or_else(
                        || {
                            let full = alias.name.as_str();
                            full.split('.').next().unwrap_or(full)
                        },
                        Identifier::as_str,
                    );
                    state.imported_symbols.insert(local_name.to_owned());
                    debug!(
                        "Collected imported symbol '{}' via 'import' in module '{}'",
                        local_name, ctx.module_name
                    );
                }
            }
        }
    }

    /// Process stdlib import for re-exports
    fn process_stdlib_import_reexports(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        import_from: &ruff_python_ast::StmtImportFrom,
        module: &str,
        state: &mut InitFunctionState,
    ) {
        for alias in &import_from.names {
            let imported_name = alias.name.as_str();
            if imported_name != "*" {
                let local_name = alias
                    .asname
                    .as_ref()
                    .map_or(imported_name, Identifier::as_str);

                // Check if this symbol should be re-exported (in __all__ or no __all__)
                let should_reexport = if let Some(Some(export_list)) = bundler
                    .get_module_id(ctx.module_name)
                    .and_then(|id| bundler.module_exports.get(&id))
                {
                    export_list.contains(&local_name.to_owned())
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
                    state
                        .stdlib_reexports
                        .insert((local_name.to_owned(), proxy_path));
                }
            }
        }
    }

    /// Collect wrapper module symbols and add them to state
    /// This reduces nesting by extracting the collection logic into a helper
    fn collect_wrapper_symbols(
        wrapper_symbols: Vec<(String, String)>,
        state: &mut InitFunctionState,
    ) {
        for (symbol_name, value_name) in wrapper_symbols {
            debug!("Collecting wrapper module symbol '{symbol_name}' for special handling");
            state
                .wrapper_module_symbols_global_only
                .push((symbol_name, value_name));
        }
    }

    /// Process wildcard import from an inlined module
    /// Returns a list of symbols from wrapper modules that need deferred assignment
    fn process_wildcard_import(
        bundler: &Bundler<'_>,
        module: &str,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        imports_from_inlined: &mut Vec<(String, String, Option<String>)>,
        current_module: &str,
    ) -> Vec<(String, String)> {
        // Delegates to module_transformer::process_wildcard_import for complex wildcard handling
        super::super::module_transformer::process_wildcard_import(
            bundler,
            module,
            symbol_renames,
            imports_from_inlined,
            current_module,
        )
    }
}
