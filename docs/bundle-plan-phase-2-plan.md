# Phase 2 Implementation Plan: Sequential Analysis Pipeline

## Overview

Phase 2 transforms the bundling architecture from multiple side-channel communications to a clean sequential pipeline following the Progressive Enrichment principle. This plan is based on deep-code-reasoning analysis that identified key architectural improvements.

## Key Principles

1. **Progressive Enrichment**: Data only becomes more structured as it flows through the pipeline
2. **Single AST Traversal**: Expensive AST work happens once during graph construction
3. **Fast Graph Algorithms**: Subsequent analyses use efficient graph traversals
4. **Decoupled Stages**: Each analysis produces independent results
5. **Final Assembly**: BundlePlan is built from all analysis results at the end

## Implementation Checklist

### Step 1: Fix CircularDependencyAnalysis Data Flow

#### 1.1 Refactor Data Structures

- [x] Create `ModuleEdge` struct to replace string-based `ImportEdge`
  ```rust
  pub struct ModuleEdge {
      pub from_module: ModuleId,
      pub to_module: ModuleId,
      pub edge_type: EdgeType,
  }
  ```
- [x] Update `CircularDependencyGroup` to use `ModuleId` instead of `String`
  ```rust
  pub struct CircularDependencyGroup {
      pub module_ids: Vec<ModuleId>, // was: modules: Vec<String>
      pub cycle_type: CircularDependencyType,
      pub import_edges: Vec<ModuleEdge>, // was: import_chain: Vec<ImportEdge>
      pub suggested_resolution: ResolutionStrategy,
  }
  ```
- [x] Update `ResolutionStrategy` enum to use structured data

#### 1.2 Update Analysis Logic

- [x] Modify `analyze_circular_dependencies` to work directly on `CriboGraph`
- [x] Remove string conversions from cycle detection logic
- [x] Update cycle classification to use `ModuleId` comparisons
- [x] Implement `build_import_chain_for_scc` using `ModuleEdge`

#### 1.3 Move String Formatting to Orchestrator

- [x] Create formatter functions in orchestrator for error messages
- [x] Update error reporting to convert `ModuleId` to names only for display
- [x] Keep analysis results in structured form throughout pipeline

### Step 2: Implement Two-Pass GraphBuilder

#### 2.1 Create Result Structures

- [ ] Define `GraphBuildResult` struct
  ```rust
  pub struct GraphBuildResult {
      pub symbol_map: FxHashMap<String, NodeIndex>,
      pub item_mappings: ItemMappings,
  }
  ```
- [ ] Define `ItemMappings` struct
  ```rust
  pub struct ItemMappings {
      pub item_to_node: FxHashMap<ItemId, AtomicNodeIndex>,
      pub node_to_item: FxHashMap<AtomicNodeIndex, ItemId>,
  }
  ```

#### 2.2 Implement Pass A: Symbol Discovery

- [ ] Create method to traverse AST and discover all top-level definitions
- [ ] For each symbol:
  - [ ] Create `ItemData` and add to graph
  - [ ] Store `NodeIndex` in local symbol map
  - [ ] Populate item-to-node mappings
- [ ] Handle all definition types:
  - [ ] Functions (`FunctionDef`)
  - [ ] Classes (`ClassDef`)
  - [ ] Module-level assignments
  - [ ] Imports (as items)

#### 2.3 Implement Pass B: Dependency Wiring

- [ ] Create method to traverse AST again for dependency analysis
- [ ] Use symbol map from Pass A to resolve intra-module references
- [ ] Create edges for:
  - [ ] Function calls
  - [ ] Class instantiations
  - [ ] Variable references
  - [ ] Import usage

#### 2.4 Update Orchestrator Integration

- [ ] Modify `process_module` to receive `GraphBuildResult`
- [ ] Update `ModuleInfo` with item mappings from result
- [ ] Implement inter-module dependency wiring in orchestrator

### Step 3: Create Analysis Pipeline Structure

#### 3.1 Create Analysis Module

- [ ] Create `crates/cribo/src/analysis/mod.rs`
- [ ] Define `AnalysisResults` struct
  ```rust
  pub struct AnalysisResults {
      pub circular_deps: Option<CircularDependencyAnalysis>,
      pub symbol_conflicts: Vec<SymbolConflict>,
      pub tree_shake_results: Option<TreeShakeResults>,
  }
  ```

#### 3.2 Implement Pipeline Runner

- [ ] Create `run_analysis_pipeline` function
- [ ] Implement sequential stages:
  1. [ ] Cycle detection (fast graph algorithm)
  2. [ ] Semantic analysis (hybrid traversal)
  3. [ ] Tree-shaking (graph traversal)
- [ ] Ensure each stage receives immutable `CriboGraph`
- [ ] Collect all results into `AnalysisResults`

#### 3.3 Update Individual Analyzers

- [ ] Update `SemanticBundler` to produce `Vec<SymbolConflict>`
- [ ] Update `TreeShaker` to produce `TreeShakeResults`
- [ ] Ensure analyzers don't mutate graph or registry

