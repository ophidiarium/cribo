//! Namespace management utilities for code generation.
//!
//! This module provides functions for creating and managing Python namespace objects
//! that simulate module structures in bundled code.

use std::path::PathBuf;

use log::{debug, warn};
use ruff_python_ast::{Expr, ExprContext, ModModule, Stmt, StmtImportFrom};

use crate::{
    analyzers::symbol_analyzer::SymbolAnalyzer,
    ast_builder::{expressions, statements},
    code_generator::{bundler::Bundler, module_registry::sanitize_module_name_for_identifier},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Information about a registered namespace
#[derive(Debug, Clone)]
pub struct NamespaceInfo {
    /// Parent module that this is an attribute of (e.g., "pkg" for "pkg.compat")
    pub parent_module: Option<String>,
    /// Tracks if the `var = types.SimpleNamespace()` statement has been generated
    pub is_created: bool,
    /// Tracks if the parent attribute assignment has been generated
    pub parent_assignment_done: bool,
    /// The context in which this namespace was required, with priority
    pub context: NamespaceContext,
    /// Symbols that need to be assigned to this namespace after its creation
    pub deferred_symbols: Vec<(String, Expr)>,
}

/// Context in which a namespace is required
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceContext {
    TopLevel,
    Attribute { parent: String },
    ImportedSubmodule,
}

impl NamespaceContext {
    /// Defines the priority for overriding contexts. Higher value wins.
    pub fn priority(&self) -> u8 {
        match self {
            Self::TopLevel => 0,
            Self::Attribute { .. } => 1,
            Self::ImportedSubmodule => 2,
        }
    }
}

/// Context for populating namespace with module symbols.
///
/// This struct encapsulates the state required by the namespace population function,
/// which was previously accessed directly from the `Bundler` struct.
pub struct NamespacePopulationContext<'a> {
    pub inlined_modules: &'a FxIndexSet<ModuleId>,
    pub module_exports: &'a FxIndexMap<ModuleId, Option<Vec<String>>>,
    pub tree_shaking_keep_symbols: &'a Option<FxIndexMap<ModuleId, FxIndexSet<String>>>,
    pub bundled_modules: &'a FxIndexSet<ModuleId>,
    pub modules_with_accessed_all: &'a FxIndexSet<(ModuleId, String)>,
    pub wrapper_modules: &'a FxIndexSet<ModuleId>,
    pub module_asts: &'a Option<Vec<(ModuleId, ModModule, PathBuf, String)>>,
    pub symbols_populated_after_deferred: &'a FxIndexSet<(ModuleId, String)>,
    pub global_deferred_imports: &'a FxIndexMap<(ModuleId, String), String>,
    pub module_init_functions: &'a FxIndexMap<ModuleId, String>,
    pub resolver: &'a crate::resolver::ModuleResolver,
}

impl NamespacePopulationContext<'_> {
    /// Check if a symbol is kept by tree shaking.
    pub fn is_symbol_kept_by_tree_shaking(&self, module_id: ModuleId, symbol_name: &str) -> bool {
        match &self.tree_shaking_keep_symbols {
            Some(kept_symbols) => kept_symbols
                .get(&module_id)
                .is_some_and(|symbols| symbols.contains(symbol_name)),
            None => true, // No tree shaking, all symbols are kept
        }
    }
}

/// Check if a parent module exports a symbol that would conflict with a submodule assignment.
///
/// This determines whether a parent attribute assignment (e.g., `parent.attr = namespace`)
/// should be skipped to avoid clobbering an explicitly exported symbol from the parent module.
///
/// The function checks if:
/// 1. The parent exports a symbol with the same name as the attribute
/// 2. The full module path is NOT a bundled/inlined module (meaning it's a re-exported symbol, not the module itself)
///
/// # Arguments
/// * `bundler` - The bundler containing module export and bundling information
/// * `parent_module` - The parent module to check for exports
/// * `attribute_name` - The attribute name to check for conflicts
/// * `full_module_path` - The full path of the module being assigned (e.g., "package.__version__")
///
/// # Returns
/// `true` if there's an export conflict that should prevent the assignment, `false` otherwise
fn has_export_conflict(
    bundler: &Bundler,
    parent_module: &str,
    attribute_name: &str,
    full_module_path: &str,
) -> bool {
    // First check if the parent exports a symbol with this name
    let parent_exports_symbol = bundler
        .module_exports
        .get(parent_module)
        .and_then(|e| e.as_ref())
        .is_some_and(|exports| exports.contains(&attribute_name.to_string()));

    if !parent_exports_symbol {
        // No export with this name, no conflict
        return false;
    }

    // The parent exports something with this name.
    // Now check if the full module path is an actual module or just a re-exported symbol.
    let is_actual_module = bundler.bundled_modules.contains(full_module_path)
        || bundler.bundled_modules.contains_key(full_module_path)
        || bundler.inlined_modules.contains(full_module_path);

    // Only skip if parent exports the symbol AND it's not an actual submodule
    // (i.e., it's a re-exported symbol from the submodule)
    !is_actual_module
}

