# Implementation Plan: Semantic Import Analysis for Mixed Import Patterns

## Problem Summary

The current implementation fails on mixed import patterns where modules have circular dependencies but the actual runtime execution order is well-defined. This happens when imports are used only within function bodies (deferred execution) vs at module level (immediate execution).

## Solution: Semantic Import Analysis (Option 5)

### Phase 1: Add Core Types for Semantic Analysis

1. **Create `semantic_analysis.rs`** module:
   - Define `ExecutionContext` enum
   - Define `ImportUsage` and `EnhancedImportInfo` structs
   - Add semantic analysis utilities

2. **Update `import_info.rs`**:
   - Add fields for tracking usage context
   - Add methods for semantic queries

### Phase 2: Implement Semantic AST Visitor

1. **Create `semantic_visitor.rs`**:
   - Implement context-aware AST visitor
   - Track execution contexts during traversal
   - Record where each import is actually used

2. **Key features**:
   - Push/pop context stack for nested scopes
   - Differentiate between runtime and deferred contexts
   - Track all name usages and resolve to imports

### Phase 3: Integrate with Import Extraction

1. **Update `extract_imports()`**:
   - Add two-pass analysis:
     - First pass: Collect imports (existing)
     - Second pass: Semantic usage analysis (new)
   - Return enhanced import information

2. **Modify `ExtractedModule`**:
   - Use `EnhancedImportInfo` instead of basic `ImportInfo`
   - Include semantic analysis results

### Phase 4: Update Dependency Graph

1. **Enhance `DependencyGraph`**:
   - Add `add_edge_with_context()` method
   - Filter edges based on runtime requirements
   - Add semantic-aware cycle detection

2. **Update `find_circular_dependencies()`**:
   - Create runtime-only subgraph
   - Run SCC algorithm only on runtime edges
   - Provide detailed diagnostics

### Phase 5: Update Bundling Logic

1. **Modify `bundle_modules()`**:
   - Use semantic information for ordering
   - Handle deferred-only imports specially
   - Generate correct module order

2. **Update code generation**:
   - Add comments indicating deferred imports
   - Optimize bundling based on usage patterns

### Phase 6: Testing

1. **Add test fixtures**:
   - `mixed_imports_simple`: Basic deferred import
   - `mixed_imports_complex`: Multiple levels
   - `mixed_imports_circular`: True circular dependency
   - `mixed_imports_class`: Class-level imports

2. **Update existing tests**:
   - Ensure backward compatibility
   - Verify no regressions

## Implementation Order

1. **Week 1**:
   - Implement core types and semantic visitor
   - Add basic usage tracking

2. **Week 2**:
   - Integrate with import extraction
   - Update dependency graph logic

3. **Week 3**:
   - Complete bundling integration
   - Add comprehensive tests

4. **Week 4**:
   - Performance optimization
   - Documentation and examples

## Risk Mitigation

1. **Performance**:
   - Two-pass analysis may be slower
   - Mitigation: Cache results, optimize visitor

2. **Complexity**:
   - More complex than current approach
   - Mitigation: Clear documentation, good tests

3. **Edge Cases**:
   - Dynamic imports, exec/eval
   - Mitigation: Conservative defaults, warnings

## Success Criteria

1. All existing tests pass
2. Mixed import pattern tests pass
3. No significant performance regression
4. Clear error messages for true circular dependencies
5. Documentation explains the semantic analysis

## Alternative Approaches Considered

1. **Option 1**: Simple deferred import tracking - Too simplistic
2. **Option 2**: Inline certain imports - Doesn't solve root cause
3. **Option 3**: Manual annotation - Poor developer experience
4. **Option 4**: Full semantic model - Over-engineered for our needs

## Conclusion

Option 5 (Semantic Import Analysis) provides the best balance of:

- Correctness: Properly handles execution contexts
- Performance: Minimal overhead with caching
- Maintainability: Clear separation of concerns
- User Experience: Works automatically without annotations

This approach is inspired by Ruff's semantic analysis but simplified for our specific use case of import bundling.
