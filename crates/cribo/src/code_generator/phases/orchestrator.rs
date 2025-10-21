//! Phase Orchestrator
//!
//! This module provides the orchestration of code generation phases.
//! The `PhaseOrchestrator` coordinates the execution of individual bundling phases
//! and manages data flow between them.

use ruff_python_ast::{ModModule, Stmt};
use ruff_text_size::TextRange;

use crate::{
    analyzers::SymbolAnalyzer,
    code_generator::{
        bundler::Bundler,
        context::{BundleParams, SemanticContext},
        phases::{
            classification::ClassificationPhase,
            entry_module::EntryModulePhase,
            initialization::{InitializationPhase, generate_future_import_statements},
            post_processing::PostProcessingPhase,
            processing::ProcessingPhase,
        },
    },
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Phase orchestrator for the code generation process
///
/// With the stateless phase design, the orchestrator can now sequentially
/// execute each phase without violating Rust's borrow checker rules.
pub(crate) struct PhaseOrchestrator;

impl PhaseOrchestrator {
    /// Execute the complete bundling process using the phase-based architecture
    ///
    /// This method orchestrates all phases of bundling:
    /// 1. Initialization: Setup and future imports collection
    /// 2. Preparation: Module trimming and AST indexing
    /// 3. Classification: Separate inlinable vs wrapper modules
    /// 4. Symbol Rename Collection: Gather renames from semantic analysis
    /// 5. Global Symbol Collection: Extract global symbols
    /// 6. Processing: Main module processing loop
    /// 7. Entry Module Processing: Special handling for entry module
    /// 8. Post-Processing: Namespace attachment, proxy generation, aliases
    /// 9. Finalization: Assemble final module and log statistics
    ///
    /// Returns the final bundled `ModModule`.
    pub(crate) fn bundle<'a>(bundler: &mut Bundler<'a>, params: &BundleParams<'a>) -> ModModule {
        let mut final_body = Vec::new();

        // Phase 1: Initialization
        log::debug!("[Orchestrator] Phase 1: Initialization");
        let init_phase = InitializationPhase::new();
        let init_result = init_phase.execute(bundler, params);

        // Add future imports to final body
        let future_import_stmts = generate_future_import_statements(&init_result);
        final_body.extend(future_import_stmts);

        // Phase 2: Preparation (module trimming, AST indexing)
        log::debug!("[Orchestrator] Phase 2: Preparation");
        let mut modules = bundler.prepare_modules(params);

        // Phase 3: Classification
        log::debug!("[Orchestrator] Phase 3: Classification");
        let classification_phase = ClassificationPhase::new();
        let classification = classification_phase.execute(bundler, &modules, params.python_version);

        // Phase 4: Symbol Rename Collection
        log::debug!("[Orchestrator] Phase 4: Symbol Rename Collection");
        let semantic_ctx = SemanticContext {
            graph: params.graph,
            symbol_registry: params.conflict_resolver.symbol_registry(),
            conflict_resolver: params.conflict_resolver,
        };
        let mut symbol_renames = bundler.collect_symbol_renames(&modules, &semantic_ctx);

        // Handle entry module symbol renaming to avoid namespace collisions
        Self::handle_entry_symbol_renaming(bundler, &modules, &mut symbol_renames);

        // Phase 5: Global Symbol Collection
        log::debug!("[Orchestrator] Phase 5: Global Symbol Collection");
        let modules_vec: Vec<(ModuleId, &ModModule, &std::path::Path, &str)> = modules
            .iter()
            .map(|(id, (ast, path, hash))| (*id, ast, path.as_path(), hash.as_str()))
            .collect();
        let mut global_symbols = SymbolAnalyzer::collect_global_symbols(&modules_vec);

        // Phase 6: Main Processing Loop
        log::debug!("[Orchestrator] Phase 6: Processing");
        let processing_phase = ProcessingPhase::new();
        let (processing_stmts, _processed_modules) = processing_phase.execute(
            bundler,
            params,
            &classification,
            &modules,
            &mut symbol_renames,
            &mut global_symbols,
        );
        final_body.extend(processing_stmts);

        // Phase 7: Entry Module Processing
        log::debug!("[Orchestrator] Phase 7: Entry Module");
        let entry_phase = EntryModulePhase::new();
        let entry_result =
            entry_phase.execute(bundler, params, &mut modules, &symbol_renames, &final_body);

        let (entry_symbols, entry_renames) = if let Some(result) = entry_result {
            final_body.extend(result.statements);
            (result.entry_symbols, result.entry_renames)
        } else {
            (FxIndexSet::default(), FxIndexMap::default())
        };

        // Phase 8: Post-Processing
        log::debug!("[Orchestrator] Phase 8: Post-Processing");
        let post_processing_phase = PostProcessingPhase::new();
        let post_processing_output =
            post_processing_phase.execute(bundler, &entry_symbols, &entry_renames, &final_body);

        // Insert proxy statements after __future__ imports
        PostProcessingPhase::insert_proxy_statements(
            post_processing_output.proxy_statements,
            &mut final_body,
        );

        // Add package child aliases
        final_body.extend(post_processing_output.alias_statements);

        // Add namespace attachments (if any)
        final_body.extend(post_processing_output.namespace_attachments);

        // Phase 9: Finalization
        log::debug!("[Orchestrator] Phase 9: Finalization");
        Self::finalize_bundle(bundler, final_body)
    }

    /// Handle entry module symbol renaming to avoid namespace collisions
    fn handle_entry_symbol_renaming(
        bundler: &Bundler<'_>,
        modules: &FxIndexMap<ModuleId, (ModModule, std::path::PathBuf, String)>,
        symbol_renames: &mut FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    ) {
        use ruff_python_ast::Stmt;

        if let Some((entry_ast, _entry_path, _)) = modules.get(&ModuleId::ENTRY) {
            let entry_is_wrapper = bundler.module_init_functions.contains_key(&ModuleId::ENTRY)
                || bundler.circular_modules.contains(&ModuleId::ENTRY);

            if entry_is_wrapper {
                let entry_mod_name = &bundler.entry_module_name;
                let entry_var =
                    crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                        entry_mod_name,
                    );
                let mut entry_map = symbol_renames
                    .shift_remove(&ModuleId::ENTRY)
                    .unwrap_or_default();

                let mut try_rename = |name: &str, kind: &str| {
                    if name == entry_var {
                        let renamed = bundler
                            .get_unique_name_with_module_suffix(entry_var.as_str(), entry_mod_name);
                        log::debug!(
                            "Registering entry-module {kind} rename to avoid namespace collision: \
                             '{entry_var}' -> '{renamed}'"
                        );
                        entry_map.insert(entry_var.clone(), renamed);
                    }
                };

                for stmt in &entry_ast.body {
                    match stmt {
                        Stmt::FunctionDef(func) => try_rename(func.name.as_str(), "function"),
                        Stmt::ClassDef(class_def) => try_rename(class_def.name.as_str(), "class"),
                        _ => {}
                    }
                }

                if !entry_map.is_empty() {
                    symbol_renames.insert(ModuleId::ENTRY, entry_map);
                }
            }
        }
    }

    /// Finalize the bundle and create the final `ModModule`
    fn finalize_bundle(bundler: &mut Bundler<'_>, final_body: Vec<Stmt>) -> ModModule {
        log::debug!(
            "Creating final ModModule with {} statements",
            final_body.len()
        );
        for (i, stmt) in final_body.iter().take(3).enumerate() {
            log::debug!("Statement {}: type = {:?}", i, std::mem::discriminant(stmt));
        }

        let result = ModModule {
            node_index: bundler.create_transformed_node("Bundled module root".to_owned()),
            range: TextRange::default(),
            body: final_body,
        };

        // Log transformation statistics
        let stats = bundler.transformation_context.get_stats();
        log::info!("Transformation statistics:");
        log::info!("  Total transformations: {}", stats.total_transformations);
        log::info!("  New nodes created: {}", stats.new_nodes);

        result
    }
}
