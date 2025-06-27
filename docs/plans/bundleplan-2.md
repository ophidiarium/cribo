# BundlePlan Architecture Review - Phase 2

## Executive Summary

This document captures the findings from a comprehensive architectural review of the new BundlePlan-powered architecture in Cribo. The review was conducted using deep-code-reasoning analysis to validate the implementation against the original design goals and assess the naming conventions for clarity and future growth.

## Review Objectives

1. Validate that the current implementation accurately represents the intended architecture
2. Ensure that struct, enum, and method naming is unambiguous, intuitive, and provides good space for future growth
3. Identify any architectural issues or improvements needed

## Key Findings

### 1. Architectural Alignment ✅

The implementation successfully achieves the core architectural goals:

- **Separation of Concerns**: The `BundleCompiler` (intelligence) and `BundleVM` (execution) are properly separated
- **Stateful vs Stateless**: `BundleCompiler` correctly maintains all complex state, while `BundleVM` is a simple state machine
- **Clean Abstraction**: `ExecutionStep` enum provides a clean abstraction for bundle operations

**Evidence**:

- `BundleCompiler` contains all semantic analysis logic, symbol renaming, import classification, and tree-shaking decisions
- `BundleVM` simply iterates through `ExecutionStep`s without any complex decision-making
- The data flow matches the design: `AnalysisResults → BundleCompiler → BundleProgram → BundleVM`

### 2. Naming Convention Assessment

#### Strong Names ✅

- `BundleCompiler`: Accurately reflects its role as the "compiler" that transforms high-level analysis into low-level instructions
- `BundleVM`: Clear metaphor for a virtual machine that executes instructions
- `ExecutionStep`: Unambiguous name for the instruction units
- `BundleProgram`: Appropriate name for the compiled output (analogous to bytecode)

#### Confusion Points ⚠️

- **`BundlePlan` and `BundlePlanBuilder`**: These exist in the codebase but are not used in the main execution flow
- The design document (`execution-step-architecture.md`) makes no mention of `BundlePlan`
- This creates ambiguity about whether these are dead code or serve a different purpose

### 3. Architectural Issues Identified

#### Issue 1: Implicit Contract Between Compiler and VM (High Priority)

**Problem**: The contract between `BundleCompiler` and `BundleVM` is implicit and fragile.

```rust
pub struct BundleProgram {
    pub steps: Vec<ExecutionStep>,
    pub ast_node_renames: FxHashMap<(ModuleId, TextRange), String>,
}
```

The VM must know to correlate `ast_node_renames` with `CopyStatement` steps, creating tight coupling.

**Risk**: Changes in the compiler's output format can break the VM without compile-time errors.

#### Issue 2: Performance Inefficiency (Medium Priority)

**Problem**: `live_items` uses `Vec<ItemId>` causing O(n) lookups.

```rust
// Current (inefficient)
live_items: FxHashMap<ModuleId, Vec<ItemId>>

// Should be
live_items: FxHashMap<ModuleId, FxHashSet<ItemId>>
```

**Impact**: Slower compilation for projects with large modules.

#### Issue 3: Heuristic-Based Import Classification (Low Priority)

**Problem**: The `is_stdlib_module` function uses a hardcoded list to determine which modules are side-effect-free and can be hoisted.

**Risk**: Incorrect classification could alter program behavior by changing execution order.

## Action Plan

### Immediate Actions (Sprint 1)

1. **Fix Performance Issue**
   - Change `live_items` from `FxHashMap<ModuleId, Vec<ItemId>>` to `FxHashMap<ModuleId, FxHashSet<ItemId>>`
   - Update all code that interacts with `live_items` to use set operations
   - **Effort**: 1-2 hours

2. **Document the Compiler/VM Contract**
   - Add comprehensive documentation to `BundleProgram` struct
   - Explicitly define how `ast_node_renames` must be used by the VM
   - Document the relationship between `ExecutionStep` variants and other program data
   - **Effort**: 1-2 hours

3. **Resolve BundlePlan Ambiguity**
   - Investigate whether `BundlePlan` and `BundlePlanBuilder` are used anywhere
   - If dead code: Remove them entirely
   - If serving a purpose: Document their role and relationship to the main flow
   - **Effort**: 2-3 hours

### Medium-term Improvements (Sprint 2)

1. **Formalize the Compiler/VM Interface**
   - Consider embedding rename information directly into `ExecutionStep::CopyStatement`
   - Or introduce a versioning scheme for `BundleProgram` format
   - **Effort**: 4-6 hours

2. **Improve Import Classification**
   - Make the stdlib module list configurable
   - Allow users to mark specific third-party modules as side-effect-free
   - Add warnings when heuristics might affect behavior
   - **Effort**: 3-4 hours

### Long-term Considerations

1. **Extensibility**
   - The `ExecutionStep` enum is well-designed for adding new operations
   - Consider adding more specialized steps for common patterns

2. **Alternative VMs**
   - The architecture supports creating alternative VMs (debug VM with logging, optimized VM, etc.)
   - Consider formalizing a VM trait/interface

3. **Caching and Optimization**
   - The clean separation allows caching `BundleProgram` for faster rebuilds
   - Consider adding program serialization for build caches

## Naming Recommendations

### Keep As-Is

- `BundleCompiler` - Clear and accurate
- `BundleVM` - Good metaphor, well understood
- `ExecutionStep` - Unambiguous
- `BundleProgram` - Appropriate for the "compiled" output

### Consider Renaming/Removing

- `BundlePlan` → `BundleStrategy` or `BundleBlueprint` (if kept)
- `BundlePlanBuilder` → `BundleStrategyBuilder` (if kept)
- Or remove entirely if these are vestigial code

### Future-Proof Naming

The current naming scheme provides good room for growth:

- `ExecutionStep` can be extended with new variants
- `BundleProgram` could evolve to include metadata, debugging info, etc.
- The Compiler/VM metaphor scales well with additional optimization passes

## Conclusion

The implementation successfully achieves the architectural goals of separating compilation intelligence from mechanical execution. The naming conventions are generally strong and intuitive, with only the `BundlePlan` creating confusion.

The main improvements needed are:

1. Formalizing the implicit contract between components
2. Fixing the performance issue with `live_items`
3. Clarifying or removing the unused `BundlePlan` code

With these changes, the architecture will be robust, maintainable, and ready for future enhancements.

## References

- Original design: `docs/design/execution-step-architecture.md`
- Implementation files:
  - `crates/cribo/src/bundle_plan/compiler.rs`
  - `crates/cribo/src/bundle_vm.rs`
  - `crates/cribo/src/bundle_plan/builder.rs`
  - `crates/cribo/src/tree_shaking.rs`
