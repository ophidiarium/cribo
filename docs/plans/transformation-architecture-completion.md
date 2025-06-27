# Transformation Architecture Completion Plan

## Status

The transformation architecture is partially implemented. Detection phase works well, but execution phase needs completion. This plan outlines the remaining work to fully realize the architecture.

## Current State

### Working Components

1. **TransformationMetadata enum**: Defined with all 5 transformation types
2. **TransformationDetector**: Successfully identifies transformations
3. **Basic execution**: RemoveImport and PartialImportRemoval work
4. **AST indexing**: NodeIndex system provides stable addressing

### Missing Components

1. **Reference tracking**: SemanticModelProvider lacks `get_references()`
2. **Granular addressing**: Transformation map uses `(ModuleId, ItemId)` instead of `NodeIndex`
3. **Symbol rewriting**: Detected but not executed
4. **Stdlib rewriting**: Only creates import, doesn't rewrite usages
5. **Circular dep moves**: TODO placeholder

## Implementation Plan

### Phase 1: Foundation Enhancements

#### 1.1 Enhance SemanticModelProvider

**Goal**: Enable tracking of all symbol usages

**Changes**:

```rust
// In semantic_model.rs
pub struct SemanticModel {
    // ... existing fields ...
    /// Map from binding to all its reference locations
    references: FxHashMap<GlobalBindingId, Vec<NodeIndex>>,
}

impl SemanticModel {
    /// Get all references to a binding
    pub fn get_references(&self, binding_id: GlobalBindingId) -> &[NodeIndex] {
        self.references
            .get(&binding_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
```

**Implementation**:

- Update semantic analysis visitor
- When resolving `Expr::Name`, record its NodeIndex
- Build references map during analysis

#### 1.2 Refactor Transformation Map

**Goal**: Enable precise, per-statement transformations

**Changes**:

```rust
// In analysis/mod.rs
pub struct AnalysisResults {
    // OLD: transformations: FxHashMap<(ModuleId, ItemId), Vec<TransformationMetadata>>,
    // NEW:
    transformations: FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
}
```

**Updates needed**:

- TransformationDetector: Use NodeIndex from items
- BundleCompiler: Query by NodeIndex
- Handle compound statements (if blocks, etc.)

#### 1.3 Update TransformationMetadata

**Goal**: Use NodeIndex for precise addressing

**Changes**:

```rust
// In transformations.rs
pub enum TransformationMetadata {
    SymbolRewrite {
        // OLD: rewrites: FxHashMap<TextRange, SymbolTransform>,
        // NEW:
        rewrites: FxHashMap<NodeIndex, String>,
    },

    CircularDepImportMove {
        target_scope: ItemId,
        // OLD: import_stmt: Option<Stmt>,
        // NEW:
        import_data: ImportData,
    },
}

pub struct ImportData {
    pub module: String,
    pub names: Vec<(String, Option<String>)>,
    pub level: u32,
}
```

### Phase 2: AST Transformation Infrastructure

#### 2.1 Implement AST Transformer

**Goal**: Apply transformations to AST and render to code

**New file**: `crates/cribo/src/ast_transformer.rs`

```rust
use ruff_python_ast::{Stmt, visitor::transformer::Transformer};
use rustc_hash::FxHashMap;

pub struct AstTransformer<'a> {
    transformations: &'a FxHashMap<NodeIndex, Vec<TransformationMetadata>>,
    current_module: ModuleId,
}

impl<'a> AstTransformer<'a> {
    pub fn transform_and_render(&self, stmt: &Stmt) -> Option<String> {
        // 1. Check for transformations on this statement
        let node_index = stmt.node_index();

        if let Some(transforms) = self.transformations.get(&node_index) {
            // Apply transformations in priority order
            for transform in transforms {
                match transform {
                    TransformationMetadata::RemoveImport { .. } => {
                        return None; // Skip this statement
                    }
                    TransformationMetadata::StdlibImportRewrite { .. } => {
                        // Generate new import statement
                        let new_stmt = self.create_stdlib_import(transform);
                        return Some(self.render_statement(&new_stmt));
                    } // ... handle other transformations
                }
            }
        }

        // For compound statements, recursively transform children
        if let Stmt::If(if_stmt) = stmt {
            return Some(self.transform_if_statement(if_stmt));
        }

        // Default: render as-is with symbol rewrites applied
        Some(self.render_with_rewrites(stmt))
    }
}
```

#### 2.2 Add ExecutionStep Variant

**Goal**: Support rendered code insertion

**Changes** in `bundle_compiler/compiler.rs`:

```rust
pub enum ExecutionStep {
    // ... existing variants ...
    /// Insert fully rendered code from AST transformation
    InsertRenderedCode {
        source_module: ModuleId,
        original_item_id: ItemId,
        code: String,
    },
}
```