/// Create an attribute assignment statement, using namespace variables when available.
///
/// This function creates `parent.attr = value` statements, but intelligently uses
/// namespace variables when they exist. For example, if assigning `services.auth`,
/// it will use the `services_auth` namespace variable if it exists.
pub fn create_attribute_assignment(
    bundler: &Bundler,
    parent: &str,
    attr: &str,
    module_name: &str,
) -> Stmt {
    // Check if there's a namespace variable for the module
    let sanitized_module = sanitize_module_name_for_identifier(module_name);

    let value_expr = if bundler.created_namespaces.contains(&sanitized_module) {
        // Use the namespace variable (e.g., services_auth instead of services.auth)
        debug!("Using namespace variable '{sanitized_module}' for {parent}.{attr} = {module_name}");
        expressions::name(&sanitized_module, ExprContext::Load)
    } else if module_name.contains('.') {
        // Create a dotted expression for the module path
        let parts: Vec<&str> = module_name.split('.').collect();
        expressions::dotted_name(&parts, ExprContext::Load)
    } else {
        // Simple name
        expressions::name(module_name, ExprContext::Load)
    };

    // Create the assignment: parent.attr = value
    statements::assign_attribute(parent, attr, value_expr)
}

/// Generates submodule attributes with exclusions for namespace organization.
///
/// This function analyzes module hierarchies and creates namespace modules and assignments
/// as needed, while handling exclusions and avoiding redundant operations.
///
/// **Note**: This is the complete 310-line implementation moved from bundler.rs to achieve
/// Transform imports from namespace packages.
///
/// This function handles the transformation of imports from namespace packages,
/// creating appropriate assignments and namespace objects as needed.
pub(super) fn transform_namespace_package_imports(
    bundler: &Bundler,
    import_from: StmtImportFrom,
    module_name: &str,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    for alias in &import_from.names {
        let imported_name = alias.name.as_str();
        let local_name = alias.asname.as_ref().unwrap_or(&alias.name).as_str();
        let full_module_path = format!("{module_name}.{imported_name}");

        if bundler.bundled_modules.contains(&full_module_path) {
            if bundler.bundled_modules.contains_key(&full_module_path) {
                // Wrapper module - ensure it's initialized first, then create reference
                // First ensure parent module is initialized if it's also a wrapper
                if bundler.bundled_modules.contains_key(module_name) {
                    result_stmts.extend(
                        crate::code_generator::module_registry::create_module_initialization_for_import(
                            module_name,
                            &bundler.bundled_modules,
                        ),
                    );
                }
                // Initialize the wrapper module if needed
                result_stmts.extend(
                    crate::code_generator::module_registry::create_module_initialization_for_import(
                        &full_module_path,
                        &bundler.bundled_modules,
                    ),
                );

                // Create assignment using dotted name since it's a nested module
                let module_expr =
                    expressions::module_reference(&full_module_path, ExprContext::Load);

                result_stmts.push(statements::simple_assign(local_name, module_expr));
            } else {
                // Inlined module - create a namespace object for it
                debug!(
                    "Submodule '{imported_name}' from namespace package '{module_name}' was \
                     inlined, creating namespace"
                );

                // For namespace hybrid modules, we need to create the namespace object
                // The inlined module's symbols are already renamed with module prefix
                // e.g., message -> message_greetings_greeting
                let _inlined_key = sanitize_module_name_for_identifier(&full_module_path);

                // Create a SimpleNamespace object manually with all the inlined symbols
                // Since the module was inlined, we need to map the original names to the
                // renamed ones
                result_stmts.push(statements::simple_assign(
                    local_name,
                    expressions::call(expressions::simple_namespace_ctor(), vec![], vec![]),
                ));

                // Add all the renamed symbols as attributes to the namespace
                // Get the symbol renames for this module if available
                if let Some(module_renames) = symbol_renames.get(&full_module_path) {
                    let module_suffix = sanitize_module_name_for_identifier(&full_module_path);
                    for (original_name, renamed_name) in module_renames {
                        // Check if this is an identity mapping (no semantic rename)
                        let actual_renamed_name = if renamed_name == original_name {
                            // No semantic rename, apply module suffix pattern

                            get_unique_name_with_module_suffix(original_name, &module_suffix)
                        } else {
                            // Use the semantic rename
                            renamed_name.clone()
                        };

                        // base.original_name = actual_renamed_name
                        result_stmts.push(statements::assign(
                            vec![expressions::attribute(
                                expressions::name(local_name, ExprContext::Load),
                                original_name,
                                ExprContext::Store,
                            )],
                            expressions::name(&actual_renamed_name, ExprContext::Load),
                        ));
                    }
                } else {
                    // Fallback: try to guess the renamed symbols based on module suffix
                    warn!(
                        "No symbol renames found for inlined module '{full_module_path}', \
                         namespace will be empty"
                    );
                }
            }
        } else {
            // Not a bundled submodule, keep as attribute access
            // This might be importing a symbol from the namespace package's __init__.py
            // But since we're here, the namespace package has no __init__.py
            warn!(
                "Import '{imported_name}' from namespace package '{module_name}' is not a bundled \
                 module"
            );
        }
    }

    if result_stmts.is_empty() {
        // If we didn't transform anything, return the original
        vec![Stmt::ImportFrom(import_from)]
    } else {
        result_stmts
    }
}

