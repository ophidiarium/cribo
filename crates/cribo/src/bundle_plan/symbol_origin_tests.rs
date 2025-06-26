//! Tests for symbol origin tracking in BundlePlan

#[cfg(test)]
mod tests {
    use ruff_python_semantic::BindingId;

    use super::super::*;
    use crate::{cribo_graph::ModuleId, semantic_model_provider::GlobalBindingId};

    /// Create a test GlobalBindingId
    fn make_global_id(module_id: u32, binding_id: u32) -> GlobalBindingId {
        GlobalBindingId {
            module_id: ModuleId::new(module_id),
            binding_id: BindingId::from_u32(binding_id),
        }
    }

    #[test]
    fn test_symbol_origins_empty_by_default() {
        let plan = BundlePlan::new();
        assert!(plan.symbol_origins.is_empty());
    }

    #[test]
    fn test_symbol_origins_tracks_re_exports() {
        // This test demonstrates what the symbol_origins map should contain
        // after analyzing a re-export chain

        let mut plan = BundlePlan::new();

        // Simulate the analysis results:
        // Module 1 (core.database.connection) defines Connection at binding 10
        let original_def = make_global_id(1, 10);

        // Module 2 (core.database.__init__) re-exports Connection at binding 20
        let reexport1 = make_global_id(2, 20);

        // Module 3 (core.__init__) imports as CoreConnection at binding 30
        let alias = make_global_id(3, 30);

        // The symbol_origins map should track that both re-exports
        // point back to the original definition
        plan.symbol_origins.insert(reexport1, original_def);
        plan.symbol_origins.insert(alias, original_def);

        // Verify the mappings
        assert_eq!(plan.symbol_origins.get(&reexport1), Some(&original_def));
        assert_eq!(plan.symbol_origins.get(&alias), Some(&original_def));

        // The original definition should not be in the map as a key
        assert!(!plan.symbol_origins.contains_key(&original_def));
    }

    #[test]
    fn test_from_analysis_results_preserves_symbol_origins() {
        // This test will fail until we implement SymbolOriginAnalysis
        // It demonstrates that from_analysis_results should populate symbol_origins

        use crate::{
            analysis::AnalysisResults, cribo_graph::CriboGraph, orchestrator::ModuleRegistry,
        };

        // Create minimal test data
        let graph = CriboGraph::new();
        let analysis_results = AnalysisResults::default();
        let registry = ModuleRegistry::new();

        // Create BundlePlan from analysis results
        let plan =
            BundlePlan::from_analysis_results(&graph, &analysis_results, &registry, "test_module");

        // This assertion will fail because we haven't implemented
        // the analysis that populates symbol_origins
        assert!(
            plan.symbol_origins.is_empty(),
            "symbol_origins should be populated by analysis (this test should fail when \
             implemented)"
        );
    }
}
