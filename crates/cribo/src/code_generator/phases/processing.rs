//! Processing Phase
//!
//! This phase handles the main module processing loop, including:
//! - Analyzing wrapper dependencies
//! - Building circular dependency groups
//! - Processing modules in dependency order
//! - Two-phase emission for circular dependencies
//! - Inlinable and wrapper module handling

use std::path::{Path, PathBuf};

use ruff_python_ast::Stmt;

use crate::{
    analyzers::module_classifier::ClassificationResult,
    code_generator::{bundler::Bundler, context::BundleParams},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Processing phase handler (stateless)
#[derive(Default)]
pub(crate) struct ProcessingPhase;

/// Context for SCC group processing
#[derive(Debug)]
struct CircularGroupContext {
    cycle_groups: Vec<Vec<ModuleId>>,
    member_to_group: FxIndexMap<ModuleId, usize>,
}

impl ProcessingPhase {
    /// Create a new processing phase
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Execute the processing phase
    ///
    /// This is the main module processing loop that:
    /// 1. Analyzes wrapper module dependencies
    /// 2. Builds circular dependency groups (SCCs)
    /// 3. Processes modules in dependency order
    /// 4. Handles circular dependencies with two-phase emission
    /// 5. Processes inlinable and wrapper modules
    ///
    /// Returns the generated statements and set of processed modules.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn execute(
        &self,
        bundler: &mut Bundler<'_>,
        params: &BundleParams<'_>,
        classification: &ClassificationResult,
        modules: &FxIndexMap<ModuleId, (ruff_python_ast::ModModule, PathBuf, String)>,
        symbol_renames: &mut FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        global_symbols: &mut FxIndexSet<String>,
    ) -> (Vec<Stmt>, FxIndexSet<ModuleId>) {
        // Analyze wrapper dependencies
        Self::analyze_wrapper_dependencies(bundler, classification, modules);

        // Build SCC groups for circular dependency handling
        let circular_ctx = Self::build_circular_groups(params);

        // Create module sets for quick lookup
        let inlinable_set: FxIndexSet<ModuleId> = classification
            .inlinable_modules
            .iter()
            .map(|(id, _, _, _)| *id)
            .collect();
        let wrapper_set: FxIndexSet<ModuleId> = classification
            .wrapper_modules
            .iter()
            .map(|(id, _, _, _)| *id)
            .collect();

        let mut all_inlined_stmts = Vec::new();
        let mut processed_modules: FxIndexSet<ModuleId> = FxIndexSet::default();
        let mut processed_cycle_groups: FxIndexSet<usize> = FxIndexSet::default();

        // Log the dependency order
        Self::log_processing_order(params);

        // Process each module in dependency order
        for module_id in params.sorted_module_ids {
            // Skip if already processed
            if processed_modules.contains(module_id) {
                continue;
            }

            let module_name = params
                .resolver
                .get_module_name(*module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));

            // Check if this module is part of a cycle group
            if let Some(group_idx) = circular_ctx.member_to_group.get(module_id) {
                if !processed_cycle_groups.contains(group_idx) {
                    Self::process_circular_group(
                        bundler,
                        *group_idx,
                        &circular_ctx,
                        modules,
                        symbol_renames,
                        params.python_version,
                        &mut all_inlined_stmts,
                        &mut processed_modules,
                    );
                    processed_cycle_groups.insert(*group_idx);
                }
                continue;
            }

            // Skip if not in our module set
            if !modules.contains_key(module_id) {
                log::debug!("  Skipping {module_name} - not in module map (likely stdlib)");
                continue;
            }

            log::info!(
                "Processing module: {} (inlinable: {}, wrapper: {})",
                module_name,
                inlinable_set.contains(module_id),
                wrapper_set.contains(module_id)
            );

            let (ast, path, _hash) = modules.get(module_id).expect("Module should exist").clone();

            if inlinable_set.contains(module_id) {
                Self::process_inlinable_module(
                    bundler,
                    *module_id,
                    &module_name,
                    ast,
                    &path,
                    &classification.module_exports_map,
                    symbol_renames,
                    params.python_version,
                    global_symbols,
                    &mut all_inlined_stmts,
                );
                processed_modules.insert(*module_id);
            } else if wrapper_set.contains(module_id) {
                Self::process_wrapper_module(
                    bundler,
                    *module_id,
                    &module_name,
                    ast,
                    &path,
                    symbol_renames,
                    params.python_version,
                    modules,
                    &mut all_inlined_stmts,
                );
                processed_modules.insert(*module_id);
            }
        }

        (all_inlined_stmts, processed_modules)
    }

    /// Analyze wrapper module dependencies
    ///
    /// This method performs critical dependency analysis to determine which wrapper
    /// modules must be initialized before inlinable modules can reference them.
    /// It computes both direct and transitive dependencies.
    fn analyze_wrapper_dependencies(
        bundler: &Bundler<'_>,
        classification: &ClassificationResult,
        _modules: &FxIndexMap<ModuleId, (ruff_python_ast::ModModule, PathBuf, String)>,
    ) {
        // Check if any wrapper participates in circular dependencies
        let has_circular_wrapped_modules = classification
            .wrapper_modules
            .iter()
            .any(|(module_id, _, _, _)| bundler.is_module_in_circular_deps(*module_id));

        if has_circular_wrapped_modules {
            log::info!(
                "Detected circular dependencies in modules with side effects - special handling \
                 required"
            );
        }

        // Track wrapper modules needed directly by inlinable modules
        let mut wrapper_modules_needed_by_inlined: FxIndexSet<ModuleId> = FxIndexSet::default();
        for (module_id, ast, module_path, _) in &classification.inlinable_modules {
            for stmt in &ast.body {
                let Stmt::ImportFrom(import_from) = stmt else {
                    continue;
                };
                bundler.collect_wrapper_needed_from_importfrom_for_inlinable(
                    *module_id,
                    import_from,
                    module_path,
                    &classification.wrapper_modules,
                    &mut wrapper_modules_needed_by_inlined,
                );
            }
        }

        // Collect wrapper-to-wrapper dependencies
        let mut wrapper_to_wrapper_deps: FxIndexMap<ModuleId, FxIndexSet<ModuleId>> =
            FxIndexMap::default();
        for (module_id, ast, module_path, _) in &classification.wrapper_modules {
            for stmt in &ast.body {
                let Stmt::ImportFrom(import_from) = stmt else {
                    continue;
                };
                bundler.collect_wrapper_to_wrapper_deps_from_stmt(
                    *module_id,
                    import_from,
                    module_path,
                    &classification.wrapper_modules,
                    &mut wrapper_to_wrapper_deps,
                );
            }
        }

        // Compute transitive dependencies
        let mut all_needed = wrapper_modules_needed_by_inlined.clone();
        let mut to_process = wrapper_modules_needed_by_inlined.clone();

        while !to_process.is_empty() {
            let mut next_to_process = FxIndexSet::default();
            for module in &to_process {
                if let Some(deps) = wrapper_to_wrapper_deps.get(module) {
                    for dep in deps {
                        if !all_needed.contains(dep) {
                            all_needed.insert(*dep);
                            next_to_process.insert(*dep);
                            let module_name = bundler
                                .resolver
                                .get_module_name(*module)
                                .unwrap_or_else(|| format!("module_{}", module.as_u32()));
                            let dep_name = bundler
                                .resolver
                                .get_module_name(*dep)
                                .unwrap_or_else(|| format!("module_{}", dep.as_u32()));
                            log::debug!(
                                "Adding transitive dependency: {dep_name} (needed by \
                                 {module_name})"
                            );
                        }
                    }
                }
            }
            to_process = next_to_process;
        }
    }

    /// Build circular dependency groups (SCCs)
    fn build_circular_groups(params: &BundleParams<'_>) -> CircularGroupContext {
        let mut cycle_groups: Vec<Vec<ModuleId>> = Vec::new();

        if let Some(analysis) = params.circular_dep_analysis {
            for group in &analysis.resolvable_cycles {
                cycle_groups.push(group.modules.clone());
            }
            for group in &analysis.unresolvable_cycles {
                cycle_groups.push(group.modules.clone());
            }
        }

        let mut member_to_group: FxIndexMap<ModuleId, usize> = FxIndexMap::default();
        for (idx, group) in cycle_groups.iter().enumerate() {
            for &mid in group {
                member_to_group.insert(mid, idx);
            }
        }

        CircularGroupContext {
            cycle_groups,
            member_to_group,
        }
    }

    /// Log the processing order for debugging
    fn log_processing_order(params: &BundleParams<'_>) {
        log::debug!("Module processing order from dependency graph:");
        for (i, module_id) in params.sorted_module_ids.iter().enumerate() {
            let module_name = params
                .resolver
                .get_module_name(*module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
            let module_path = params.resolver.get_module_path(*module_id);
            log::debug!("  {}. {} (path: {:?})", i + 1, module_name, module_path);
        }
    }

    /// Process a circular dependency group with two-phase emission
    #[expect(clippy::too_many_arguments)]
    fn process_circular_group(
        bundler: &mut Bundler<'_>,
        group_idx: usize,
        circular_ctx: &CircularGroupContext,
        modules: &FxIndexMap<ModuleId, (ruff_python_ast::ModModule, PathBuf, String)>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        python_version: u8,
        all_inlined_stmts: &mut Vec<Stmt>,
        processed_modules: &mut FxIndexSet<ModuleId>,
    ) {
        // Delegate to existing bundler method
        // This keeps the complex two-phase logic intact for now
        // Can be further decomposed in a future iteration
        use crate::{
            ast_builder::{expressions, statements},
            code_generator::{
                context::ModuleTransformContext, init_function::InitFunctionBuilder,
                module_registry::sanitize_module_name_for_identifier,
            },
        };

        let _params = bundler.graph.expect("graph must be set");

        // Get group members
        let mut members: Vec<(ModuleId, String)> = circular_ctx.cycle_groups[group_idx]
            .iter()
            .filter_map(|mid| {
                bundler
                    .resolver
                    .get_module_name(*mid)
                    .map(|name| (*mid, name))
            })
            .collect();

        // Sort by package depth, then by name
        members.sort_by(|a, b| {
            let depth_a = a.1.matches('.').count();
            let depth_b = b.1.matches('.').count();
            depth_a.cmp(&depth_b).then_with(|| a.1.cmp(&b.1))
        });

        log::debug!(
            "Processing SCC group ({} modules): {:?}",
            members.len(),
            members.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>()
        );

        // Phase A: Predeclare module objects
        for (mid, mname) in &members {
            if processed_modules.contains(mid) {
                continue;
            }

            // Register as wrapper if not already
            if !bundler.module_synthetic_names.contains_key(mid) {
                if let Some((_, _path, hash)) = modules.get(mid) {
                    crate::code_generator::module_registry::register_module(
                        *mid,
                        mname,
                        hash,
                        &mut bundler.module_synthetic_names,
                        &mut bundler.module_init_functions,
                    );
                }
                bundler.inlined_modules.shift_remove(mid);
            }

            // Check if package
            let is_package = modules.get(mid).is_some_and(|(_, p, _)| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(crate::python::module_path::is_init_file_name)
            });

            // Create namespace
            let sanitized = sanitize_module_name_for_identifier(mname);
            if !bundler.created_namespaces.contains(&sanitized) {
                let mut ns_stmts = crate::ast_builder::module_wrapper::create_wrapper_module(
                    mname, "", // No init function, so no init_func_name needed
                    None, is_package,
                );
                all_inlined_stmts.append(&mut ns_stmts);
                bundler.created_namespaces.insert(sanitized.clone());
            }

            // Ensure parent-child attribute attachment exists
            bundler.create_namespace_chain_for_module(mname, &sanitized, all_inlined_stmts);

            processed_modules.insert(*mid);
        }

        // Phase B: Define init functions
        for (mid, mname) in &members {
            let (ast, path, _hash) = modules.get(mid).expect("cycle member must exist").clone();

            let global_info = crate::analyzers::GlobalAnalyzer::analyze(mname, &ast);
            let is_in_circular = circular_ctx.member_to_group.contains_key(mid);

            let transform_ctx = ModuleTransformContext {
                module_name: mname,
                module_path: &path,
                global_info: global_info.clone(),
                conflict_resolver: bundler.conflict_resolver,
                python_version,
                is_wrapper_body: true,
                is_in_circular_deps: is_in_circular,
            };

            let init_function = InitFunctionBuilder::new(bundler, &transform_ctx, symbol_renames)
                .build(ast.clone())
                .expect("Init function transformation should not fail");

            // Insert lifted globals
            if let Some(ref info) = global_info
                && !info.global_declarations.is_empty()
            {
                let lifter = crate::code_generator::globals::GlobalsLifter::new(info);
                for (_, lifted_name) in &lifter.lifted_names {
                    all_inlined_stmts.push(statements::simple_assign(
                        lifted_name,
                        expressions::none_literal(),
                    ));
                }
            }

            if bundler.emitted_wrapper_inits.insert(*mid) {
                let init_func_name = if let Stmt::FunctionDef(f) = &init_function {
                    f.name.as_str().to_owned()
                } else {
                    bundler
                        .module_init_functions
                        .get(mid)
                        .expect("init function must exist")
                        .clone()
                };

                let mut init_stmts =
                    crate::ast_builder::module_wrapper::create_init_function_statements(
                        mname,
                        &init_func_name,
                        init_function,
                    );
                all_inlined_stmts.append(&mut init_stmts);
            }
        }
    }

    /// Process an inlinable module
    #[expect(clippy::too_many_arguments)]
    fn process_inlinable_module(
        bundler: &mut Bundler<'_>,
        module_id: ModuleId,
        module_name: &str,
        ast: ruff_python_ast::ModModule,
        path: &Path,
        module_exports_map: &FxIndexMap<ModuleId, Option<Vec<String>>>,
        symbol_renames: &mut FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        python_version: u8,
        global_symbols: &mut FxIndexSet<String>,
        all_inlined_stmts: &mut Vec<Stmt>,
    ) {
        use ruff_python_ast::{Identifier, Keyword};
        use ruff_text_size::TextRange;

        use crate::{
            ast_builder::{expressions, statements},
            code_generator::{
                context::InlineContext, docstring_extractor,
                module_registry::sanitize_module_name_for_identifier,
            },
        };

        log::debug!("Inlining module: {module_name}");

        // Create namespace for inlinable modules
        let namespace_var = sanitize_module_name_for_identifier(module_name);
        if !bundler.created_namespaces.contains(&namespace_var) {
            log::debug!("Creating namespace for inlinable module '{module_name}'");

            let module_docstring = docstring_extractor::extract_module_docstring(&ast);

            let mut keywords = vec![Keyword {
                node_index: bundler.create_node_index(),
                arg: Some(Identifier::new("__name__", TextRange::default())),
                value: expressions::string_literal(module_name),
                range: TextRange::default(),
            }];

            if let Some(docstring) = module_docstring {
                keywords.push(Keyword {
                    node_index: bundler.create_node_index(),
                    arg: Some(Identifier::new("__doc__", TextRange::default())),
                    value: expressions::string_literal(&docstring),
                    range: TextRange::default(),
                });
            }

            let namespace_stmt = statements::simple_assign(
                &namespace_var,
                expressions::call(expressions::simple_namespace_ctor(), vec![], keywords),
            );
            all_inlined_stmts.push(namespace_stmt);
            bundler.created_namespaces.insert(namespace_var.clone());

            bundler.create_namespace_chain_for_module(
                module_name,
                &namespace_var,
                all_inlined_stmts,
            );
        }

        // Create inline context
        let mut inline_ctx = InlineContext {
            module_exports_map,
            global_symbols,
            module_renames: symbol_renames,
            inlined_stmts: all_inlined_stmts,
            import_aliases: FxIndexMap::default(),
            import_sources: FxIndexMap::default(),
            python_version,
        };

        // Actually inline the module code
        bundler.inline_module(module_name, ast, path, &mut inline_ctx);

        // Populate namespace with symbols
        log::debug!("[processing] Populating namespace for inlined module: {module_name}");

        let namespace_var = sanitize_module_name_for_identifier(module_name);
        let population_ctx = crate::code_generator::namespace_manager::NamespacePopulationContext {
            bundled_modules: &bundler.bundled_modules,
            inlined_modules: &bundler.inlined_modules,
            module_exports: &bundler.module_exports,
            tree_shaking_keep_symbols: &bundler.tree_shaking_keep_symbols,
            modules_with_accessed_all: &bundler.modules_with_accessed_all,
            wrapper_modules: &bundler.wrapper_modules,
            module_asts: &bundler.module_asts,
            modules_with_explicit_all: &bundler.modules_with_explicit_all,
            module_init_functions: &bundler.module_init_functions,
            resolver: bundler.resolver,
        };

        let population_stmts =
            crate::code_generator::namespace_manager::populate_namespace_with_module_symbols(
                &population_ctx,
                &namespace_var,
                module_id,
                symbol_renames,
            );

        all_inlined_stmts.extend(population_stmts);
        bundler.modules_with_populated_symbols.insert(module_id);
    }

    /// Process a wrapper module
    #[expect(clippy::too_many_arguments)]
    fn process_wrapper_module(
        bundler: &mut Bundler<'_>,
        module_id: ModuleId,
        module_name: &str,
        ast: ruff_python_ast::ModModule,
        path: &Path,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        python_version: u8,
        modules: &FxIndexMap<ModuleId, (ruff_python_ast::ModModule, PathBuf, String)>,
        all_inlined_stmts: &mut Vec<Stmt>,
    ) {
        use crate::{
            ast_builder::{expressions, statements},
            code_generator::{
                context::ModuleTransformContext, init_function::InitFunctionBuilder,
                module_registry::sanitize_module_name_for_identifier,
            },
        };

        log::debug!("Processing wrapper module: {module_name}");

        let content_hash = modules
            .get(&module_id)
            .map_or_else(|| "000000".to_owned(), |(_, _, hash)| hash.clone());

        let _synthetic_name = bundler
            .module_synthetic_names
            .entry(module_id)
            .or_insert_with(|| {
                crate::code_generator::module_registry::get_synthetic_module_name(
                    module_name,
                    &content_hash,
                )
            })
            .clone();

        let init_func_name_from_map = bundler
            .module_init_functions
            .get(&module_id)
            .expect("init function must exist")
            .clone();

        let global_info = crate::analyzers::GlobalAnalyzer::analyze(module_name, &ast);
        let is_in_circular = bundler.is_module_in_circular_deps(module_id);

        let transform_ctx = ModuleTransformContext {
            module_name,
            module_path: path,
            global_info: global_info.clone(),
            conflict_resolver: bundler.conflict_resolver,
            python_version,
            is_wrapper_body: true,
            is_in_circular_deps: is_in_circular,
        };

        let init_function = InitFunctionBuilder::new(bundler, &transform_ctx, symbol_renames)
            .build(ast)
            .expect("Init function transformation should not fail");

        let init_func_name = if let Stmt::FunctionDef(f) = &init_function {
            f.name.as_str().to_owned()
        } else {
            init_func_name_from_map
        };

        let is_package = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(crate::python::module_path::is_init_file_name);

        let module_var = sanitize_module_name_for_identifier(module_name);
        let namespace_already_exists = bundler.created_namespaces.contains(&module_var);

        let mut wrapper_stmts = if namespace_already_exists {
            if bundler.emitted_wrapper_inits.insert(module_id) {
                crate::ast_builder::module_wrapper::create_init_function_statements(
                    module_name,
                    &init_func_name,
                    init_function,
                )
            } else {
                Vec::new()
            }
        } else if bundler.emitted_wrapper_inits.insert(module_id) {
            crate::ast_builder::module_wrapper::create_wrapper_module(
                module_name,
                &init_func_name,
                Some(init_function),
                is_package,
            )
        } else {
            crate::ast_builder::module_wrapper::create_wrapper_module(
                module_name,
                "",
                None,
                is_package,
            )
        };

        // Insert lifted globals
        if let Some(ref info) = global_info {
            if !info.global_declarations.is_empty() {
                let globals_lifter = crate::code_generator::globals::GlobalsLifter::new(info);
                let mut lifted_declarations = Vec::new();
                for (_, lifted_name) in &globals_lifter.lifted_names {
                    lifted_declarations.push(statements::simple_assign(
                        lifted_name,
                        expressions::none_literal(),
                    ));
                }

                if !lifted_declarations.is_empty() && !wrapper_stmts.is_empty() {
                    if namespace_already_exists {
                        all_inlined_stmts.extend(lifted_declarations);
                    } else if wrapper_stmts.len() >= 2 {
                        let namespace_stmt = wrapper_stmts.remove(0);
                        all_inlined_stmts.push(namespace_stmt);
                        all_inlined_stmts.extend(lifted_declarations);
                    }
                }
            }
        }
        all_inlined_stmts.extend(wrapper_stmts);

        if !namespace_already_exists {
            bundler.created_namespaces.insert(module_var.clone());
        }

        bundler.create_namespace_chain_for_module(module_name, &module_var, all_inlined_stmts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_group_context_construction() {
        let cycle_groups = vec![
            vec![ModuleId::ENTRY, ModuleId::new(1)],
            vec![ModuleId::new(2), ModuleId::new(3)],
        ];

        let mut member_to_group = FxIndexMap::default();
        for (idx, group) in cycle_groups.iter().enumerate() {
            for &mid in group {
                member_to_group.insert(mid, idx);
            }
        }

        let ctx = CircularGroupContext {
            cycle_groups,
            member_to_group: member_to_group.clone(),
        };

        assert_eq!(ctx.cycle_groups.len(), 2);
        assert_eq!(ctx.member_to_group.len(), 4);
        assert_eq!(ctx.member_to_group.get(&ModuleId::ENTRY), Some(&0));
        assert_eq!(ctx.member_to_group.get(&ModuleId::new(2)), Some(&1));
    }

    #[test]
    fn test_empty_circular_group_context() {
        let ctx = CircularGroupContext {
            cycle_groups: vec![],
            member_to_group: FxIndexMap::default(),
        };

        assert!(ctx.cycle_groups.is_empty());
        assert!(ctx.member_to_group.is_empty());
    }
}