/// Get a unique name for a symbol, using the module suffix pattern.
///
/// Helper function used by `transform_namespace_package_imports`.
fn get_unique_name_with_module_suffix(base_name: &str, module_name: &str) -> String {
    let module_suffix = sanitize_module_name_for_identifier(module_name);
    format!("{base_name}_{module_suffix}")
}

// NOTE: ensure_namespace_exists was removed as it became obsolete after implementing
// the centralized namespace registry. Its functionality is now handled by:
// - require_namespace() for registration
// - generate_required_namespaces() for generation

/// Parameters for namespace creation
#[derive(Default)]
pub struct NamespaceParams {
    /// Whether to generate the namespace immediately
    pub immediate: bool,
    /// Attributes to set on the namespace after creation (name, value expression)
    pub attributes: Option<Vec<(String, Expr)>>,
}

impl NamespaceParams {
    /// Create params for immediate generation
    pub fn immediate() -> Self {
        Self {
            immediate: true,
            attributes: None,
        }
    }
}

/// Determines the appropriate namespace context for a given path.
/// Returns Attribute context if the path has a parent, otherwise `TopLevel`.
fn determine_namespace_context(path: &str) -> NamespaceContext {
    if let Some((parent, _)) = path.rsplit_once('.') {
        NamespaceContext::Attribute {
            parent: parent.to_string(),
        }
    } else {
        NamespaceContext::TopLevel
    }
}

