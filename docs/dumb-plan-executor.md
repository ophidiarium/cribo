# Dumb Plan Executor Design & Implementation Plan

## Executive Summary

The Dumb Plan Executor represents a fundamental architectural shift in Cribo's code generation approach. Instead of making bundling decisions during code generation, all decisions are made during the analysis phase and stored in the BundlePlan. The executor simply follows these pre-computed instructions without any decision-making logic.

## Problem Statement

The current code generator suffers from:

1. **Architectural Violation**: Makes bundling decisions during generation phase
2. **Side-Channel Communication**: Relies on implicit state like `namespace_imported_modules`
3. **Tangled Logic**: Mixes decision-making with code generation
4. **Maintenance Burden**: Complex heuristics scattered throughout the code
5. **Correctness Issues**: Re-exports and wrapped modules not properly instantiated

## Design Principles

### 1. Separation of Concerns

- **Analysis Phase**: All decisions about what, how, and when to bundle
- **Execution Phase**: Mechanical translation of decisions into Python AST

### 2. Declarative Over Imperative

- BundlePlan contains declarative instructions
- Executor interprets instructions without conditional logic

### 3. Single Source of Truth

- BundlePlan is the only source of bundling decisions
- No hidden state or side channels

### 4. Progressive Enhancement

- Start with minimal V1 that handles basic cases
- Add complexity only when BundlePlan provides the data

## Architecture Overview

```
┌─────────────────┐     ┌──────────────┐     ┌─────────────────┐
│ Analysis Phase  │────▶│  BundlePlan  │────▶│ Dumb Executor   │
│                 │     │              │     │                 │
│ - Tree Shaking  │     │ - Statement  │     │ - Read Plan     │
│ - Conflict Res  │     │   Order      │     │ - Apply AST     │
│ - Symbol Origin │     │ - Renames    │     │   Transforms    │
│ - Import Hoist  │     │ - Hoisted    │     │ - Emit Code     │
│                 │     │   Imports    │     │                 │
└─────────────────┘     └──────────────┘     └─────────────────┘
```

## BundlePlan Enhancements

### New Fields Required

```rust
pub struct BundlePlan {
    // Existing fields...
    /// Hoisted imports in declarative form (not AST nodes)
    pub hoisted_imports: Vec<HoistedImport>,

    /// Primary driver for the executor - granular execution steps
    pub execution_plan: Vec<ExecutionStep>,
}

#[derive(Debug, Clone)]
pub enum HoistedImport {
    Future(String), // e.g., "annotations"
    Stdlib(String), // e.g., "json"
}

#[derive(Debug, Clone)]
pub enum ExecutionStep {
    /// Hoist a `from __future__ import ...` statement
    HoistFutureImport { name: String },

    /// Hoist a standard library import
    HoistStdlibImport { name: String },

    /// Define the init function for a wrapped module
    DefineInitFunction { module_id: ModuleId },

    /// Create the module object by calling its init function
    CallInitFunction {
        module_id: ModuleId,
        target_variable: String,
    },

    /// Directly inline a statement from a source module
    InlineStatement {
        module_id: ModuleId,
        item_id: ItemId,
    },
}

// Enhanced module metadata for executor
#[derive(Debug, Clone)]
pub enum ModuleInstantiation {
    Inline, // Default: statements inserted directly
    Wrap {
        init_function_name: String,
        exports: Vec<String>, // Pre-computed by analysis
    },
}

#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub instantiation: ModuleInstantiation,
    pub has_side_effects: bool,
    // ... other metadata
}
```

## Implementation Phases

### Phase 1: Minimal V1 Implementation (MVP)

**Goal**: Replace current generator for basic bundling without wrapped modules

**Components**:

1. Create `dumb_executor.rs` with `generate_bundle_v1` function
2. Enhance BundlePlan with `hoisted_future_imports` and `hoisted_stdlib_imports`
3. Simple statement emission following `final_statement_order`
4. Apply `ast_node_renames` during emission

**Code Structure**:

```rust
pub fn generate_bundle_v1(
    plan: &BundlePlan,
    source_asts: &HashMap<ModuleId, ModModule>,
) -> Result<ModModule> {
    let mut final_body = Vec::new();

    // Process execution steps in order - no decision making!
    for step in &plan.execution_plan {
        match step {
            ExecutionStep::HoistFutureImport { name } => {
                let stmt = generate_future_import(name);
                final_body.push(stmt);
            }
            ExecutionStep::HoistStdlibImport { name } => {
                let stmt = generate_stdlib_import(name);
                final_body.push(stmt);
            }
            ExecutionStep::InlineStatement { module_id, item_id } => {
                let stmt = get_statement(source_asts, *module_id, *item_id)?;
                let renamed_stmt = apply_ast_renames(stmt, plan, *module_id);
                final_body.push(renamed_stmt);
            }
            ExecutionStep::DefineInitFunction { .. } | ExecutionStep::CallInitFunction { .. } => {
                // Phase 2 - skip for now
                continue;
            }
        }
    }

    Ok(ModModule {
        body: final_body,
        range: TextRange::default(),
    })
}
```

