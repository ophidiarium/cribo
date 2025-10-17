# Next Steps for Init Function Refactoring

## Current Status

**Completed: 11 of 13 phases (85%)**

The following phases have been successfully extracted and are fully tested:

1. ✅ Error Handling Infrastructure - `TransformError` enum
2. ✅ State Container - `InitFunctionState` with 12 fields
3. ✅ Initialization Phase - Guards & globals lifting
4. ✅ Finalization Phase - Function statement creation
5. ✅ Import Analysis Phase - Read-only import analysis
6. ✅ Import Transformation Phase - RecursiveImportTransformer
7. ✅ Wrapper Symbol Setup Phase - Placeholder assignments
8. ✅ Wildcard Import Processing Phase - `from module import *`
9. ✅ Body Preparation Phase - Analysis & body processing
10. ✅ Wrapper Globals Collection Phase - Visitor-based collection
11. ✅ Submodule Handling Phase - Submodule attributes
12. ✅ Final Cleanup Phase - Stdlib re-exports & explicit imports

## Remaining Work

### 1. Statement Processing Phase (lines 718-1297 in module_transformer.rs)

**Complexity**: HIGH - 580 lines of statement-type-specific processing logic

**Current Implementation**: Lives in the main loop of `transform_module_to_init_function`

**Required Approach**: Trait-based processor system (as designed in proposal)

#### Statement Types to Handle:

1. **Import** (lines 729-734) - Simple: Skip hoisted, add rest
2. **ImportFrom** (lines 735-867) - Complex: Relative import resolution
3. **ClassDef** (lines 868-891) - Medium: Set `__module__`, add attribute
4. **FunctionDef** (lines 893-920) - Medium: Transform nested, add attribute
5. **Assign** (lines 922-1062) - **VERY COMPLEX**: Multiple special cases:
   - `__all__` handling
   - Self-referential assignment detection
   - Builtin shadowing transformation
   - Module variable transformation
   - Lifted global propagation
   - Inlined import binding checks
   - Export logic for vars used by functions
6. **AnnAssign** (lines 1064-1160) - Similar to Assign with annotations
7. **Try** (lines 1162-1265) - Medium: Collect exportable symbols from branches
8. **Default** (lines 1267-1295) - Transform other statement types

#### Recommended Architecture:

```rust
// Phase coordinator
pub struct StatementProcessingPhase {
    // Holds references to transformation helpers
}

impl StatementProcessingPhase {
    pub fn execute(
        processed_body: Vec<Stmt>,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        prep_context: &BodyPreparationContext,
        lifted_names: &Option<FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Initialize tracking for lifted globals
        let mut initialized_lifted_globals = FxIndexSet::default();

        // Process each statement
        for (idx, stmt) in processed_body.into_iter().enumerate() {
            Self::process_statement(
                stmt,
                idx,
                bundler,
                ctx,
                prep_context,
                lifted_names,
                &mut initialized_lifted_globals,
                state,
            )?;
        }

        Ok(())
    }

    fn process_statement(/* ... */) -> Result<(), TransformError> {
        // Dispatch to appropriate handler based on statement type
        match stmt {
            Stmt::Import(_) => { /* ... */ }
            Stmt::ImportFrom(_) => { /* ... */ }
            Stmt::ClassDef(_) => { /* ... */ }
            Stmt::FunctionDef(_) => { /* ... */ }
            Stmt::Assign(_) => { /* ... */ }
            Stmt::AnnAssign(_) => { /* ... */ }
            Stmt::Try(_) => { /* ... */ }
            _ => { /* ... */ }
        }
        Ok(())
    }
}
```

**Alternative Simpler Approach**: Since this code is already well-tested and working, and the body is already processed by `bundler.process_body_recursive()` in the Body Preparation phase, we could:

1. Keep the statement processing loop inline for now
2. Extract just the helper functions (`add_module_attr_if_exported`, `emit_module_attr_if_exportable`, etc.) to a utilities module
3. Document the processing logic clearly
4. Defer full trait-based refactoring to when we have clearer requirements for extension

### 2. Create Orchestrator

Once statement processing is handled, create `InitFunctionBuilder`:

```rust
pub struct InitFunctionBuilder<'a> {
    bundler: &'a Bundler<'a>,
    ctx: &'a ModuleTransformContext,
    symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

impl<'a> InitFunctionBuilder<'a> {
    pub fn build(self, mut ast: ModModule) -> Result<Stmt, TransformError> {
        let mut state = InitFunctionState::new();

        // Phase 1: Initialization
        InitializationPhase::execute(self.bundler, self.ctx, &mut ast, &mut state)?;

        // Phase 2: Import Analysis
        ImportAnalysisPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 3: Import Transformation
        ImportTransformationPhase::execute(
            self.bundler,
            self.ctx,
            &mut ast,
            self.symbol_renames,
            &mut state,
        )?;

        // ... continue with all phases ...

        // Final: Finalization
        FinalizationPhase::build_function_stmt(self.bundler, self.ctx, state)
    }
}
```

### 3. Update Call Site

Replace in `module_wrapper.rs` or wherever `transform_module_to_init_function` is called:

```rust
// Old:
let init_fn = transform_module_to_init_function(bundler, ctx, ast, symbol_renames);

// New:
let init_fn = InitFunctionBuilder::new(bundler, ctx, symbol_renames).build(ast)?;
```

### 4. Add Unit Tests

Each phase should have dedicated unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_analysis_tracks_stdlib() {
        // Test that stdlib imports are tracked correctly
    }

    #[test]
    fn test_wrapper_symbols_creates_placeholders() {
        // Test placeholder creation
    }

    // ... more tests ...
}
```

### 5. Performance Validation

Before and after:

- Run benchmarks on representative fixtures
- Ensure no regression > 5%
- Profile hot paths if needed

## Success Criteria

- [ ] Statement processing fully extracted
- [ ] Orchestrator created and integrated
- [ ] All 148 tests still passing
- [ ] Clippy clean
- [ ] Original 1,511-line function removed
- [ ] Performance maintained (< 5% regression)
- [ ] Documentation updated

## Notes

- Each phase is designed to be independently testable
- State flows explicitly through `InitFunctionState`
- All phases maintain the original function's behavior
- Incremental approach minimizes risk