/// Registers a request for a namespace, creating or updating its info.
/// This is the ONLY function that should be called to request a namespace.
/// It is idempotent and handles parent registration recursively.
///
/// If params.immediate is true, generates and returns the creation statements immediately
/// instead of deferring to `generate_required_namespaces()`.
/// If params.attributes is non-empty, generates attribute assignment statements.
pub fn require_namespace(
    bundler: &mut Bundler,
    path: &str,
    context: NamespaceContext,
    params: NamespaceParams,
) -> Vec<Stmt> {
    // 1. Recursively require parent namespaces if `path` is dotted
    if let Some((parent_path, _)) = path.rsplit_once('.') {
        // Determine the context for the parent using the helper function
        let parent_context = determine_namespace_context(parent_path);
        // Parent namespaces are never immediate - they should be part of centralized generation
        require_namespace(
            bundler,
            parent_path,
            parent_context,
            NamespaceParams::default(),
        );
    }

    // 2. Get or create the sanitized name for `path`
    let sanitized_name = if let Some(existing) = bundler.path_to_sanitized_name.get(path) {
        existing.clone()
    } else {
        let sanitized = sanitize_module_name_for_identifier(path);
        bundler
            .path_to_sanitized_name
            .insert(path.to_string(), sanitized.clone());
        sanitized
    };

    // 3-5. Update or create the NamespaceInfo in the registry
    bundler
        .namespace_registry
        .entry(sanitized_name.clone())
        .and_modify(|info| {
            // Update context only if the new context has higher priority
            if context.priority() > info.context.priority() {
                info.context = context.clone();
            }
        })
        .or_insert_with(|| {
            // Determine parent module (but no aliases here - they're context dependent)
            let parent_module = path.rsplit_once('.').map(|(p, _)| p.to_string());

            NamespaceInfo {
                parent_module,
                is_created: false,
                parent_assignment_done: false,
                context,
                deferred_symbols: Vec::new(),
            }
        });

    // Store deferred attributes if provided and not immediate
    if !params.immediate
        && let Some(attributes) = params.attributes.as_ref()
        && let Some(info) = bundler.namespace_registry.get_mut(&sanitized_name)
    {
        for (attr_name, attr_value) in attributes {
            info.deferred_symbols
                .push((attr_name.clone(), attr_value.clone()));
        }
    }

    if let Some(info) = bundler.namespace_registry.get(&sanitized_name) {
        debug!(
            "Required namespace: {path} -> {sanitized_name} with context {:?}, immediate: {}",
            info.context, params.immediate
        );
    }

    let mut result_stmts = Vec::new();

    // If immediate generation is requested and namespace hasn't been created yet
    if params.immediate {
        debug!(
            "Immediate generation requested for namespace '{sanitized_name}', checking if already \
             created"
        );

        // CRITICAL FIX: Before creating a namespace, ensure its parent exists if needed
        if path.contains('.')
            && let Some((parent_path, _)) = path.rsplit_once('.')
        {
            let parent_sanitized = sanitize_module_name_for_identifier(parent_path);

            // Check if parent namespace exists in registry but hasn't been created yet
            if let Some(parent_info) = bundler.namespace_registry.get(&parent_sanitized)
                && !parent_info.is_created
            {
                debug!(
                    "Parent namespace '{parent_path}' needs to be created before child '{path}'"
                );

                // Recursively create parent namespace with immediate generation
                let parent_context = determine_namespace_context(parent_path);
                let parent_stmts = require_namespace(
                    bundler,
                    parent_path,
                    parent_context,
                    NamespaceParams::immediate(),
                );
                result_stmts.extend(parent_stmts);
            }
        }

        // Note: types module is accessed via _cribo proxy, no explicit import needed

        // Check namespace info and gather necessary data before mutable borrow
        let namespace_info = bundler
            .namespace_registry
            .get(&sanitized_name)
            .map(|info| (info.is_created, info.parent_module.clone()));

        if let Some((is_created, parent_module)) = namespace_info {
            debug!("Namespace '{sanitized_name}' found in registry, is_created: {is_created}");
            if is_created {
                debug!("Namespace '{sanitized_name}' already created, skipping");
            } else {
                // Build keywords for the namespace constructor
                let mut keywords = Vec::new();

                // Always add __name__ as a keyword argument
                keywords.push(expressions::keyword(
                    Some("__name__"),
                    expressions::string_literal(path),
                ));

                // Add any additional attributes as keyword arguments
                if let Some(attributes) = params.attributes {
                    for (attr_name, attr_value) in attributes {
                        keywords.push(expressions::keyword(Some(&attr_name), attr_value));
                    }
                }

                // Store the keyword count for logging before moving
                let keyword_count = keywords.len();

                // Generate the namespace creation statement with keywords
                let creation_stmt = statements::assign(
                    vec![expressions::name(&sanitized_name, ExprContext::Store)],
                    expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
                );
                result_stmts.push(creation_stmt);

                // Mark as created in both the registry and the runtime tracker
                if let Some(info) = bundler.namespace_registry.get_mut(&sanitized_name) {
                    info.is_created = true;
                }
                bundler.created_namespaces.insert(sanitized_name.clone());

                debug!("Generated namespace '{sanitized_name}' with {keyword_count} keywords");

                // CRITICAL: Also generate parent attribute assignment if parent exists
                if let Some(parent_module) = parent_module {
                    let parent_sanitized = sanitize_module_name_for_identifier(&parent_module);

                    // Check if parent is already created
                    if bundler.created_namespaces.contains(&parent_sanitized) {
                        // Extract the attribute name from the path
                        let attr_name = path.rsplit_once('.').map_or(path, |(_, name)| name);

                        // Check if we should skip this assignment.
                        // We skip if the parent exports a symbol with the same name AND it's not an actual submodule.
                        let export_conflict =
                            has_export_conflict(bundler, &parent_module, attr_name, path);

                        if export_conflict {
                            debug!(
                                "Skipping parent attribute assignment for '{parent_sanitized}.{attr_name}' - parent exports same-named symbol"
                            );
                            // Mark as done to avoid later duplication attempts
                            if let Some(info) = bundler.namespace_registry.get_mut(&sanitized_name)
                            {
                                info.parent_assignment_done = true;
                            }
                        } else {
                            debug!(
                                "Generating parent attribute assignment: {parent_sanitized}.{attr_name} = {sanitized_name}"
                            );

                            let parent_assign_stmt = statements::assign_attribute(
                                &parent_sanitized,
                                attr_name,
                                expressions::name(&sanitized_name, ExprContext::Load),
                            );
                            result_stmts.push(parent_assign_stmt);

                            debug!(
                                "Added parent assignment to result_stmts: {parent_sanitized}.{attr_name} = {sanitized_name}"
                            );

                            // Mark parent assignment as done
                            if let Some(info) = bundler.namespace_registry.get_mut(&sanitized_name)
                            {
                                info.parent_assignment_done = true;
                            }
                        }
                    } else {
                        debug!(
                            "Deferring parent attribute assignment for '{sanitized_name}' - parent '{parent_sanitized}' not created yet"
                        );
                    }
                }
            }
        } else {
            debug!("Namespace '{sanitized_name}' not found in registry");
        }
    }

    result_stmts
}

