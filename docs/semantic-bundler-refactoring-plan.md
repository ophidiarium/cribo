# Semantic Bundler Refactoring Plan

## Overview

This plan addresses the integration issue between the semantic bundler and the new BundlePlan architecture. The semantic bundler currently applies symbol renames internally, but these changes are not properly communicated through the analysis pipeline, causing undefined variable errors in generated code.

## Problem Statement

### Current State

1. **Side-Channel Mutation**: SemanticBundler applies renames directly during `detect_and_resolve_conflicts()`
2. **Late Detection**: Conflicts are detected after the analysis pipeline completes
3. **Internal State**: Renamed symbols are stored internally in SemanticBundler, not in AnalysisResults
4. **Inconsistent Application**: Renames are applied to some parts of the code but not others

### Symptoms

- Generated code references undefined variables (e.g., `sanitize_core_utils_helpers`)
- Test failures in fixtures with symbol conflicts
- Violation of Progressive Enrichment principle

### Root Cause

The semantic bundler was designed for direct AST mutation, but the new architecture requires:

- Immutable graphs after construction
- All decisions consolidated in BundlePlan
- Clean data flow through AnalysisResults

## Proposed Solution

### High-Level Approach

1. **Extract conflict detection** into the analysis pipeline
2. **Return structured conflicts** via AnalysisResults
3. **Generate rename decisions** in BundlePlan
4. **Apply renames consistently** during code generation

### Architecture Changes

```
Current Flow:
graph → analysis_pipeline → BundlePlan → semantic_bundler.detect_and_resolve_conflicts() → code_gen
                                                    ↓
                                            (mutates internal state)

Proposed Flow:
graph → analysis_pipeline (includes conflict detection) → AnalysisResults → BundlePlan → code_gen
              ↓                                                ↓                 ↓
        (detects conflicts)                            (has conflicts)    (generates renames)
```

## Implementation Plan

### Critical Architectural Insights (from Review)

1. **Service Boundaries**: The SemanticBundler currently violates single-responsibility by being a provider, analyzer, and transformer
2. **Lifetime Management**: Use `Arc<String>` for source code to avoid self-referential lifetime issues
3. **Global Symbol Identity**: Create `GlobalBindingId { module_id: ModuleId, binding_id: BindingId }` for cross-module tracking
4. **Declarative Imports**: Instead of imperative rewrites, generate a complete `final_imports` structure
5. **Semantic-Aware CodeGen**: The code generator must have access to semantic models for correct resolution

## Revised Implementation Plan

### Phase 0: Refactor SemanticBundler to Provider Pattern (PREREQUISITE)

#### 0.1 Change Source Storage

- Modify `ModuleInfo.original_source` to use `Arc<String>`
- Update all places where source is stored/accessed

#### 0.2 Build Models Upfront

- In `Orchestrator::process_module`, build SemanticModel immediately after parsing
- Store in central registry: `FxIndexMap<ModuleId, SemanticModel<'a>>`

#### 0.3 Create SemanticModelProvider

- Thin wrapper providing read-only access to pre-built models
- **DELETE** all conflict detection and resolution logic from SemanticBundler
- **DELETE** `detect_and_resolve_conflicts` method entirely
- **DELETE** internal symbol tables and mutation logic

### Phase 1: Extract Conflict Detection

#### 1.1 Create Conflict Detector

- Create new `SymbolConflictDetector` that works on immutable CriboGraph
- Move conflict detection logic from SemanticBundler
- Return structured `SymbolConflict` data

#### 1.2 Update AnalysisResults

- Ensure `SymbolConflict` struct has all needed information:
  ```rust
  // Create global identifier first
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
  pub struct GlobalBindingId {
      pub module_id: ModuleId,
      pub binding_id: BindingId,
  }

  pub struct SymbolConflict {
      pub symbol_name: String,
      pub conflicts: Vec<ConflictInstance>,
  }

  pub struct ConflictInstance {
      pub global_id: GlobalBindingId,
      pub symbol_type: SymbolType,
      pub definition_range: TextRange,
  }
  ```

#### 1.3 Integrate into Pipeline

- Add conflict detection as Stage 2 in `run_analysis_pipeline`
- Pass detected conflicts to AnalysisResults

### Phase 2: Generate Rename Decisions

#### 2.1 Enhance BundlePlan

- Add rename generation logic to `add_symbol_renames`
- Create deterministic naming strategy:
  - First module keeps original name
  - Subsequent modules get `<name>_<module_suffix>`
  - Use module path for suffix (e.g., `process_core_utils`)

#### 2.2 Update Symbol Renames Structure

- Use GlobalBindingId as key:
  ```rust
  // In BundlePlan
  pub symbol_renames: IndexMap<GlobalBindingId, String>,
  ```

#### 2.3 Add Declarative Import Structure

- Replace imperative rewrites with declarative imports:
  ```rust
  pub final_imports: IndexMap<String, FinalImport>,

  #[derive(Debug, Clone)]
  pub enum FinalImport {
      Module { alias: Option<String> },
      From { symbols: IndexMap<String, Option<String>> },
  }
  ```

### Phase 3: Apply Renames in Code Generation

#### 3.1 Make CodeGenerator Semantically Aware