### Step 4: Implement BundlePlan Assembly

#### 4.1 Create Assembly Method

- [ ] Add `from_analysis_results` method to `BundlePlan`
- [ ] Accept:
  - [ ] `&CriboGraph`
  - [ ] `&AnalysisResults`
  - [ ] `&ModuleRegistry`

#### 4.2 Convert Analysis Results to Plan Entries

##### 4.2.1 Circular Dependencies

- [ ] Create `add_circular_dep_rewrites` helper
- [ ] For each resolvable cycle:
  - [ ] Identify specific imports to move
  - [ ] Determine target functions
  - [ ] Create `ImportRewrite` entries
- [ ] Handle different resolution strategies:
  - [ ] Function-scoped imports
  - [ ] Lazy imports
  - [ ] Deferred initialization

##### 4.2.2 Symbol Conflicts

- [ ] Create `add_symbol_rename` helper
- [ ] For each conflict:
  - [ ] Determine rename strategy
  - [ ] Add to `symbol_renames` map
  - [ ] Track affected modules

##### 4.2.3 Tree-Shaking

- [ ] Create `add_tree_shake_decisions` helper
- [ ] Convert used items to `live_items` map
- [ ] Mark modules for removal if completely unused

##### 4.2.4 Module Classification

- [ ] Create `classify_modules` helper
- [ ] Determine bundle type for each module:
  - [ ] Inlinable (no side effects)
  - [ ] Wrapper (has side effects)
  - [ ] Conditional (complex logic)
- [ ] Set module metadata in plan

### Step 5: Update Orchestrator for Sequential Pipeline

#### 5.1 Refactor `bundle_core`

- [ ] Replace individual analysis calls with pipeline runner
- [ ] Remove side-channel communications
- [ ] Update flow:
  1. [ ] Build graph (immutable after this)
  2. [ ] Run analysis pipeline
  3. [ ] Build BundlePlan from results
  4. [ ] Generate code with plan

#### 5.2 Update Helper Methods

- [ ] Update `emit_static_bundle` to use only BundlePlan
- [ ] Remove `build_bundle_plan` placeholder from Phase 1
- [ ] Clean up analysis-specific parameters

### Step 6: Update Code Generator

#### 6.1 Remove Analysis Dependencies

- [ ] Remove `circular_dep_analysis` parameter from `BundleParams`
- [ ] Remove direct `SemanticBundler` usage
- [ ] Use only `bundle_plan` for decisions

#### 6.2 Implement Plan Execution

- [ ] Create methods to apply each type of plan decision:
  - [ ] `apply_import_rewrites`
  - [ ] `apply_symbol_renames`
  - [ ] `filter_dead_code`
  - [ ] `handle_module_types`

### Step 7: Testing and Validation

#### 7.1 Unit Tests

- [ ] Test two-pass GraphBuilder with various AST patterns
- [ ] Test CircularDependencyAnalysis with ModuleId
- [ ] Test each analysis stage independently
- [ ] Test BundlePlan assembly from various inputs

#### 7.2 Integration Tests

- [ ] Compare output with Phase 1 implementation
- [ ] Verify circular dependency resolution still works
- [ ] Test symbol conflict resolution
- [ ] Validate tree-shaking behavior

#### 7.3 Performance Tests

- [ ] Benchmark two-pass vs single-pass GraphBuilder
- [ ] Measure pipeline performance vs old approach
- [ ] Verify memory usage improvements

### Step 8: Cleanup

#### 8.1 Remove Old Code

- [ ] Remove string-based `ImportEdge` struct
- [ ] Remove old circular dependency analysis methods
- [ ] Remove side-channel communication code
- [ ] Clean up unused imports and methods

#### 8.2 Documentation

- [ ] Update architecture documentation
- [ ] Document new pipeline flow
- [ ] Add examples of BundlePlan structure
- [ ] Update code comments

## Success Criteria

- [ ] All tests pass
- [ ] No clippy warnings
- [ ] Performance equal or better than Phase 1
- [ ] Clear separation of concerns achieved
- [ ] No data regression (Progressive Enrichment maintained)
- [ ] BundlePlan contains all bundling decisions

## Risk Mitigation

1. **Risk**: Two-pass GraphBuilder might be slower
   - **Mitigation**: Benchmark and optimize if needed
   - **Fallback**: Cache AST traversal results

2. **Risk**: Missing some analysis in new pipeline
   - **Mitigation**: Comprehensive testing against Phase 1
   - **Fallback**: Add missing analysis to pipeline

3. **Risk**: BundlePlan assembly too complex
   - **Mitigation**: Break into smaller helper functions
   - **Fallback**: Incremental assembly with validation

## Completion Checklist

- [ ] All code changes implemented
- [ ] All tests written and passing
- [ ] Performance validated
- [ ] Documentation updated
- [ ] Code review completed
- [ ] Branch ready for merge