/// Detect namespace requirements from imports of inlined submodules.
/// This pre-registers namespaces that will be needed during import transformation,
/// allowing the centralized system to create them upfront.
pub fn detect_namespace_requirements_from_imports(
    bundler: &mut Bundler,
    modules: &[(String, ModModule, PathBuf, String)],
) {
    use ruff_python_ast::Stmt;

    debug!("Detecting namespace requirements from imports");

    // Scan all modules for `from X import Y` statements
    for (module_name, ast, module_path, _) in modules {
        for stmt in &ast.body {
            if let Stmt::ImportFrom(import_from) = stmt
                && let Some(from_module) = &import_from.module
            {
                let from_module_str = from_module.as_str();

                // Handle relative imports
                let resolved_module = if import_from.level > 0 {
                    bundler.resolver.resolve_relative_to_absolute_module_name(
                        import_from.level,
                        Some(from_module_str),
                        module_path,
                    )
                } else {
                    Some(from_module_str.to_string())
                };

                if let Some(resolved) = resolved_module {
                    // Check each imported name
                    for alias in &import_from.names {
                        let imported_name = alias.name.as_str();
                        let full_module_path = format!("{resolved}.{imported_name}");

                        // Check if this is importing an inlined submodule
                        if bundler.inlined_modules.contains(&full_module_path) {
                            debug!(
                                "Found import of inlined submodule '{full_module_path}' in module \
                                 '{module_name}', pre-registering namespace"
                            );

                            // Register the namespace WITHOUT attributes - those will be added after
                            // inlining The attributes can't be set now
                            // because the symbols don't exist yet
                            let params = NamespaceParams::default();

                            // Register the namespace with the centralized system
                            require_namespace(
                                bundler,
                                &full_module_path,
                                NamespaceContext::ImportedSubmodule,
                                params,
                            );
                        }
                    }
                }
            }
        }
    }

    debug!(
        "Pre-registered {} namespace requirements",
        bundler.namespace_registry.len()
    );
}