### Phase 2: Wrapped Module Support

**Goal**: Handle modules with side effects via init functions

**Components**:

1. Add `module_instantiation_order` to BundlePlan
2. Generate init functions for wrapped modules
3. Call init functions in correct order
4. Update namespace assignments

**Enhancements**:

- Analysis phase determines which modules need wrapping
- Analysis phase computes instantiation order
- Executor mechanically generates init functions and calls

### Phase 3: ExecutionStep Integration

**Goal**: Make generation fully declarative

**Components**:

1. Convert BundlePlan fields into ExecutionStep sequence
2. Simple interpreter loop over steps
3. Remove all conditional logic from executor

**Final Structure**:

```rust
pub fn generate_bundle_v2(
    plan: &BundlePlan,
    source_asts: &HashMap<ModuleId, ModModule>,
) -> Result<ModModule> {
    let mut final_body = Vec::new();

    // Stateless execution - just follow the plan!
    for step in &plan.execution_plan {
        let stmt = execute_step(step, plan, source_asts)?;
        final_body.push(stmt);
    }

    Ok(ModModule {
        body: final_body,
        range: TextRange::default(),
    })
}

// Pure function - no state, no decisions
fn execute_step(
    step: &ExecutionStep,
    plan: &BundlePlan,
    source_asts: &HashMap<ModuleId, ModModule>,
) -> Result<Stmt> {
    match step {
        ExecutionStep::HoistFutureImport { name } => Ok(generate_future_import(name)),
        ExecutionStep::HoistStdlibImport { name } => Ok(generate_stdlib_import(name)),
        ExecutionStep::DefineInitFunction { module_id } => {
            let metadata = plan
                .module_metadata
                .get(module_id)
                .expect("Plan referenced module without metadata");
            Ok(generate_init_function(module_id, metadata, source_asts)?)
        }
        ExecutionStep::CallInitFunction {
            module_id,
            target_variable,
        } => Ok(generate_init_call(module_id, target_variable)),
        ExecutionStep::InlineStatement { module_id, item_id } => {
            let stmt = get_statement(source_asts, *module_id, *item_id)?;
            Ok(apply_ast_renames(stmt, plan, *module_id))
        }
    }
}
```

## Migration Strategy

### 1. Immediate Switch - No Parallel Implementation

- **Replace code_generator.rs entirely** - no feature flags, no dual maintenance
- The dumb executor becomes the only implementation from day one
- This forces us to address all issues immediately rather than deferring them

### 2. Progressive Testing with xfail Strategy

- **Rename all bundling test fixtures** with `xfail_` prefix initially
- As the dumb executor gains capabilities, tests will start passing
- When a test passes, it will fail with "xfail test unexpectedly passed"
- Remove the `xfail_` prefix to convert it to a regular test
- This provides clear visibility of progress and prevents regressions

**Example progression:**

```
Day 1: All tests are xfail_*
xfail_simple_inline/main.py
xfail_stdlib_imports/main.py
xfail_wrapped_modules/main.py
xfail_circular_deps/main.py

Day 3: Simple cases work
simple_inline/main.py ✓
stdlib_imports/main.py ✓
xfail_wrapped_modules/main.py
xfail_circular_deps/main.py

Day 7: More complex cases
simple_inline/main.py ✓
stdlib_imports/main.py ✓
wrapped_modules/main.py ✓
xfail_circular_deps/main.py
```

### 3. Implementation Order

1. **Start with simplest fixtures** that only need basic inlining
2. **Add stdlib import hoisting** to handle import normalization
3. **Implement wrapped modules** with @functools.cache
4. **Handle complex cases** like circular dependencies and re-exports
5. **Edge cases last** - conditional imports, dynamic **all**, etc.

### 4. Analysis Phase Updates

- Update all analysis passes to populate `execution_plan`
- Remove any code that assumes old generator behavior
- Ensure every decision is captured in BundlePlan

## Testing Strategy

### Unit Tests

1. Test each ExecutionStep type independently
2. Test AST transformation utilities
3. Test init function generation

### Integration Tests

1. Use existing bundling fixtures
2. Compare output between old and new generators
3. Ensure functional equivalence

### Regression Prevention

1. Snapshot tests for generated code
2. Execution tests for bundled output
3. Performance benchmarks

## Key Architectural Decisions

Based on deep analysis, these decisions are final:

### 1. **name** Handling