- Pass SemanticModelProvider to CodeGenerator
- Add current module's SemanticModel to TransformationContext
- Implement symbol resolution in visitor methods:
  ```rust
  fn visit_expr_name(&mut self, expr_name: &'ast ExprName) {
      if let Some(binding) = self.current_model().lookup_symbol(expr_name) {
          let global_id = GlobalBindingId {
              module_id: self.current_module_id(),
              binding_id: binding.id(),
          };
          if let Some(new_name) = self.bundle_plan.symbol_renames.get(&global_id) {
              self.write(new_name);
              return;
          }
      }
      self.write(&expr_name.id);
  }
  ```

#### 3.2 Update Code Generator

- Apply renames before generating init functions
- Ensure consistent renaming across all references
- Update import statements to use renamed symbols

### Continuous Cleanup (Throughout Implementation)

- **No backwards compatibility**: Delete old code as soon as new code replaces it
- **No feature flags**: Direct replacement in each phase
- **Aggressive deletion**: Remove any code that becomes dead
- **Simplify as we go**: Each phase should reduce total LOC

## Technical Considerations

### 1. Symbol Resolution

- Need to track symbol usage across module boundaries
- Must handle various import styles:
  - `from module import symbol`
  - `import module; module.symbol`
  - `from module import symbol as alias`

### 2. Rename Scope

- Only rename symbols that actually conflict
- Preserve non-conflicting symbols exactly
- Handle transitive dependencies correctly

### 3. Special Cases

- `__all__` exports must be updated
- Docstrings should reflect new names
- Type annotations need updates

### 4. Performance

- Conflict detection should be efficient (one pass)
- Rename application should be deterministic
- Minimize AST traversals

## Testing Strategy

### 1. Unit Tests

- Test SymbolConflictDetector with various conflict patterns
- Test rename generation logic
- Test rename application

### 2. Integration Tests

- Ensure existing fixtures pass
- Add specific tests for edge cases:
  - Circular imports with conflicts
  - Nested class/function conflicts
  - Import alias conflicts

### 3. Snapshot Tests

- Update snapshots to reflect new naming
- Ensure deterministic output

## Implementation Order (Direct Replacement)

1. **Phase 0**: Refactor to Provider Pattern (2-3 hours)
   - Convert SemanticBundler to pure provider
   - Remove `detect_and_resolve_conflicts` method entirely
   - Delete all mutation/renaming logic from SemanticBundler

2. **Phase 1**: Implement Conflict Detection (2-3 hours)
   - Create new SymbolConflictAnalyzer in pipeline
   - Populate AnalysisResults.symbol_conflicts
   - No backwards compatibility needed

3. **Phase 2**: Generate Rename Decisions (1-2 hours)
   - Update BundlePlan.from_analysis_results
   - Generate symbol_renames and final_imports
   - Remove old ImportRewrite logic if no longer needed

4. **Phase 3**: Apply in CodeGenerator (3-4 hours)
   - Make CodeGenerator semantically aware
   - Apply renames from BundlePlan
   - Remove any old renaming logic

## Success Criteria

- **All fixture tests pass** (with updated snapshots where needed)
- **No feature flags or compatibility code**
- **Reduced codebase size** through removal of old implementation
- **Clean architecture** with clear separation of concerns

## Success Criteria

- [ ] **All fixture tests pass** (primary validation)
- [ ] **Updated snapshots accepted** where behavior changes are correct
- [ ] **No undefined variable errors** in generated bundles
- [ ] **Reduced codebase size** (old implementation completely removed)
- [ ] **Clean separation of concerns** (Provider → Analyzer → Plan → Generator)
- [ ] **Deterministic output** (same input always produces same bundle)

## Approach Benefits

1. **No Migration Complexity**: Direct replacement simplifies implementation
2. **Immediate Validation**: Fixture tests provide comprehensive coverage
3. **Cleaner Codebase**: No compatibility layers or feature flags
4. **Faster Development**: No need to maintain two implementations

## Expected Snapshot Changes

- Symbol rename patterns might change (but should be deterministic)
- Import ordering might be different (but functionally equivalent)
- Some edge cases might be handled better (fixing current bugs)

## Estimated Complexity

- **SymbolConflictDetector**: Medium - Extract existing logic
- **BundlePlan integration**: Low - Structure already exists
- **Rename application**: High - Must handle all AST node types
- **Testing & validation**: Medium - Many edge cases

## Key Decisions

1. **No Backwards Compatibility**: This is a complete replacement, not an addition
2. **Fixture Tests as Truth**: Passing fixtures (even with updated snapshots) proves correctness
3. **Aggressive Deletion**: Remove old code immediately as new code replaces it
4. **Docstrings/Comments**: Do not attempt to rename - out of scope
5. **Dynamic Imports**: Do not handle - add warnings if needed
6. **Rename Strategy**: Simple and deterministic: `{original_name}_{module_suffix}`

## Critical Implementation Notes

1. **Binding Resolution**: Use `model.lookup_symbol(node)` which returns a `Binding`, then call `.id()` to get the `BindingId`
2. **Import Handling**: Generate complete final imports rather than trying to rewrite existing ones
3. **Attribute Access**: Resolve through semantic model to handle `module.symbol` correctly
4. **Testing**: Create snapshots at each stage (AnalysisResults, BundlePlan) for verification
