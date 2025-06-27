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

- [x] Define `GraphBuildResult` struct
  ```rust
  pub struct GraphBuildResult {
      pub symbol_map: FxHashMap<String, NodeIndex>,
      pub item_mappings: ItemMappings,
  }
  ```
- [x] Define `ItemMappings` struct
  ```rust
  pub struct ItemMappings {
      pub item_to_node: FxHashMap<ItemId, AtomicNodeIndex>,
      pub node_to_item: FxHashMap<AtomicNodeIndex, ItemId>,
  }
  ```

#### 2.2 Implement Pass A: Symbol Discovery

- [x] Create method to traverse AST and discover all top-level definitions
- [x] For each symbol:
  - [x] Create `ItemData` and add to graph
  - [x] Store `NodeIndex` in local symbol map
  - [x] Populate item-to-node mappings
- [x] Handle all definition types:
  - [x] Functions (`FunctionDef`)
  - [x] Classes (`ClassDef`)
  - [x] Module-level assignments
  - [x] Imports (as items)

#### 2.3 Implement Pass B: Dependency Wiring

- [x] Create method to traverse AST again for dependency analysis
- [ ] Use symbol map from Pass A to resolve intra-module references
- [ ] Create edges for:
  - [ ] Function calls
  - [ ] Class instantiations
  - [ ] Variable references
  - [ ] Import usage
- [ ] **Extract symbol names from statements** (needed for namespace assignment)

#### 2.4 Update Orchestrator Integration

- [x] Modify `process_module` to receive `GraphBuildResult`
- [x] Update `ModuleInfo` with item mappings from result
- [x] Add `use_two_pass_graph_builder` config option
- [x] Add environment variable support for two-pass mode
- [ ] Implement inter-module dependency wiring in orchestrator

### Step 3: Create Analysis Pipeline Structure

#### 3.1 Create Analysis Module

- [x] Create `crates/cribo/src/analysis/mod.rs`
- [x] Define `AnalysisResults` struct
  ```rust
  pub struct AnalysisResults {
      pub circular_deps: Option<CircularDependencyAnalysis>,
      pub symbol_conflicts: Vec<SymbolConflict>,
      pub tree_shake_results: Option<TreeShakeResults>,
  }
  ```

#### 3.2 Implement Pipeline Runner

- [x] Create `run_analysis_pipeline` function
- [x] Implement sequential stages:
  1. [x] Cycle detection (fast graph algorithm)
  2. [x] Semantic analysis (hybrid traversal)
  3. [x] Tree-shaking (graph traversal)
- [x] Ensure each stage receives immutable `CriboGraph`
- [x] Collect all results into `AnalysisResults`

#### 3.3 Update Individual Analyzers

- [x] ~~Update `SemanticBundler` to produce `Vec<SymbolConflict>`~~ (Deprecated - using different approach)
- [x] Update `TreeShaker` to produce `TreeShakeResults`
- [x] Ensure analyzers don't mutate graph or registry

### Step 4: Implement BundlePlan Assembly

#### 4.1 Create Assembly Method

- [x] Add `from_analysis_results` method to `BundlePlan`
- [x] Accept:
  - [x] `&CriboGraph`
  - [x] `&AnalysisResults`
  - [x] `&ModuleRegistry`

#### 4.2 Convert Analysis Results to Plan Entries

##### 4.2.1 Circular Dependencies

- [x] Create `add_circular_dep_rewrites` helper
- [x] For each resolvable cycle:
  - [x] Identify specific imports to move
  - [x] Determine target functions
  - [x] Create `ImportRewrite` entries
- [x] Handle different resolution strategies:
  - [x] Function-scoped imports
  - [x] Lazy imports
  - [x] Deferred initialization

##### 4.2.2 Symbol Conflicts

- [x] Create `add_symbol_rename` helper
- [x] For each conflict:
  - [x] Determine rename strategy
  - [x] Add to `symbol_renames` map
  - [x] Track affected modules

##### 4.2.3 Tree-Shaking

- [x] Create `add_tree_shake_decisions` helper
- [x] Convert used items to `live_items` map
- [x] Mark modules for removal if completely unused

##### 4.2.4 Module Classification

- [x] Create `classify_modules` helper
- [x] Determine bundle type for each module:
  - [x] Inlinable (no side effects)
  - [x] Wrapper (has side effects)
  - [x] Conditional (complex logic)
- [x] Set module metadata in plan

### Step 5: Update Orchestrator for Sequential Pipeline

#### 5.1 Refactor `bundle_core`

- [x] Replace individual analysis calls with pipeline runner
- [x] Remove side-channel communications
- [x] Update flow:
  1. [x] Build graph (immutable after this)
  2. [x] Run analysis pipeline
  3. [x] Build BundlePlan from results
  4. [x] Generate code with plan
- [ ] Fix issue: Semantic bundler symbol renaming not properly integrated with BundlePlan

#### 5.2 Update Helper Methods

- [x] Update `emit_static_bundle` to use only BundlePlan
- [x] Remove `build_bundle_plan` placeholder from Phase 1
- [x] Clean up analysis-specific parameters

### Step 6: Update Code Generator

#### 6.1 Remove Analysis Dependencies

- [x] ~~Remove `circular_dep_analysis` parameter from `BundleParams`~~ (Deprecated - using ExecutionStep approach)
- [x] ~~Remove direct `SemanticBundler` usage~~ (Deprecated - using ExecutionStep approach)
- [x] Use only `bundle_plan` for decisions

#### 6.2 Implement Plan Execution

- [x] ~~Create methods to apply each type of plan decision:~~ (Replaced with ExecutionStep approach)
  - [x] ~~`apply_import_rewrites`~~ → ExecutionStep variants
  - [x] ~~`apply_symbol_renames`~~ → AST transformer in plan_executor
  - [x] ~~`filter_dead_code`~~ → Handled by live_items in BundlePlan
  - [x] ~~`handle_module_types`~~ → ExecutionStep variants
- [x] **NEW**: Implement dumb plan executor with ExecutionStep enum
- [ ] **CURRENT**: Generate proper ExecutionSteps for namespace modules

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

- [x] All code changes implemented (with known issue: semantic bundler integration)
- [ ] All tests written and passing (5 tests failing due to semantic bundler issue)
- [ ] Performance validated
- [ ] Documentation updated
- [ ] Code review completed
- [ ] Branch ready for merge

## Known Issues

1. ~~**Semantic Bundler Integration**: The semantic bundler's internal symbol renaming is not properly integrated with the BundlePlan.~~ **RESOLVED** - Replaced with ExecutionStep-based approach

2. **Symbol Extraction for Namespace Assignment**: Need to extract symbol names from module statements to generate proper `CopyStatementToNamespace` steps. Currently falling back to `InlineStatement` which doesn't create the namespace assignments.