/// Create namespace for inlined module.
///
/// Populate a namespace object with all symbols from a given module, applying renames.
///
/// This function generates AST statements to populate a namespace object with symbols
/// from a module, handling tree-shaking, re-exports, and symbol renaming.
pub fn populate_namespace_with_module_symbols(
    ctx: &mut NamespacePopulationContext,
    target_name: &str,
    module_id: ModuleId,
    symbol_renames: &FxIndexMap<String, FxIndexMap<String, String>>,
) -> Vec<Stmt> {
    let mut result_stmts = Vec::new();

    // Get the module name from the resolver
    let Some(module_info) = ctx.resolver.get_module(module_id) else {
        return result_stmts;
    };
    let module_name = &module_info.name;

    // Get the module's exports
    if let Some(exports) = ctx.module_exports.get(&module_id).and_then(|e| e.as_ref()) {
        // Build the namespace access expression for the target
        let parts: Vec<&str> = target_name.split('.').collect();

        // First, add __all__ attribute to the namespace
        // Create the target expression for __all__
        let all_target = expressions::dotted_name(&parts, ExprContext::Load);

        // Filter exports to only include symbols that survived tree-shaking
        let filtered_exports = SymbolAnalyzer::filter_exports_by_tree_shaking(
            exports,
            module_name,
            ctx.tree_shaking_keep_symbols.as_ref(),
            true,
        );

        // Check if __all__ assignment already exists for this namespace
        let all_assignment_exists = result_stmts.iter().any(|stmt| {
            if let Stmt::Assign(assign) = stmt
                && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                && let Expr::Name(base) = attr.value.as_ref()
            {
                return base.id.as_str() == target_name && attr.attr.as_str() == "__all__";
            }
            false
        });

        if all_assignment_exists {
            debug!("Skipping duplicate __all__ assignment for namespace '{target_name}'");
        } else if ctx
            .modules_with_accessed_all
            .iter()
            .any(|(_, alias)| alias == target_name)
        {
            // Only create __all__ assignment if the code actually accesses it
            let all_list = expressions::list(
                filtered_exports
                    .iter()
                    .map(|name| expressions::string_literal(name.as_str()))
                    .collect(),
                ExprContext::Load,
            );

            // Create __all__ assignment statement
            result_stmts.push(statements::assign(
                vec![expressions::attribute(
                    all_target,
                    "__all__",
                    ExprContext::Store,
                )],
                all_list,
            ));

            debug!(
                "Created __all__ assignment for namespace '{target_name}' with exports: \
                 {filtered_exports:?} (accessed in code)"
            );
        } else {
            debug!(
                "Skipping __all__ assignment for namespace '{target_name}' - not accessed in code"
            );
        }

        // For each exported symbol that survived tree-shaking, add it to the namespace
        'symbol_loop: for symbol in &filtered_exports {
            let symbol_name = symbol.as_str();

            // For re-exported symbols, check if the original symbol is kept by tree-shaking
            let should_include = if ctx.tree_shaking_keep_symbols.is_some() {
                // First check if this symbol is directly defined in this module
                if ctx.is_symbol_kept_by_tree_shaking(module_id, symbol_name) {
                    true
                } else {
                    // If not, check if this is a re-exported symbol from another module
                    // For modules with __all__, we always include symbols that are re-exported
                    // even if they're not directly defined in the module
                    let module_has_all_export = ctx
                        .module_exports
                        .get(module_name)
                        .and_then(|exports| exports.as_ref())
                        .is_some_and(|exports| exports.contains(&symbol_name.to_string()));

                    if module_has_all_export {
                        debug!(
                            "Including re-exported symbol {symbol_name} from module {module_name} \
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
                debug!(
                    "Skipping namespace assignment for {module_name}.{symbol_name} - removed by \
                     tree-shaking"
                );
                continue;
            }

            // Check if this symbol is actually a submodule
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let is_bundled_submodule = ctx.bundled_modules.contains(&full_submodule_path);
            let is_inlined = ctx.inlined_modules.contains(&full_submodule_path);
            let uses_init_function = ctx.wrapper_modules.contains(&full_submodule_path);

            if is_bundled_submodule {
                debug!(
                    "Symbol '{symbol_name}' in module '{module_name}' is a submodule (bundled: \
                     {is_bundled_submodule}, inlined: {is_inlined}, uses_init: \
                     {uses_init_function})"
                );

                // For inlined submodules, check if the parent module re-exports a symbol
                // with the same name as the submodule (e.g., __version__ from __version__
                // module)
                if is_inlined {
                    // Check if the submodule has a symbol with the same name as itself
                    if let Some(submodule_exports) = ctx
                        .module_exports
                        .get(&full_submodule_path)
                        .and_then(|e| e.as_ref())
                        && submodule_exports.contains(&symbol_name.to_string())
                    {
                        // The submodule exports a symbol with the same name as itself
                        // Check if the parent module re-exports this symbol
                        debug!(
                            "Submodule '{full_submodule_path}' exports symbol '{symbol_name}' \
                             with same name"
                        );

                        // Get the renamed symbol from the submodule
                        if let Some(submodule_renames) = symbol_renames.get(&full_submodule_path)
                            && let Some(renamed) = submodule_renames.get(symbol_name)
                        {
                            debug!(
                                "Creating namespace assignment: {target_name}.{symbol_name} = \
                                 {renamed} (re-exported from submodule)"
                            );

                            // Create the assignment
                            let target = expressions::dotted_name(&parts, ExprContext::Load);
                            result_stmts.push(statements::assign(
                                vec![expressions::attribute(
                                    target,
                                    symbol_name,
                                    ExprContext::Store,
                                )],
                                expressions::name(renamed, ExprContext::Load),
                            ));
                            continue 'symbol_loop;
                        }
                    }
                }

                // Skip other submodules - they are handled separately
                // This prevents creating invalid assignments like `mypkg.compat = compat`
                // when `compat` is a submodule, not a local variable
                continue;
            }

            // Get the renamed symbol if it exists
            let actual_symbol_name = if let Some(module_renames) = symbol_renames.get(module_name) {
                module_renames
                    .get(symbol_name)
                    .cloned()
                    .unwrap_or_else(|| symbol_name.to_string())
            } else {
                symbol_name.to_string()
            };

            // Create the target expression
            // For simple modules, this will be the module name directly
            // For dotted modules (e.g., greetings.greeting), build the chain
            let target = expressions::dotted_name(&parts, ExprContext::Load);

            // Check if this specific symbol was already populated after deferred imports
            // This happens for modules that had forward references and were populated later
            if ctx
                .symbols_populated_after_deferred
                .contains(&(module_name.to_string(), symbol_name.to_string()))
                && target_name == sanitize_module_name_for_identifier(module_name).as_str()
            {
                debug!(
                    "Skipping symbol assignment {target_name}.{symbol_name} = \
                     {actual_symbol_name} - this specific symbol was already populated after \
                     deferred imports"
                );
                continue;
            }

            // Check if this assignment already exists in result_stmts
            let assignment_exists = result_stmts.iter().any(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && assign.targets.len() == 1
                    && let Expr::Attribute(attr) = &assign.targets[0]
                {
                    // Check if this is the same assignment target
                    if let Expr::Name(base) = attr.value.as_ref() {
                        return base.id.as_str() == target_name
                            && attr.attr.as_str() == symbol_name;
                    }
                }
                false
            });

            if assignment_exists {
                debug!(
                    "[populate_namespace_with_module_symbols_with_renames] Skipping duplicate \
                     namespace assignment: {target_name}.{symbol_name} = {actual_symbol_name} \
                     (assignment already exists)"
                );
                continue;
            }

            // Also check if this is a parent module assignment that might already exist
            // For example, if we're processing mypkg.exceptions and the symbol CustomJSONError
            // is in mypkg's __all__, check if mypkg.CustomJSONError = CustomJSONError already
            // exists
            if module_name.contains('.') {
                let parent_module = module_name
                    .rsplit_once('.')
                    .map_or("", |(parent, _)| parent);
                if !parent_module.is_empty()
                    && let Some(Some(parent_exports)) = ctx.module_exports.get(parent_module)
                    && parent_exports.contains(&symbol_name.to_string())
                {
                    // This symbol is re-exported by the parent module
                    // Check if the parent assignment already exists
                    let parent_assignment_exists = result_stmts.iter().any(|stmt| {
                        if let Stmt::Assign(assign) = stmt
                            && assign.targets.len() == 1
                            && let Expr::Attribute(attr) = &assign.targets[0]
                        {
                            // Check if this is the same assignment
                            if let Expr::Name(base) = attr.value.as_ref() {
                                return base.id.as_str() == parent_module
                                    && attr.attr.as_str() == symbol_name;
                            }
                        }
                        false
                    });

                    if parent_assignment_exists {
                        debug!(
                            "[populate_namespace_with_module_symbols_with_renames/parent] \
                             Skipping duplicate namespace assignment: {target_name}.{symbol_name} \
                             = {actual_symbol_name} (parent assignment already exists in \
                             result_stmts)"
                        );
                        continue;
                    }
                }
            }

            // Check if symbol is a dunder name
            if symbol_name.starts_with("__") && symbol_name.ends_with("__") {
                // For dunder names, check if they're in the __all__ list
                if !exports.contains(&symbol_name.to_string()) {
                    debug!(
                        "Skipping dunder name '{symbol_name}' not in __all__ for module \
                         '{module_name}'"
                    );
                    continue;
                }
            }

            // Also check if this assignment was already made by deferred imports
            // This handles the case where imports create namespace assignments that
            // would be duplicated by __all__ processing
            if !ctx.global_deferred_imports.is_empty() {
                // Check if this symbol was deferred by the same module (intra-module imports)
                let key = (module_name.to_string(), symbol_name.to_string());
                if ctx.global_deferred_imports.contains_key(&key) {
                    debug!(
                        "Skipping namespace assignment for '{symbol_name}' - already created by \
                         deferred import from module '{module_name}'"
                    );
                    continue;
                }
            }

            // For wrapper modules, check if the symbol is imported from an inlined submodule
            // These symbols are already added via module attribute assignments
            if ctx.wrapper_modules.contains(module_name)
                && is_symbol_from_inlined_submodule(ctx, module_name, symbol_name)
            {
                continue 'symbol_loop;
            }

            // Check if this is a submodule that uses an init function
            let full_submodule_path = format!("{module_name}.{symbol_name}");
            let submodule_id = ctx.resolver.get_module_id_by_name(&full_submodule_path);
            let uses_init_function = submodule_id
                .and_then(|id| ctx.module_init_functions.get(&id))
                .is_some();

            if uses_init_function {
                // This is a submodule that uses an init function
                // The assignment will be handled by the init function call
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it uses an \
                     init function"
                );
                continue;
            }

            // Check if this is an inlined submodule (no local variable exists)
            let is_inlined_submodule = ctx.inlined_modules.contains(&full_submodule_path);
            if is_inlined_submodule {
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it's an \
                     inlined submodule"
                );
                continue;
            }

            // Check if this is a submodule at all (vs a symbol defined in the module)
            let is_bundled_submodule = ctx.bundled_modules.contains(&full_submodule_path);
            if is_bundled_submodule {
                // This is a submodule that's bundled but neither inlined nor uses init
                // function This can happen when the submodule is
                // handled differently (e.g., by deferred imports)
                debug!(
                    "Skipping namespace assignment for '{target_name}.{symbol_name}' - it's a \
                     bundled submodule"
                );
                continue;
            }

            // Check if this symbol is re-exported from a wrapper module
            // If so, we need to reference it from that module's namespace
            let symbol_expr = if let Some((source_module, original_name)) =
                find_symbol_source_module(ctx, module_name, symbol_name)
            {
                // Symbol is imported from a wrapper module
                // After the wrapper module's init function runs, the symbol will be available
                // as source_module.original_name (handles aliases correctly)
                debug!(
                    "Creating namespace assignment: {target_name}.{symbol_name} = \
                     {source_module}.{original_name} (re-exported from wrapper module)"
                );

                // Create a reference to the symbol from the source module
                let source_parts: Vec<&str> = source_module.split('.').collect();
                let source_expr = expressions::dotted_name(&source_parts, ExprContext::Load);
                expressions::attribute(source_expr, &original_name, ExprContext::Load)
            } else {
                // Symbol is defined in this module or renamed
                debug!(
                    "Creating namespace assignment: {target_name}.{symbol_name} = \
                     {actual_symbol_name} (local symbol)"
                );
                expressions::name(&actual_symbol_name, ExprContext::Load)
            };

            // Now add the symbol as an attribute
            result_stmts.push(statements::assign(
                vec![expressions::attribute(
                    target,
                    symbol_name,
                    ExprContext::Store,
                )],
                symbol_expr,
            ));
        }
    }

    result_stmts
}

