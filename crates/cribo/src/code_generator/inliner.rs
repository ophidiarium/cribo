//! Module inlining functionality for the bundler
//!
//! This module handles the inlining of Python modules into the final bundle,
//! including class, assignment, and annotation inlining.

use std::path::Path;

use anyhow::Result;
use log::debug;
use ruff_python_ast::{Expr, Identifier, ModModule, Stmt, StmtAssign, StmtClassDef};
use ruff_text_size::TextRange;

use super::{
    bundler::Bundler,
    context::InlineContext,
    expression_handlers, import_deduplicator,
    import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
    module_registry::generate_unique_name,
};
use crate::{
    ast_builder::statements,
    types::{FxIndexMap, FxIndexSet},
};

/// Result of the inlining process
pub struct InliningResult {
    /// The statements generated from inlining
    pub statements: Vec<Stmt>,
    /// Import statements that need to be deferred
    pub deferred_imports: Vec<Stmt>,
}

impl Bundler<'_> {
    /// Resolve the renamed name for a symbol, considering semantic renames and conflicts
    fn resolve_renamed_name(
        &self,
        original_name: &str,
        module_name: &str,
        ctx: &InlineContext,
    ) -> String {
        // Check if there's a semantic rename that's different from the original
        if let Some(new_name) = ctx
            .module_renames
            .get(module_name)
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
    pub fn inline_module(
        &mut self,
        module_name: &str,
        mut ast: ModModule,
        module_path: &Path,
        ctx: &mut InlineContext,
    ) -> Result<Vec<Stmt>> {
        let mut module_renames = FxIndexMap::default();

        // Apply hard dependency rewriting BEFORE import transformation
        if !self.hard_dependencies.is_empty() && self.circular_modules.contains(module_name) {
            self.rewrite_hard_dependencies_in_module(&mut ast, module_name);
        }

        // Then apply recursive import transformation to the module
        let mut transformer = RecursiveImportTransformer::new(RecursiveImportTransformerParams {
            bundler: self,
            module_name,
            module_path: Some(module_path),
            symbol_renames: ctx.module_renames,
            deferred_imports: ctx.deferred_imports,
            is_entry_module: false, // This is not the entry module
            is_wrapper_init: false, // Not a wrapper init
            global_deferred_imports: Some(&self.global_deferred_imports), // Pass global registry
        });
        transformer.transform_module(&mut ast);

        // Copy import aliases from the transformer to the inline context
        ctx.import_aliases = transformer.import_aliases;

        // Reorder statements to ensure proper declaration order
        let statements = if self.circular_modules.contains(module_name) {
            self.reorder_statements_for_circular_module(module_name, ast.body, ctx.python_version)
        } else {
            // Even for non-circular modules, ensure module-level variables are declared
            // before functions that might use them
            self.reorder_statements_for_proper_declaration_order(ast.body)
        };

        // Build a map of imported symbols to their source modules
        ctx.import_sources = self.build_import_source_map(&statements, module_name);

        // Process each statement in the module
        for stmt in statements {
            match &stmt {
                Stmt::Import(import_stmt) => {
                    // Imports have already been transformed by RecursiveImportTransformer
                    // Include them in the inlined output
                    if !import_deduplicator::is_hoisted_import(self, &stmt) {
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
                    if !import_deduplicator::is_hoisted_import(self, &stmt) {
                        ctx.inlined_stmts.push(stmt.clone());
                    }
                }
                Stmt::FunctionDef(func_def) => {
                    let func_name = func_def.name.to_string();
                    if !self.should_inline_symbol(&func_name, module_name, ctx.module_exports_map) {
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
                    if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
                        expression_handlers::rewrite_aliases_in_stmt(
                            &mut temp_stmt,
                            semantic_renames,
                        );
                    }

                    ctx.inlined_stmts.push(temp_stmt);
                }
                Stmt::ClassDef(class_def) => {
                    self.inline_class(class_def, module_name, &mut module_renames, ctx);
                }
                Stmt::Assign(assign) => {
                    self.inline_assignment(assign, module_name, &mut module_renames, ctx);
                }
                Stmt::AnnAssign(ann_assign) => {
                    self.inline_ann_assignment(ann_assign, module_name, &mut module_renames, ctx);
                }
                // TypeAlias statements are safe metadata definitions
                Stmt::TypeAlias(_) => {
                    // Type aliases don't need renaming in Python, they're just metadata
                    ctx.inlined_stmts.push(stmt);
                }
                // Pass statements are no-ops and safe
                Stmt::Pass(_) => {
                    // Pass statements can be included as-is
                    ctx.inlined_stmts.push(stmt);
                }
                // Expression statements that are string literals are docstrings
                Stmt::Expr(expr_stmt) => {
                    if matches!(expr_stmt.value.as_ref(), Expr::StringLiteral(_)) {
                        // This is a docstring - safe to include
                        ctx.inlined_stmts.push(stmt);
                    } else {
                        // Other expression statements shouldn't exist in side-effect-free modules
                        log::warn!(
                            "Unexpected expression statement in side-effect-free module \
                             '{module_name}': {stmt:?}"
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
            ctx.module_renames
                .insert(module_name.to_string(), module_renames);
        }

        Ok(Vec::new()) // Statements are accumulated in ctx.inlined_stmts
    }

    /// Inline a class definition
    pub(crate) fn inline_class(
        &mut self,
        class_def: &StmtClassDef,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let class_name = class_def.name.to_string();
        if !self.should_inline_symbol(&class_name, module_name, ctx.module_exports_map) {
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

        // Apply renames to base classes
        // CRITICAL: For cross-module inheritance, we need to apply renames from the
        // source module of each base class, not just from the current module.
        if let Some(ref mut arguments) = class_def_clone.arguments {
            for arg in &mut arguments.args {
                // Try to determine the source module for base class names
                if let Expr::Name(name_expr) = arg {
                    let base_class_name = name_expr.id.as_str();

                    // Check if this base class was imported from another module
                    if let Some(source_module) = ctx.import_sources.get(base_class_name) {
                        // This base class was imported from another module
                        // Use that module's renames instead of the current module's
                        if let Some(source_renames) = ctx.module_renames.get(source_module)
                            && let Some(renamed) = source_renames.get(base_class_name)
                        {
                            log::debug!(
                                "Applying cross-module rename for base class '{base_class_name}' \
                                 from module '{source_module}': '{base_class_name}' -> '{renamed}'"
                            );
                            name_expr.id = renamed.clone().into();
                            continue;
                        }
                    }

                    // Not imported or no rename found in source module, apply local renames
                    if let Some(renamed) = module_renames.get(base_class_name) {
                        name_expr.id = renamed.clone().into();
                    }
                } else {
                    // Complex base class expression, use standard rewriting
                    expression_handlers::rewrite_aliases_in_expr(arg, module_renames);
                }
            }
        }

        // Apply renames and resolve import aliases in class body
        for body_stmt in &mut class_def_clone.body {
            Self::resolve_import_aliases_in_stmt(body_stmt, &ctx.import_aliases);
            expression_handlers::rewrite_aliases_in_stmt(body_stmt, module_renames);
            // Also apply semantic renames from context
            if let Some(semantic_renames) = ctx.module_renames.get(module_name) {
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
        let Some(name) = expression_handlers::extract_simple_assign_target(assign) else {
            return;
        };

        // Special handling for circular modules: include private module-level variables
        // that may be used by public functions
        let is_circular_module = self.circular_modules.contains(module_name);
        let is_single_underscore_private = name.starts_with('_') && !name.starts_with("__");

        // For circular modules, we need special handling of private variables
        if is_circular_module && is_single_underscore_private {
            // For circular modules, we always include single-underscore private module-level
            // variables because they might be used by functions that are part of the
            // circular dependency
            log::debug!("Including private variable '{name}' from circular module '{module_name}'");
        } else if !self.should_inline_symbol(&name, module_name, ctx.module_exports_map) {
            // For all other cases, use the standard inlining check
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
                "Deferring assignment '{name}' in module '{module_name}' as it references a \
                 namespace module"
            );
            ctx.deferred_imports.push(Stmt::Assign(assign_clone));
        } else {
            ctx.inlined_stmts.push(Stmt::Assign(assign_clone));
        }
    }

    /// Inline an annotated assignment statement
    pub(crate) fn inline_ann_assignment(
        &mut self,
        ann_assign: &ruff_python_ast::StmtAnnAssign,
        module_name: &str,
        module_renames: &mut FxIndexMap<String, String>,
        ctx: &mut InlineContext,
    ) {
        let Expr::Name(name) = ann_assign.target.as_ref() else {
            return;
        };

        let var_name = name.id.to_string();
        if !self.should_inline_symbol(&var_name, module_name, ctx.module_exports_map) {
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
        ctx.inlined_stmts.push(Stmt::AnnAssign(ann_assign_clone));
    }
}

/// Inline all modules into the bundle
pub fn inline_all_modules(
    bundler: &mut Bundler,
    inlinable_modules: &[(String, ModModule, std::path::PathBuf, String)],
    module_exports_map: &FxIndexMap<String, Option<Vec<String>>>,
    symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
    global_symbols: &mut FxIndexSet<String>,
    python_version: u8,
) -> Result<InliningResult> {
    let mut all_deferred_imports = Vec::new();
    let mut all_inlined_stmts = Vec::new();

    for (module_name, ast, module_path, _content_hash) in inlinable_modules {
        debug!("Inlining module '{module_name}'");

        let mut inlined_stmts = Vec::new();
        let mut deferred_imports = Vec::new();
        let mut inline_ctx = InlineContext {
            module_exports_map,
            global_symbols,
            module_renames: symbol_renames,
            inlined_stmts: &mut inlined_stmts,
            import_aliases: FxIndexMap::default(),
            deferred_imports: &mut deferred_imports,
            import_sources: FxIndexMap::default(),
            python_version,
        };
        bundler.inline_module(module_name, ast.clone(), module_path, &mut inline_ctx)?;
        debug!(
            "Inlined {} statements from module '{}'",
            inlined_stmts.len(),
            module_name
        );
        all_inlined_stmts.extend(inlined_stmts);

        // Filter deferred imports to avoid conflicts
        // If an inlined module imports a symbol but doesn't export it,
        // and that symbol would conflict with other imports, skip it
        for stmt in deferred_imports {
            let should_include = if let Stmt::Assign(assign) = &stmt {
                if let [Expr::Name(target)] = assign.targets.as_slice()
                    && let Expr::Name(_value) = &*assign.value
                {
                    let symbol_name = target.id.as_str();

                    // Check if this module exports the symbol
                    let exports_symbol =
                        if let Some(Some(exports)) = module_exports_map.get(module_name) {
                            exports.contains(&symbol_name.to_string())
                        } else {
                            // No explicit __all__, check if it's a module-level definition
                            // For now, assume it's not exported if there's no __all__
                            false
                        };

                    if exports_symbol {
                        true
                    } else {
                        // Check if this would conflict with existing deferred imports
                        let has_conflict = all_deferred_imports.iter().any(|existing| {
                            if let Stmt::Assign(existing_assign) = existing
                                && let [Expr::Name(existing_target)] =
                                    existing_assign.targets.as_slice()
                            {
                                existing_target.id.as_str() == symbol_name
                            } else {
                                false
                            }
                        });

                        if has_conflict {
                            debug!(
                                "Skipping deferred import '{symbol_name}' from module \
                                 '{module_name}' due to conflict"
                            );
                            false
                        } else {
                            true
                        }
                    }
                } else {
                    true
                }
            } else {
                true
            };

            if should_include {
                // Check if this deferred import already exists in all_deferred_imports
                let is_duplicate = if let Stmt::Assign(assign) = &stmt {
                    if let Expr::Name(target) = &assign.targets[0] {
                        let target_name = target.id.as_str();

                        // Check against existing deferred imports
                        all_deferred_imports.iter().any(|existing| {
                            if let Stmt::Assign(existing_assign) = existing
                                && let [Expr::Name(existing_target)] =
                                    existing_assign.targets.as_slice()
                                && existing_target.id.as_str() == target_name
                            {
                                expression_handlers::expr_equals(
                                    &assign.value,
                                    &existing_assign.value,
                                )
                            } else {
                                false
                            }
                        })
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !is_duplicate {
                    all_deferred_imports.push(stmt);
                }
            }
        }
    }

    Ok(InliningResult {
        statements: all_inlined_stmts,
        deferred_imports: all_deferred_imports,
    })
}
