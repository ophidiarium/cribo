# BundlePlan Refactoring Completion Plan

## Executive Summary

The BundlePlan refactoring is incomplete, leaving behind architectural confusion. We have two vestigial systems (BundlePlan and final_layout) that need to be removed to achieve the clean architecture described in `execution-step-architecture.md`.

## Current State vs. Target State

### Current State (Confusing)

```
AnalysisResults ──┬──> BundleCompiler ──> BundleProgram ──> BundleVM
                  │
                  └──> BundlePlan (unused)
                        └──> final_layout (unused experiment)
```

### Target State (Clean)

```
AnalysisResults ──> BundleCompiler ──> BundleProgram ──> BundleVM
```

## Root Causes

1. **Incomplete Refactoring**: The old architecture (BundlePlan as shared mutable state) was partially replaced but not fully removed
2. **Abandoned Experiment**: The final_layout system was an attempt to move from imperative (ExecutionStep) to declarative output, but was abandoned mid-implementation

## Action Plan

### Phase 1: Test Migration (1-2 hours)

**Goal**: Remove BundlePlan dependencies from tests

1. Identify all tests using `BundlePlan` or `BundlePlan::from_analysis_results`
   ```bash
   rg "BundlePlan::|use.*BundlePlan" --glob "**/*test*.rs"
   ```

2. For each test, refactor from:
   ```rust
   // Old pattern
   let bundle_plan = BundlePlan::from_analysis_results(&graph, &analysis_results, &registry, "module");
   assert_eq!(bundle_plan.symbol_renames.len(), expected);
   ```

   To:
   ```rust
   // New pattern
   let compiler = BundleCompiler::new(&analysis_results, &graph, &registry, "module")?;
   let program = compiler.compile()?;
   // Assert against program fields or execution results
   ```

3. Update test assertions to work with `BundleProgram` instead of `BundlePlan`

### Phase 2: Remove BundlePlan (1 hour)

**Goal**: Delete all BundlePlan-related code

1. Delete `crates/cribo/src/bundle_plan/builder.rs`
2. Remove from `crates/cribo/src/bundle_plan/mod.rs`:
   - `pub mod builder;`
   - `struct BundlePlan` definition
   - `impl BundlePlan` block
   - Any helper methods like `from_analysis_results`

3. Remove any imports/uses of BundlePlan throughout the codebase

### Phase 3: Remove final_layout (1 hour)

**Goal**: Delete the abandoned experiment

1. Delete `crates/cribo/src/bundle_plan/final_layout.rs`
2. Remove from `crates/cribo/src/bundle_plan/mod.rs`:
   - `pub mod final_layout;`
   - All `pub use final_layout::*` statements
   - The misleading comment: `/// NOTE: This will be deprecated in favor of final_layout`

### Phase 4: Module Rename (1 hour)

**Goal**: Rename bundle_plan to bundle_compiler for semantic clarity

1. Rename directory: `bundle_plan/` → `bundle_compiler/`
2. Update all imports across the codebase:
   ```rust
   // Old
   // New
   use crate::{
       bundle_compiler::{BundleCompiler, BundleProgram},
       bundle_plan::{BundleCompiler, BundleProgram},
   };
   ```

3. Update `crates/cribo/src/lib.rs` to export the renamed module

### Phase 5: Final Cleanup (30 minutes)

1. Clean up `bundle_compiler/mod.rs`:
   - Remove any remaining deprecated comments
   - Ensure only necessary exports (BundleCompiler, BundleProgram, ExecutionStep)
   - Add clear module documentation

2. Update documentation:
   - Ensure design docs match the final architecture
   - Remove any references to BundlePlan or final_layout

## Validation Checklist

- [ ] All tests pass after refactoring
- [ ] No references to `BundlePlan` remain in the codebase
- [ ] No references to `final_layout` remain in the codebase
- [ ] Module is renamed to `bundle_compiler`
- [ ] Documentation is updated
- [ ] The flow is clean: `AnalysisResults → BundleCompiler → BundleProgram → BundleVM`

## Benefits After Completion

1. **Clear Architecture**: No confusion about which components are active vs. vestigial
2. **Semantic Clarity**: Module names match their purpose
3. **Maintainability**: New developers won't waste time understanding dead code
4. **Single Source of Truth**: One clear path through the bundling pipeline

## Risk Assessment

- **Low Risk**: These changes only remove unused code and rename modules
- **Test Coverage**: Existing tests will be preserved, just refactored
- **No Functional Changes**: The working pipeline remains unchanged

## Timeline

Total estimated time: 4-6 hours

This can be done in a single focused session or split across multiple commits for easier review.
