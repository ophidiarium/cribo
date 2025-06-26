# Import Handling Design Refinements - Summary

## Critical Architectural Insights

### 1. The Symbol Resolution Timing Paradox - SOLVED

**Problem**: Single-pass analysis creates circular dependency between tree-shaking and import resolution.

**Solution**: Multi-pass analysis architecture:

- Pass 1: Discover all potential exports (before tree-shaking)
- Pass 2: Build dependency graph and perform tree-shaking
- Pass 3: Generate final plan using results from both passes

### 2. ExecutionStep vs Declarative Structure - RESOLVED

**Problem**: Flat list of execution steps requires complex sorting logic in the "dumb" executor.

**Solution**: Replace with declarative `FinalBundleLayout` struct that enforces correct file structure:

```rust
struct FinalBundleLayout {
    future_imports: Vec<String>,           // Must be first
    hoisted_imports: Vec<HoistedImport>,   // External imports
    namespace_creations: Vec<String>,       // Empty namespaces
    inlined_code: Vec<(ModuleId, ItemId)>, // All module code
    namespace_populations: Vec<...>,        // Populate namespaces
}
```

### 3. ImportCategory Ambiguity - CLARIFIED

**Problem**: Enum conflates import source with required action.

**Solution**: New `ImportAction` enum that explicitly describes the operation:

- `HoistVerbatim` - for `__future__` imports
- `HoistExternal` - for stdlib/third-party
- `CreateNamespace` - for `import module`
- `InlineSymbol` - for `from module import symbol`
- `LeaveInPlace` - for function-scoped imports

## Key Implementation Decisions

### Two-Stage Namespace Initialization

Solves circular import dependencies elegantly:

1. Create all namespace objects at bundle top
2. Execute all module code
3. Populate namespaces at bundle end

This ensures all symbols exist before any namespace is populated.

### Edge Case Handling Rules

1. **TYPE_CHECKING**: Always False (skip if body)
2. **`__name__ == "__main__"`**: True for entry point only
3. **Function-scoped imports**: Leave untouched
4. **Relative imports**: Resolve to absolute during analysis
5. **Module side effects**: Preserve via topological sort

### AST Range Preservation

The existing `ast_node_renames` map is correct. The executor should:

1. Check the map for each AST node's (ModuleId, TextRange)
2. Use the mapped name if present
3. Use original text if not

## Immediate Action Items

### 1. Implement PotentialExportsMap (HIGH PRIORITY)

```rust
// Add to analysis phase
pub struct PotentialModuleExports {
    pub symbols: FxHashMap<String, GlobalBindingId>,
    pub all_declaration: Option<Vec<String>>,
}
```

### 2. Refactor BundlePlan Structure (HIGH PRIORITY)

- Add `FinalBundleLayout` struct
- Deprecate `execution_plan: Vec<ExecutionStep>`
- Update plan builder to populate new structure

### 3. Implement ImportAction Classification (HIGH PRIORITY)

- Replace ImportCategory logic
- Add import action determination to resolver
- Handle all import variants correctly

### 4. Add Namespace Population Step

```rust
pub struct NamespacePopulationStep {
    pub target_namespace: String,
    pub exports_to_assign: Vec<(String, String)>,
}
```

## Testing Strategy

### Critical Test Cases

1. Circular imports with namespace objects
2. Mixed import styles in same module
3. Star imports with `__all__`
4. Conditional imports (TYPE_CHECKING)
5. Package relative imports
6. Function-scoped imports
7. Side effect ordering

### Validation Approach

- Each phase should have unit tests
- Integration tests for complete flow
- Snapshot tests for deterministic output
- Real-world fixture tests

## Performance Considerations

### SimpleNamespace Overhead

- Start with SimpleNamespace for all direct imports
- Profile performance impact
- Only optimize if measurably significant
- Potential optimization: direct inlining for constant-only modules

## Error Handling

Implement diagnostics collection:

```rust
pub struct AnalysisOutput {
    pub bundle_plan: Option<BundlePlan>,
    pub diagnostics: Vec<Diagnostic>,
}
```

Don't fail on first error - collect all diagnostics for better UX.

## Next Sprint Planning

**Week 1**: Foundation

- [ ] Implement multi-pass analysis structure
- [ ] Create PotentialExportsMap generation
- [ ] Design FinalBundleLayout struct

**Week 2**: Core Implementation

- [ ] Implement ImportAction classification
- [ ] Build namespace creation/population logic
- [ ] Integrate with existing symbol renaming

**Week 3**: Edge Cases & Testing

- [ ] Handle all documented edge cases
- [ ] Comprehensive test suite
- [ ] Performance profiling
- [ ] Documentation updates