/// Check if a symbol in a wrapper module is imported from an inlined submodule.
///
/// This helper function reduces nesting in `populate_namespace_with_module_symbols`
/// by extracting the logic for checking if a symbol is already handled via module
/// attribute assignments.
fn is_symbol_from_inlined_submodule(
    ctx: &NamespacePopulationContext,
    module_name: &str,
    symbol_name: &str,
) -> bool {
    debug!(
        "Module '{module_name}' is a wrapper module, checking if symbol '{symbol_name}' is \
         imported from inlined submodule"
    );

    let Some(module_asts) = ctx.module_asts.as_ref() else {
        return false;
    };

    // Find the module's AST to check its imports
    let Some((_, ast, module_path, _)) = module_asts
        .iter()
        .find(|(name, _, _, _)| name == module_name)
    else {
        return false;
    };

    // Check if this symbol is imported from an inlined submodule
    for stmt in &ast.body {
        let Stmt::ImportFrom(import_from) = stmt else {
            continue;
        };

        let resolved_module = crate::code_generator::symbol_source::resolve_import_module(
            ctx.resolver,
            import_from,
            module_path,
        );

        if let Some(ref resolved) = resolved_module {
            // Check if the resolved module is inlined
            if ctx.inlined_modules.contains(resolved) {
                // Check if our symbol is in this import
                for alias in &import_from.names {
                    if alias.name.as_str() == symbol_name {
                        debug!(
                            "Skipping namespace assignment for '{symbol_name}' - already imported \
                             from inlined module '{resolved}' and added as module attribute"
                        );
                        // Skip this symbol - it's already added via module attributes
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Find the source module and original name for a re-exported symbol.
///
/// This helper function checks if a symbol is imported from another module
/// and returns the source module name and original symbol name if it's a wrapper module.
/// This handles import aliases correctly (e.g., `from .base import YAMLObject as YO`).
fn find_symbol_source_module(
    ctx: &NamespacePopulationContext,
    module_name: &str,
    symbol_name: &str,
) -> Option<(String, String)> {
    let module_asts = ctx.module_asts.as_ref()?;

    crate::code_generator::symbol_source::find_symbol_source_from_wrapper_module(
        module_asts,
        ctx.resolver,
        ctx.module_info_registry,
        module_name,
        symbol_name,
    )
}
