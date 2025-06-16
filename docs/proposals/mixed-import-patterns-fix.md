# Proposal: Complete Fix for Mixed Import Patterns with Circular Dependencies

## Executive Summary

This proposal outlines a comprehensive solution for handling Python modules with circular dependencies that use mixed import patterns (some imports at module level, others inside functions). The current implementation has made progress but still fails when imports are moved to function scope but used in class initializers.

## Problem Statement

### Current Issue

The `mixed_import_patterns` test fixture demonstrates a common Python pattern where:

1. Module `config` imports `logger` at module level and uses it in `Config.__init__`
2. Module `logger` imports `config` inside a function to avoid circular dependency
3. The import rewriter moves `config`'s import of `logger` to function scope
4. This causes `NameError` when `Config.__init__` tries to use `get_logger`

### Root Causes

1. **Timing Mismatch**: Import rewriting happens before module classification (inline vs wrapper)
2. **Incomplete Usage Analysis**: Import usage in class initializers isn't properly detected as module-level usage
3. **Lack of Coordination**: Import rewriter and code generator make independent decisions

## Design Options

### Option 1: Enhanced Import Usage Analysis (Recommended)

**Approach**: Improve import usage detection to identify when imports are used in contexts that execute at module initialization time.

**Components**:

1. **Enhanced Usage Visitor**: Extend import usage analysis to track:
   - Usage in class `__init__` methods
   - Usage in class body assignments
   - Usage in decorator expressions
   - Usage in default parameter values

2. **Import Rewriter Integration**: Modify import rewriter to:
   - Check if import is used in module-initialization contexts
   - Preserve module-level imports that have such usage
   - Only move imports that are truly function-scoped

**Implementation**:

```rust
// In cribo_graph or new visitor
struct ImportUsageContext {
    pub is_module_level: bool,
    pub is_class_init: bool,
    pub is_decorator: bool,
    pub is_default_param: bool,
}

impl ImportUsageAnalyzer {
    fn analyze_import_usage(&self, import: &ImportInfo) -> ImportUsageContext {
        // Visit AST and track where import is used
        // Return context indicating if it's safe to move to function scope
    }
}
```

**Pros**:

- Addresses root cause of incorrect import movement
- Maintains existing architecture
- More accurate import analysis benefits other features

**Cons**:

- Requires deeper AST analysis
- May increase compilation time

### Option 2: Two-Phase Module Classification

**Approach**: Determine module classification (inline/wrapper) before import rewriting.

**Components**:

1. **Early Classification**: Analyze modules to determine wrapper requirements:
   - Has side effects
   - Imports other first-party modules
   - Is imported directly or as namespace

2. **Classification-Aware Import Rewriting**: Only rewrite imports for modules that will be inlined

**Implementation**:

```rust
// In orchestrator
let module_classifications = self.classify_modules(&parsed_modules)?;

// Pass classifications to import rewriter
let movable_imports = import_rewriter.analyze_movable_imports(
    graph, 
    &resolvable_cycles,
    &module_classifications  // New parameter
);
```

**Pros**:

- Prevents import rewriting for wrapper modules
- Cleaner separation of concerns

**Cons**:

- Requires significant refactoring
- May duplicate some analysis work

### Option 3: Lazy Import Transformation

**Approach**: Generate import code that works regardless of scope.

**Components**:

1. **Import Helper Functions**: Generate helper functions for each import:
   ```python
   def __cribo_import_get_logger():
       if 'get_logger' not in globals():
           from logger import get_logger as _get_logger
           globals()['get_logger'] = _get_logger
       return globals()['get_logger']
   ```

2. **Usage Transformation**: Replace `get_logger()` with `__cribo_import_get_logger()()`

**Pros**:

- Works in any context
- No need for complex usage analysis

**Cons**:

- Generated code is less readable
- Performance overhead from helper calls
- More complex AST transformations

### Option 4: Hybrid Module Initialization

**Approach**: Allow modules to have both initialization-time and runtime imports.

**Components**:

1. **Split Import Lists**: Maintain two import lists per module:
   - Initialization imports (executed at module init)
   - Runtime imports (moved to functions)

2. **Smart Import Placement**: Based on usage analysis, place imports appropriately

**Pros**:

- Flexible approach
- Minimal changes to existing code

**Cons**:

- Increases complexity
- May lead to duplicate imports

### Option 5: Semantic Analysis Integration with ruff_python_semantic

**Approach**: Leverage ruff_python_semantic's comprehensive semantic analysis capabilities through the existing semantic_bundler infrastructure.

**Components**:

1. **Enhanced Semantic Model**: Extend `semantic_bundler.rs` to track:
   - Import usage contexts (module-level vs function-level)
   - Execution timing of symbol references
   - Class initialization dependencies

