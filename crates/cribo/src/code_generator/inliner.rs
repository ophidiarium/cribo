//! Module inlining functionality for the bundler
//!
//! This module handles the inlining of Python modules into the final bundle,
//! including class, assignment, and annotation inlining.

use std::path::Path;

use log::debug;
use ruff_python_ast::{Expr, Identifier, ModModule, Stmt, StmtAssign, StmtClassDef};
use ruff_text_size::TextRange;

use super::{
    bundler::Bundler,
    context::InlineContext,
    expression_handlers, import_deduplicator,
    import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
    module_registry::{INIT_RESULT_VAR, generate_unique_name},
};
use crate::{ast_builder::statements, types::FxIndexMap};

impl Bundler<'_> {
    /// Resolve the renamed name for a symbol, considering semantic renames and conflicts
    fn resolve_renamed_name(
        &self,
        original_name: &str,
        module_name: &str,
        ctx: &InlineContext,
    ) -> String {
        // Check if there's a semantic rename that's different from the original
        let module_id = self
            .resolver
            .get_module_id_by_name(module_name)
            .expect("Module should exist");
        if let Some(new_name) = ctx
            .module_renames
            .get(&module_id)
            .and_then(|renames| renames.get(original_name))
            .filter(|&name| name != original_name)
        {
            debug!(
                "Using semantic rename for '{original_name}' to '{new_name}' in module \
                 '{module_name}'"
            );
            return new_name.clone();
        }

        // No semantic rename, or semantic rename is the same as the original.
        // Check for conflict.
        if ctx.global_symbols.contains(original_name) {
            let base_name = self.get_unique_name_with_module_suffix(original_name, module_name);
            generate_unique_name(&base_name, ctx.global_symbols)
        } else {
            original_name.to_string()
        }
    }

    /// Inline a module
    pub(crate) fn inline_module(
        &mut self,
        module_name: &str,
        mut ast: ModModule,
        _module_path: &Path,
        ctx: &mut InlineContext,
    ) {
        let module_id = self
            .resolver
            .get_module_id_by_name(module_name)
            .expect("Module should exist");
        let mut module_renames = FxIndexMap::default();

        // Then apply recursive import transformation to the module
        let mut transformer = RecursiveImportTransformer::new(&RecursiveImportTransformerParams {
            bundler: self,
            module_id,
            symbol_renames: ctx.module_renames,
            is_wrapper_init: false, // Not a wrapper init
            global_deferred_imports: Some(&self.global_deferred_imports), // Pass global registry
            python_version: ctx.python_version,
        });
        transformer.transform_module(&mut ast);

        // Copy import aliases from the transformer to the inline context
        ctx.import_aliases = transformer.import_aliases;

        // Reorder statements to ensure proper declaration order
        let statements = if self.circular_modules.contains(&module_id) {
            log::debug!("Module '{module_name}' is circular, applying reordering");
            self.reorder_statements_for_circular_module(module_name, ast.body, ctx.python_version)
        } else {
            // For non-circular modules, only reorder if there are actual issues that require it
            // Simple modules should be inlined as-is without reordering
            log::debug!(
                "Module '{module_name}' is not circular, preserving original statement order"
            );
            ast.body
        };

        // Build a map of imported symbols to their source modules
        ctx.import_sources = self.build_import_source_map(&statements, module_name);

        // Process each statement in the module
        log::debug!(
            "Processing {} statements for module '{}'",
            statements.len(),
            module_name
        );
        for (idx, stmt) in statements.iter().enumerate() {
            let stmt_desc = match stmt {
                Stmt::Import(_) => "Import".to_string(),
                Stmt::ImportFrom(_) => "ImportFrom".to_string(),
                Stmt::Assign(a) if a.targets.len() == 1 => match &a.targets[0] {
                    Expr::Name(n) => format!("Assign(Name({}))", n.id.as_str()),
                    Expr::Attribute(attr) => {
                        if let Expr::Name(b) = attr.value.as_ref() {
                            format!(
                                "Assign(Attribute({}.{}))",
                                b.id.as_str(),
                                attr.attr.as_str()
                            )
                        } else {
                            "Assign(Attribute(complex))".to_string()
                        }
                    }
                    _ => "Assign(Other)".to_string(),
                },
                Stmt::FunctionDef(f) => format!("FunctionDef({})", f.name.as_str()),
                Stmt::ClassDef(c) => format!("ClassDef({})", c.name.as_str()),
                _ => "Other".to_string(),
            };
            log::debug!("Processing statement {idx} in '{module_name}': {stmt_desc}");
            match &stmt {
                Stmt::Import(import_stmt) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !import_deduplicator::is_hoisted_import(self, stmt) {
                        log::debug!(
                            "Including non-hoisted import in inlined module '{}': {:?}",
                            module_name,
                            import_stmt
                                .names
                                .iter()
                                .map(|a| (
                                    a.name.as_str(),
                                    a.asname.as_ref().map(ruff_python_ast::Identifier::as_str)
                                ))
                                .collect::<Vec<_>>()
                        );
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::ImportFrom(_) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !import_deduplicator::is_hoisted_import(self, stmt) {
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_id, ctx.module_exports_map) {
                        continue;
                    }

                    // Check if this symbol was renamed by semantic analysis
                    let renamed_name = self.resolve_renamed_name(&func_name, module_name, ctx);

                    // Always track the symbol mapping, even if not renamed
                    module_renames.insert(func_name.clone(), renamed_name.clone());
                    ctx.global_symbols.insert(renamed_name.clone());

                    // Clone and rename the function
                    let mut func_def_clone = func_def.clone();
                    func_def_clone.name = Identifier::new(renamed_name, TextRange::default());

                    // Apply renames to function annotations (parameters and return type)
                    if let Some(ref mut returns) = func_def_clone.returns {
                        expression_handlers::resolve_import_aliases_in_expr(
                            returns,
                            &ctx.import_aliases,
                        );
                        expression_handlers::rewrite_aliases_in_expr(returns, &module_renames);
                    }

                    // Apply renames to parameter annotations
                    for param in &mut func_def_clone.parameters.args {
                        if let Some(ref mut annotation) = param.parameter.annotation {
                            expression_handlers::resolve_import_aliases_in_expr(
                                annotation,
                                &ctx.import_aliases,
                            );
                            expression_handlers::rewrite_aliases_in_expr(
                                annotation,
                                &module_renames,
                            );
                        }
                    }

                    // First resolve import aliases in function body
                    for body_stmt in &mut func_def_clone.body {
                        Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
                    }

                    // Create a temporary statement to rewrite the entire function properly
                    let mut temp_stmt = Stmt::FunctionDef(func_def_clone);

                    // Apply renames to the entire function (this will handle global statements
                    // correctly)
                    expression_handlers::rewrite_aliases_in_stmt(&mut temp_stmt, &module_renames);

                    // Also apply semantic renames from context
                    if let Some(semantic_renames) = ctx.module_renames.get(&module_id) {
                        expression_handlers::rewrite_aliases_in_stmt(
                            &mut temp_stmt,
                            semantic_renames,
                        );
                    }

                    ctx.inlined_stmts.push(temp_stmt);
                }
                Stmt::ClassDef(class_def) => {
                    self.inline_class(class_def, module_name, module_id, &mut module_renames, ctx);
                }
                Stmt::Assign(assign) => {
                    // Log what we're processing
                    if assign.targets.len() == 1 {
                        match &assign.targets[0] {
                            Expr::Name(name) => {
                                log::debug!(
                                    "Processing simple assignment in '{}': {} = ...",
                                    module_name,
                                    name.id.as_str()
                                );
                            }
                            Expr::Attribute(attr) => {
                                if let Expr::Name(base) = attr.value.as_ref() {
                                    log::debug!(
                                        "Processing attribute assignment in '{}': {}.{} = ...",
                                        module_name,
                                        base.id.as_str(),
                                        attr.attr.as_str()
                                    );
                                }
                            }
                            _ => {
                                log::debug!(
                                    "Processing other assignment in '{}': {:?}",
                                    module_name,
                                    assign.targets[0]
                                );
                            }
                        }
                    }
                    self.inline_assignment(assign, module_name, &mut module_renames, ctx);
                }
                Stmt::AnnAssign(ann_assign) => {
                    self.inline_ann_assignment(
                        ann_assign,
                        module_name,
                        module_id,
                        &mut module_renames,
                        ctx,
                    );
                }
                // TypeAlias statements are safe metadata definitions
                Stmt::TypeAlias(_) => {
                    // Type aliases don't need renaming in Python, they're just metadata
                    ctx.inlined_stmts.push(stmt.clone());
                }
                // Pass statements are no-ops and safe
                Stmt::Pass(_) => {
                    // Pass statements can be included as-is
                    ctx.inlined_stmts.push(stmt.clone());
                }
                // Expression statements that are string literals are docstrings
                Stmt::Expr(expr_stmt) => {
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        // This is a docstring - safe to include
                        ctx.inlined_stmts.push(stmt.clone());
                    } else {
                        // Other expression statements shouldn't exist in side-effect-free modules
                        log::warn!(
                            "Unexpected expression statement in side-effect-free module \
                             '{module_name}': {stmt:?}"
                        );
                    }
                }
                Stmt::For(for_stmt) => {
                    // Check if this is a deferred import pattern (iterating over INIT_RESULT_VAR)
                    if let Expr::Call(call) = &*for_stmt.iter
                        && let Expr::Name(func_name) = &*call.func
                        && func_name.id.as_str() == "dir"
                        && call.arguments.args.len() == 1
                        && let Expr::Name(arg_name) = &call.arguments.args[0]
                        && arg_name.id.as_str() == INIT_RESULT_VAR
                    {
                        // This is a pattern for copying attributes
                        // Skip it silently as it will be handled separately
                        log::debug!("Skipping deferred import For loop in module '{module_name}'");
                    } else {
                        // Other For loops shouldn't exist in side-effect-free modules
                        log::warn!(
                            "Unexpected For loop in side-effect-free module '{module_name}': \
                             {for_stmt:?}"
                        );
                    }
                }
                _ => {
                    // Any other statement type that we haven't explicitly handled
                    log::warn!(
                        "Unexpected statement type in side-effect-free module '{module_name}': \
                         {stmt:?}"
                    );
                }
            }
        }

        // Store the renames for this module
        if !module_renames.is_empty() {
            ctx.module_renames.insert(module_id, module_renames);
        }

        // Statements are accumulated in ctx.inlined_stmts
    }

    /// Rewrite a class argument expression (base class or keyword value)
    /// applying appropriate renames based on import sources and module context
    fn rewrite_class_arg_expr(
        &self,
        expr: &mut Expr,
        ctx: &InlineContext,
        module_renames: &FxIndexMap<String, String>,
        arg_kind: &str,
    ) {
        if let Expr::Name(name_expr) = expr {
            let name = name_expr.id.as_str();

            // Check if this value was imported from another module.
            // If it was imported under an alias (e.g. `from pkg import X as Y`),
            // resolve the canonical symbol via ctx.import_aliases and use its last
            // segment to query the source module's renames.
            if let Some(source_module) = ctx.import_sources.get(name) {
                let lookup_key = if let Some(canonical) = ctx.import_aliases.get(name) {
                    canonical.rsplit('.').next().unwrap_or(canonical.as_str())
                } else {
                    name
                };

                // Use that module's renames instead of the current module's
                let source_module_id = self
                    .get_module_id(source_module)
                    .expect("Source module should exist");
                if let Some(source_renames) = ctx.module_renames.get(&source_module_id)
                    && let Some(renamed) = source_renames.get(lookup_key)
                {
                    log::debug!(
                        "Applying cross-module rename for {arg_kind} '{name}' from module \
                         '{source_module}': '{lookup_key}' -> '{renamed}'"
                    );
                    name_expr.id = renamed.clone().into();
                    return;
                }
            }

            // Not imported or no rename found in source module, apply local renames
            if let Some(renamed) = module_renames.get(name) {
                name_expr.id = renamed.clone().into();
            }
        } else {
            // Complex expression: first resolve import aliases, then apply renames
            expression_handlers::resolve_import_aliases_in_expr(expr, &ctx.import_aliases);
            expression_handlers::rewrite_aliases_in_expr(expr, module_renames);
        }
    }

    /// Inline a class definition
    pub(crate) fn inline_class(
        &mut self,
        class_def: &StmtClassDef,
        module_name: &str,
        module_id: crate::resolver::ModuleId,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let class_name = class_def.name.to_string();
        if !self.should_inline_symbol(&class_name, module_id, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = self.resolve_renamed_name(&class_name, module_name, ctx);

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(class_name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the class
        let mut class_def_clone = class_def.clone();
        class_def_clone.name = Identifier::new(renamed_name.clone(), TextRange::default());

        // Apply renames to base classes and keyword arguments
        // CRITICAL: For cross-module inheritance, we need to apply renames from the
        // source module of each base class, not just from the current module.
        if let Some(ref mut arguments) = class_def_clone.arguments {
            // Apply renames to base classes
            for arg in &mut arguments.args {
                self.rewrite_class_arg_expr(arg, ctx, module_renames, "base class");
            }

            // Also apply renames to keyword arguments (e.g., metaclass=SomeMetaclass)
            for keyword in &mut arguments.keywords {
                // For metaclass keyword arguments, we need to handle forward references
                // to classes in the same module that haven't been processed yet
                if let Some(ident) = &keyword.arg
                    && ident.as_str() == "metaclass"
                    && let Expr::Name(name_expr) = &mut keyword.value
                {
                    let metaclass_name = name_expr.id.as_str();
                    // Check if this metaclass is from the same module and has a semantic rename
                    if !ctx.import_sources.contains_key(metaclass_name) {
                        // Not imported, so it's from the current module
                        // Use resolve_renamed_name to get the pre-computed semantic rename
                        let resolved_name =
                            self.resolve_renamed_name(metaclass_name, module_name, ctx);
                        log::debug!(
                            "Metaclass '{metaclass_name}' in module '{module_name}' resolves to \
                             '{resolved_name}'"
                        );
                        if resolved_name != metaclass_name {
                            log::debug!(
                                "Applying semantic rename for metaclass '{metaclass_name}' -> \
                                 '{resolved_name}' in module '{module_name}'"
                            );
                            name_expr.id = resolved_name.into();
                            continue;
                        }
                    }
                }

                self.rewrite_class_arg_expr(
                    &mut keyword.value,
                    ctx,
                    module_renames,
                    "keyword value",
                );
            }
        }

        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
            expression_handlers::rewrite_aliases_in_stmt(body_stmt, module_renames);
            // Also apply semantic renames from context
            if let Some(semantic_renames) = ctx.module_renames.get(&module_id) {
                expression_handlers::rewrite_aliases_in_stmt(body_stmt, semantic_renames);
            }
        }

        ctx.inlined_stmts.push(Stmt::ClassDef(class_def_clone));

        // Set the __module__ attribute to preserve the original module name
        ctx.inlined_stmts.push(statements::set_string_attribute(
            &renamed_name,
            "__module__",
            module_name,
        ));

        // If the class was renamed, also set __name__ to preserve the original class name
        if renamed_name != class_name {
            ctx.inlined_stmts.push(statements::set_string_attribute(
                &renamed_name,
                "__name__",
                &class_name,
            ));

            // Set __qualname__ to match __name__ for proper repr()
            ctx.inlined_stmts.push(statements::set_string_attribute(
                &renamed_name,
                "__qualname__",
                &class_name,
            ));
        }
    }

    /// Inline an assignment statement
    pub(crate) fn inline_assignment(
        &mut self,
        assign: &StmtAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        // Check if this is a module initialization assignment (e.g., requests.sessions =
        // _cribo_init_...) These are generated by our import transformation and must be
        // preserved
        if !assign.targets.is_empty()
            && let Expr::Attribute(attr) = &assign.targets[0]
            && let Expr::Call(call) = &*assign.value
            && let Expr::Name(func_name) = &*call.func
            && func_name.id.starts_with("_cribo_init_")
        {
            // This is a module initialization - preserve it as-is
            log::debug!(
                "Preserving module initialization assignment in '{}': {}.{} = {}()",
                module_name,
                if let Expr::Name(base) = &*attr.value {
                    base.id.as_str()
                } else {
                    "?"
                },
                attr.attr.as_str(),
                func_name.id.as_str()
            );
            ctx.inlined_stmts.push(Stmt::Assign(assign.clone()));
            return;
        }

        // Also check if this is a wrapper module init call (e.g., compat = compat.__init__(compat))
        // These are generated by our import transformation and must be preserved
        if !assign.targets.is_empty()
            && let Expr::Name(target_name) = &assign.targets[0]
            && let Expr::Call(call) = &*assign.value
            && let Expr::Attribute(attr) = &*call.func
            && let Expr::Name(base_name) = &*attr.value
            && attr.attr.as_str() == crate::python::constants::INIT_STEM
            && base_name.id == target_name.id
        {
            // This is a wrapper module initialization - preserve it as-is
            log::debug!(
                "Preserving wrapper module initialization in '{}': {} = {}.__init__({})",
                module_name,
                target_name.id.as_str(),
                base_name.id.as_str(),
                base_name.id.as_str()
            );
            ctx.inlined_stmts.push(Stmt::Assign(assign.clone()));
            return;
        }

        let Some(name) = expression_handlers::extract_simple_assign_target(assign) else {
            log::debug!(
                "Skipping non-simple assignment in '{module_name}' - target is not a simple name",
            );
            return;
        };

        // Special handling for circular modules: include private module-level variables
        // that may be used by public functions
        let module_id = self
            .resolver
            .get_module_id_by_name(module_name)
            .expect("Module should exist");
        let is_circular_module = self.circular_modules.contains(&module_id);
        let is_single_underscore_private = name.starts_with('_') && !name.starts_with("__");

        // Check if this is an import alias assignment created by import transformation
        // These are assignments where the RHS is either:
        // 1. A name that references a namespace module (e.g., greetings_messages)
        // 2. An attribute access to a namespace module (e.g., core_utils_helpers.process)
        let is_import_alias = match assign.value.as_ref() {
            Expr::Name(name_expr) => {
                let rhs_name = name_expr.id.as_str();
                // Check if the RHS is a sanitized module name (e.g., greetings_messages)
                self.bundled_modules.iter().any(|bundled_id| {
                    if let Some(bundled_name) = self.resolver.get_module_name(*bundled_id) {
                        let sanitized =
                            crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                                &bundled_name,
                            );
                        sanitized == rhs_name
                    } else {
                        false
                    }
                })
            }
            Expr::Attribute(attr_expr) => {
                // Check if the base is a namespace module (e.g., core_utils_helpers in
                // core_utils_helpers.process)
                if let Expr::Name(base_name) = attr_expr.value.as_ref() {
                    let base = base_name.id.as_str();
                    // Check if the base is a sanitized module name
                    self.bundled_modules.iter().any(|bundled_id| {
                        if let Some(bundled_name) = self.resolver.get_module_name(*bundled_id) {
                            let sanitized =
                                crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                                    &bundled_name,
                                );
                            sanitized == base
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            }
            _ => false,
        };

        if is_import_alias {
            log::debug!("Including import alias assignment '{name}' in module '{module_name}'");
            // Don't skip import aliases - they're created by transformation and should always be
            // included
        } else if is_circular_module && is_single_underscore_private {
            // For circular modules, we always include single-underscore private module-level
            // variables because they might be used by functions that are part of the
            // circular dependency
            log::debug!("Including private variable '{name}' from circular module '{module_name}'");
        } else if !self.should_inline_symbol(&name, module_id, ctx.module_exports_map) {
            // For all other cases, use the standard inlining check
            log::debug!(
                "Not inlining symbol '{name}' from module '{module_name}' - failed \
                 should_inline_symbol check"
            );
            return;
        }

        // Clone the assignment first
        let mut assign_clone = assign.clone();

        // Check if this is a self-referential assignment
        let is_self_referential =
            expression_handlers::is_self_referential_assignment(assign, ctx.python_version);

        // Skip self-referential assignments entirely - they're meaningless
        if is_self_referential {
            log::debug!("Skipping self-referential assignment '{name}' in module '{module_name}'");
            // Still need to track the rename for the symbol so namespace creation works
            // But we should check if there's already a rename for this symbol
            // (e.g., from a function or class definition)
            if !module_renames.contains_key(&name) {
                // Only create a rename if we haven't seen this symbol yet
                let renamed_name = self.resolve_renamed_name(&name, module_name, ctx);
                module_renames.insert(name.clone(), renamed_name.clone());
                ctx.global_symbols.insert(renamed_name);
            }
            return;
        }

        // Apply existing renames to the RHS value BEFORE creating new rename for LHS
        expression_handlers::resolve_import_aliases_in_expr(
            &mut assign_clone.value,
            &ctx.import_aliases,
        );
        expression_handlers::rewrite_aliases_in_expr(&mut assign_clone.value, module_renames);

        // Now create a new rename for the LHS
        // Check if this symbol was renamed by semantic analysis
        let renamed_name = self.resolve_renamed_name(&name, module_name, ctx);

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(name.clone(), renamed_name.clone());
        ctx.global_symbols.insert(renamed_name.clone());

        // Apply the rename to the LHS
        if let Expr::Name(name_expr) = &mut assign_clone.targets[0] {
            name_expr.id = renamed_name.clone().into();
        }

        // Check if this assignment references a module that will be created as a namespace
        // If it does, we need to defer it until after namespace creation
        if self.assignment_references_namespace_module(&assign_clone, module_name, ctx) {
            log::debug!(
                "Assignment '{name}' in module '{module_name}' references a namespace module"
            );
            // Note: deferred imports functionality has been removed
            // This assignment was previously deferred but now added immediately
            ctx.inlined_stmts.push(Stmt::Assign(assign_clone));
        } else {
            ctx.inlined_stmts.push(Stmt::Assign(assign_clone));
        }
    }

    /// Inline an annotated assignment statement
    pub(crate) fn inline_ann_assignment(
        &mut self,
        ann_assign: &ruff_python_ast::StmtAnnAssign,
        module_name: &str,
        module_id: crate::resolver::ModuleId,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Expr::Name(name) = ann_assign.target.as_ref() else {
            return;
        };

        let var_name = name.id.to_string();
        if !self.should_inline_symbol(&var_name, module_id, ctx.module_exports_map) {
            return;
        }

        // Check if this symbol was renamed by semantic analysis
        let renamed_name = self.resolve_renamed_name(&var_name, module_name, ctx);

        // Always track the symbol mapping, even if not renamed
        module_renames.insert(var_name.clone(), renamed_name.clone());
        if renamed_name != var_name {
            log::debug!(
                "Renaming annotated variable '{var_name}' to '{renamed_name}' in module \
                 '{module_name}'"
            );
        }
        ctx.global_symbols.insert(renamed_name.clone());

        // Clone and rename the annotated assignment
        let mut ann_assign_clone = ann_assign.clone();
        if let Expr::Name(name_expr) = ann_assign_clone.target.as_mut() {
            name_expr.id = renamed_name.into();
        }

        // Also rewrite annotation expressions to handle pre-transform aliases
        expression_handlers::resolve_import_aliases_in_expr(
            &mut ann_assign_clone.annotation,
            &ctx.import_aliases,
        );
        expression_handlers::rewrite_aliases_in_expr(
            &mut ann_assign_clone.annotation,
            module_renames,
        );
        if let Some(semantic_renames) = ctx.module_renames.get(&module_id) {
            expression_handlers::rewrite_aliases_in_expr(
                &mut ann_assign_clone.annotation,
                semantic_renames,
            );
        }

        ctx.inlined_stmts.push(Stmt::AnnAssign(ann_assign_clone));
    }
}