- **Analysis Phase**: Pre-compute correct `__name__` values based on module instantiation type
- **BundlePlan**: Add entries to `ast_node_renames` for wrapped modules
- **Executor**: Simple AST node replacement, no logic

### 2. Module Initialization

- **Use @functools.cache**: Thread-safe, concise, declarative
- **Analysis Phase**: Ensure `functools` import is hoisted when needed
- **Executor**: Mechanically add decorator to init functions

### 3. Error Handling

- **Executor assumes BundlePlan is correct**: Use `expect()` liberally
- **Validation**: Optional debug-only `BundlePlan::validate()` method
- **Panics indicate planner bugs**: Fail loudly and early

### 4. **all** Resolution

- **Fully resolved during analysis**: No star imports in BundlePlan
- **ModuleMetadata contains exports list**: Pre-computed from usage analysis
- **Executor knows nothing about **all****: Just generates from exports list

### 5. State Management

- **Executor is stateless**: Each step is a pure function
- **No intermediate state**: All information in BundlePlan
- **Test individual steps**: Isolation enables easy debugging

## Success Criteria

### Correctness

- [ ] All existing tests pass with new executor
- [ ] Re-exports work correctly
- [ ] Wrapped modules properly instantiated
- [ ] No undefined symbol errors

### Architecture

- [ ] Zero decision-making in executor
- [ ] All logic in analysis phase
- [ ] Clean separation of concerns
- [ ] No side-channel communication

### Maintainability

- [ ] Reduced code complexity
- [ ] Clear data flow
- [ ] Easy to debug
- [ ] Simple to extend

## Implementation Timeline

### Phase 1: Foundation (Day 1-2)

- [ ] Rename all test fixtures with xfail_ prefix
- [ ] Create ExecutionStep enum in bundle_plan/mod.rs
- [ ] Update ModuleMetadata with ModuleInstantiation enum
- [ ] Replace code_generator.rs with plan_executor.rs
- [ ] Implement minimal generate_bundle_v1 for basic inlining
- [ ] Get first simple fixture passing (remove its xfail_ prefix)

### Phase 2: Core Features (Day 3-5)

- [ ] Add HoistedImport enum and import generation
- [ ] Update analysis to populate execution_plan
- [ ] Implement AST node renaming in executor
- [ ] Handle stdlib import hoisting
- [ ] Get 5-10 more fixtures passing

### Phase 3: Advanced Features (Day 6-8)

- [ ] Implement wrapped module support with @functools.cache
- [ ] Add DefineInitFunction and CallInitFunction steps
- [ ] Handle **name** rewriting for wrapped modules
- [ ] Resolve **all** in analysis phase
- [ ] Get wrapped module fixtures passing

### Phase 4: Complex Cases (Day 9-10)

- [ ] Handle circular dependencies
- [ ] Support re-exports and symbol origins
- [ ] Add BundlePlan::validate() for debugging
- [ ] Get remaining fixtures passing
- [ ] Remove old code_generator.rs code paths

## Risks & Mitigations

### Risk 1: Immediate Break of All Tests

**Mitigation**: xfail strategy allows controlled progression while maintaining CI green

### Risk 2: Missing Critical Functionality

**Mitigation**: Start with simplest cases, build incrementally with clear progress tracking

### Risk 3: Analysis Phase Gaps

**Mitigation**: Each failing test reveals what the analysis phase needs to provide

### Risk 4: No Fallback Path

**Mitigation**: Git history allows reverting if absolutely necessary, but commitment to new architecture prevents technical debt accumulation

## Future Enhancements

### Post-MVP Improvements

1. Optimize AST cloning/transformation
2. Streaming code generation
3. Parallel execution steps
4. Incremental bundling support

### Long-term Vision

- Pluggable execution backends
- Alternative output formats
- Cross-language bundling
- Build-time optimizations

## Benefits of Immediate Switch

### No Technical Debt Accumulation

- Avoids maintaining two implementations
- Forces immediate resolution of architectural issues
- Prevents "temporary" workarounds from becoming permanent

### Faster Iteration

- Direct feedback on what's broken
- No time wasted on compatibility layers
- Clear progress visibility through xfail conversions

### Cleaner Codebase

- No feature flags cluttering the code
- No abstraction layers for dual support
- Single, focused implementation

### Forced Completeness

- Can't defer hard problems
- Must handle all cases to ship
- Results in more robust final solution

## Conclusion

The Dumb Plan Executor with immediate switch strategy represents a bold but correct architectural decision. By:

1. Committing fully to the new architecture
2. Using xfail for controlled progression
3. Separating all decisions from execution
4. Making the executor truly stateless

We create a bundler that is not only more correct and maintainable but also easier to understand, test, and extend. The temporary pain of the switch is vastly outweighed by the long-term benefits of a clean, principled architecture.