2. **Semantic-Aware Visitor**: Create visitor that uses ruff_python_semantic's:
   - Scope analysis to understand execution contexts
   - Binding analysis to track import usage
   - Flow analysis to determine initialization-time dependencies

3. **Integration Points**:
   ```rust
   // In semantic_bundler.rs
   impl SemanticBundler {
       pub fn analyze_import_execution_context(
           &self,
           import_id: ImportId,
           module_id: ModuleId,
       ) -> ImportExecutionContext {
           let semantic_model = self.get_semantic_model(module_id);
           let binding = semantic_model.binding(import_id);

           // Use ruff_python_semantic to determine if binding is used
           // in module initialization context
           let usage_contexts = binding
               .references()
               .map(|ref_id| self.analyze_reference_context(ref_id))
               .collect();

           ImportExecutionContext {
               is_module_init_time: self.any_init_time_usage(&usage_contexts),
               is_class_body: self.any_class_body_usage(&usage_contexts),
               is_function_only: self.all_function_scoped(&usage_contexts),
           }
       }
   }
   ```

4. **Visitor Implementation**:
   ```rust
   // New visitor using semantic information
   struct SemanticImportUsageVisitor<'a> {
       semantic_bundler: &'a SemanticBundler,
       module_id: ModuleId,
   }

   impl<'a> Visitor<'_> for SemanticImportUsageVisitor<'a> {
       fn visit_stmt(&mut self, stmt: &Stmt) {
           if let Stmt::ImportFrom(import) = stmt {
               let import_id = self.semantic_bundler.get_import_id(import);
               let context = self
                   .semantic_bundler
                   .analyze_import_execution_context(import_id, self.module_id);

               // Store context for import rewriter
               self.record_import_context(import, context);
           }
           walk_stmt(self, stmt);
       }
   }
   ```

**Pros**:

- Leverages battle-tested semantic analysis from Ruff
- More accurate than custom AST traversal
- Handles complex Python semantics correctly
- Integrates with existing semantic infrastructure
- Benefits from Ruff's ongoing improvements

**Cons**:

- Requires deeper understanding of ruff_python_semantic APIs
- May need to extend semantic_bundler interface
- Potential version coupling with Ruff crates

## Recommended Solution

After careful consideration, **Option 5 (Semantic Analysis Integration)** is recommended as the primary approach, with **Option 1 (Enhanced Import Usage Analysis)** as a fallback if semantic integration proves too complex.

**Option 5 is preferred because it**:

1. Leverages Ruff's proven semantic analysis capabilities
2. Provides the most accurate understanding of Python semantics
3. Integrates with our existing semantic_bundler infrastructure
4. Handles edge cases that custom analysis might miss
5. Benefits from ongoing improvements in the Ruff project

## Implementation Plan

### Phase 1: Semantic Analysis Integration

1. Extend `semantic_bundler.rs` with import execution context analysis:
   - Add `ImportExecutionContext` struct
   - Implement `analyze_import_execution_context` method
   - Create helper methods for context detection
2. Study ruff_python_semantic APIs for:
   - Scope and binding analysis
   - Reference tracking
   - Execution context determination

### Phase 2: Semantic-Aware Import Analysis

1. Create `SemanticImportUsageVisitor` that uses semantic_bundler
2. Integrate visitor with import discovery pipeline
3. Modify `cribo_graph` to store execution context information

### Phase 3: Import Rewriter Integration

1. Update `ImportRewriter::analyze_movable_imports` to:
   - Query semantic execution context for each import
   - Preserve imports with module-initialization usage
   - Only move truly function-scoped imports
2. Add configuration for semantic-based decisions

### Phase 4: Testing and Validation

1. Ensure `mixed_import_patterns` test passes
2. Add comprehensive test cases:
   - Imports used in class `__init__` methods
   - Imports used in decorators
   - Imports used in default parameters
   - Imports used in class attributes
   - Complex nested usage patterns
3. Verify no regressions in existing tests
4. Performance benchmarking to ensure acceptable overhead

## Success Criteria

1. `mixed_import_patterns` test passes without errors
2. No regression in existing test suite
3. Generated code remains readable and efficient
4. Performance impact is minimal (<5% increase in bundling time)
5. Semantic analysis correctly identifies all module-initialization contexts

## Risks and Mitigations

1. **Risk**: Complex AST analysis may have edge cases
   - **Mitigation**: Comprehensive test suite covering various import patterns

2. **Risk**: Performance impact from deeper analysis
   - **Mitigation**: Use caching and optimize visitor traversal

3. **Risk**: Breaking changes to existing behavior
   - **Mitigation**: Extensive regression testing