**VM Update** in `bundle_vm.rs`:

```rust
ExecutionStep::InsertRenderedCode { code, .. } => {
    output.push_str(&code);
    output.push('\n');
}
```

### Phase 3: Complete Transformation Implementations

#### 3.1 SymbolRewrite Execution

**Goal**: Apply symbol rewrites during AST transformation

**Implementation**:

1. Collect all SymbolRewrite transformations for current module
2. During AST walking, check each Name node's index
3. Replace with new name if found in rewrites map
4. Handle Name → Attribute transformations

#### 3.2 StdlibImportRewrite Full Implementation

**Goal**: Normalize imports AND rewrite all usages

**Detection Enhancement**:

```rust
// In transformation_detector.rs
fn detect_stdlib_normalization(&self, ...) {
    // 1. Create StdlibImportRewrite for the import
    transformations.push(TransformationMetadata::StdlibImportRewrite { ... });
    
    // 2. Find all symbol usages
    for (symbol, canonical) in &symbols {
        let binding_id = self.resolve_symbol(module_id, symbol);
        let references = self.semantic_provider.get_references(binding_id);
        
        // 3. Create SymbolRewrite for each usage
        let mut rewrites = FxHashMap::default();
        for &node_index in references {
            rewrites.insert(node_index, canonical.clone());
        }
        
        if !rewrites.is_empty() {
            transformations.push(TransformationMetadata::SymbolRewrite { rewrites });
        }
    }
}
```

#### 3.3 CircularDepImportMove Implementation

**Goal**: Move imports to break circular dependencies

**Detection**:

```rust
// In transformation_detector.rs
fn detect_circular_moves(&self, ...) {
    // For each circular dependency group
    for group in &circular_deps {
        // Find imports that can be moved
        for import in self.find_movable_imports(group) {
            // Determine target function/class
            if let Some(target) = self.find_move_target(&import) {
                let import_data = self.extract_import_data(&import);

                transformations.insert(
                    import.node_index,
                    vec![TransformationMetadata::CircularDepImportMove {
                        target_scope: target,
                        import_data,
                    }],
                );
            }
        }
    }
}
```

**Execution**:

```rust
// In ast_transformer.rs
fn handle_circular_move(&self, transform: &TransformationMetadata) {
    // 1. Original import location: return None (remove it)
    // 2. Store import_data for later insertion
    // 3. When transforming target function, insert at beginning of body
}
```

### Phase 4: Integration and Cleanup

#### 4.1 Update BundleCompiler Main Loop

**Goal**: Use AST transformation for all items

```rust
// In bundle_compiler.rs
fn process_module_items(&mut self, module_id: ModuleId) {
    let transformer = AstTransformer::new(&self.transformations, module_id);

    for item_id in &self.live_items[&module_id] {
        let item_data = &self.graph.modules[&module_id].items[&item_id];

        // Get the AST for this item
        if let Some(ast) = self.get_item_ast(module_id, item_id) {
            // Transform and render
            if let Some(code) = transformer.transform_and_render(&ast) {
                self.steps.push(ExecutionStep::InsertRenderedCode {
                    source_module: module_id,
                    original_item_id: *item_id,
                    code,
                });
            }
        }
    }
}
```

#### 4.2 Remove Old Mutation Code

**Goal**: Clean up superseded code

**Files to update**:

- `stdlib_normalization.rs`: Convert to analysis-only or remove
- `import_rewriter.rs`: Convert to analysis-only or remove
- Remove mutation calls from orchestrator
- Update tests to verify via snapshots

### Phase 5: Testing Strategy

#### 5.1 Unit Tests

**TransformationDetector**:

- Test each detection pattern
- Verify correct TransformationMetadata generation
- Test edge cases (nested imports, conditional imports)

**AstTransformer**:

- Test each transformation type
- Test compound statement handling
- Test priority ordering

#### 5.2 Integration Tests

**Snapshot tests**:

- All existing snapshots must pass unchanged
- Add new fixtures for complex transformations
- Test circular dependency resolution

**End-to-end tests**:

- Test complete bundling with all transformations
- Verify bundled code executes correctly
- Performance benchmarks

## Success Criteria

1. **All transformations execute**: No more "detected but not implemented"
2. **Tests pass**: All existing tests continue to pass
3. **Clean architecture**: Clear separation between phases
4. **No AST mutations**: All transformations via new architecture
5. **Performance neutral**: No significant performance regression

## Risk Mitigation

### Complexity Risk

- Break implementation into small PRs
- Each transformation type can be completed independently
- Maintain backwards compatibility during transition

### Performance Risk

- Profile AST transformation and rendering
- Cache rendered code where possible
- Consider lazy transformation for large modules

### Testing Risk

- Extensive snapshot testing catches regressions
- Unit tests for each component
- Integration tests for complex scenarios
